pub mod browser;
pub mod checks;
pub mod drafts;
pub mod lists;
pub mod tasklog;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
// `abandoned` 用的是 std 的锁（不是下面 tokio 那个同名 `Mutex`）：R48 之后往里头存 guard 的
// 是 `Inflight::Drop`，而 `Drop` 里不能 await。别名只给这一个字段用，其余表锁照旧。
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::config::jobs::{Defaults, Job, JobKind};
use crate::config::{AppConfig, ConfigError};
use crate::endpoint::Endpoint;
use crate::gate::{self, GateReport};
use crate::mutex::{Busy, JobMutex, MultiGuard};
use crate::note::{note_for_text, ShortNote};
use crate::preview::{PreviewIntent, PreviewRegistry, StoredPreview};
use crate::prune::{self, MirrorPlan, PruneReport};
use crate::rc::types::{JobStatus, StatsView, SubmitAck};
use crate::rc::{ListError, RcError};
use crate::rcd::{RcdError, RcdHandle};
use crate::scope::Scope;
use tasklog::{Level, TaskLog};

pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// 只对**读**生效。提交不重试：超时的那次可能已经落地，重试会起两个 job 打同一对目录。
pub const RC_RETRIES: u32 = 3;

/// 一次「在跑」里面的两段。**列清单**那一段两侧各列举一次、一个东西都还没改；
/// **传输**那一段 rclone 的 job 已经在动那两棵树。
///
/// 为什么要分开说（操作员那句「不该让用户等结果」）：对齐按下确认之后先要列一趟两侧
/// （真库上几十秒），那一段屏上以前**什么都不说**；而「传输进行中，别关窗」那一句在那个时候
/// 是个善意的谎 —— 那时关窗什么都不会打断。分开之后每一段说各自那一句。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskPhase {
    Listing,
    Transferring,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TaskState {
    Idle,
    Queued,
    /// `jobid` 是 `Option`：列清单那一段还没有 rc 的 job 可指，填一个 0 之类的假值
    /// 就是让下一个人拿它去 `job/status` 轮询一个不存在的 job。
    Running { phase: TaskPhase, jobid: Option<i64>, stats: Option<StatsView> },
    Done { jobid: i64, stats: StatsView, dry_run: bool, previewed: Option<usize> },
    /// 终态失败。**那一句是两段的**（`ShortNote`）：这一臂有两个生产者，而它们的性质不同——
    /// 一个是 rclone 的原话（`status.error`，带 `operation error S3` / `StatusCode` / JSON 尾巴），
    /// 另一个是我们自己的不变量句（带 `deletes=` 这类读数）。
    /// 命令失败那一条路早就走 `note_for_text` 分了两段，终态是**第三条路**：它是数据字段不是错误返回，
    /// 所以以前直接把原串交给界面，操作员在「任务失败」下面读到的就是 rclone 的话。
    Failed { jobid: i64, error: ShortNote },
    Unknown { reason: String },
}

/// 行上那一句差异摘要（spec §3）。**只是上一次的结果**：谁去算、几点算都由调用侧决定，
/// 这条读数不参与任何判定 —— 它的存在就是为了界面上那两个数带着一句"这是几点算的"，
/// 而不是被当成现状（清单本身不在这里，那是点开之后才要看的东西）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryView {
    pub pending_upload: usize,
    pub orphans: usize,
    /// 这一次那两个位置里 **dst 那一侧**是不是本地目录（由 `PruneReport` 带过来，这里不重判）。
    /// 那两个数按位置命名，界面上「会删 N 个」要说的是本档会删的那一半 —— 判据只能在这里给。
    pub dst_is_local: bool,
    /// 空基准那次盘点的诊断随集合一起带出去（R29）：为真时那两个数不可信，
    /// 而 `run_inventory` 本来就会在这种情形硬拒绝，所以这里通常是 false。
    pub empty_basis: bool,
    pub measured_at: String,
}

#[derive(Debug, Error)]
pub enum RunError {
    // 话术按**目录树**说，不按"一对目录"说（裁定 R92）：锁的单位是树，父子也算同一棵
    // （判据见 `mutex::conflicts_with`）。"同一对目录上已有任务在跑"在父子那一格是**错的**——
    // 占着的是一对包着它的目录，而这句话是给操作员看的拒绝理由，说错方向比不说更坏。
    // 界面那一行的「正在跑：谁」不引用这句（它读的是 `JobView.lockedBy`，同一个查询的读数）。
    #[error("另一条任务正在写同一棵目录树：{owner}")]
    Busy { owner: String },
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Rc(#[from] RcError),
    #[error(transparent)]
    List(#[from] ListError),
    #[error("{0}")]
    Invariant(String),
    #[error("读写日志失败：{0}")]
    Io(String),
}

impl Serialize for RunError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// rcd 的身份读数，供命令层拼 `RcdStatus`。只交出**读得到、且不含凭据**的四个字段：
/// 端口、PID、`core/version` 回读到的版本、execute_id。凭据（rc 用户/口令）从来不在这里，
/// 也不在 `RcdHandle` 之外任何地方——`RcdHandle` 自己就故意不 derive `Debug`。
pub struct RcdInfo {
    pub port: u16,
    pub pid: u32,
    pub rclone_version: String,
    pub execute_id: String,
}

/// 传输跑完之后唯一该说的话。两条判据，方向不同：
/// **copy 不许有删除**（老口径一字不动），并且 **对齐真删的量必须等于人确认过的那份清单**。
/// 第二条不是洁癖：`sync` 的删除范围由 rclone 自己算，面板能担保的只有"两次列表一致"，
/// 而这一句把那个担保变成有人听的结果而不是注释。`previewed == None` 意味着"这次不是走对齐进来的"
/// （老状态、或 copy 任务），**不许**当成 0 —— 那会把两者都误判成违约。
/// 违约那一句交出去也是**两段**：短的那句说"哪里不对"（人一眼读得完），
/// 长的那份留着 `deletes=` 这类读数——排障要它，界面上的明面不要。
pub fn transfer_invariant_violation(
    kind: &JobKind,
    stats: &StatsView,
    previewed: Option<usize>,
) -> Option<ShortNote> {
    if matches!(kind, JobKind::Copy) {
        let violated = stats.deletes > 0 || stats.deleted_dirs > 0;
        if !violated {
            return None;
        }
        return Some(ShortNote::with_detail(
            "复制不该删东西，却报了删除。",
            format!("copy 任务报告了删除：deletes={} deleted_dirs={}。\
                     出现删除说明底层实际跑的是 sync，已按失败处理。",
                    stats.deletes, stats.deleted_dirs),
        ));
    }
    previewed.filter(|p| (*p as u64) != stats.deletes).map(|p| ShortNote::with_detail(
        "对齐时删掉的和给你看的不一样。",
        format!("预览 {p} 个，实际 deletes={} deleted_dirs={}。\
                 要么对端在确认之后被动过，要么两次列表不是同一批。\
                 日志里有那次预览的逐条清单，请拿它核对。",
               stats.deletes, stats.deleted_dirs)))
}

/// 「开始」这个入口对哪一类任务说得通。
///
/// 复制可以直接跑。对齐的实删**不在这里**：它走自己那颗「让两边对齐」，那里有确认与动手前的
/// 重列，所以这个入口只受理复制。但**只读那一趟放行**（`dry_run`）：它一个东西都不改。
/// 放行它不再是"给操作员一条交叉核对的路"——那条入口 2026-10-05 已从对齐那一格撤了
/// （见 `DESIGN.md` 的 Components 那一条），对数仍然跑，只是跑在 `tests/mirror_runner.rs` 里。
///
/// 门装在 runner 上而不是只装在 Tauri 命令壳里：壳带着 `tauri::State`，测试里构造不出来，
/// 而壳外面还有调用方。命令层因此只转发，不再自己写一遍判断（两处各写一遍，就必有一处
/// 与代码矛盾）。
///
/// 话术只说**今天**成立的事（R56 第 2 条：那是给操作员看的句子，不是给开发者看的）：
/// 指得出对齐真正的那个入口（卡上的「让两边对齐」）。
/// 那句尾的"还可以用「先试跑」"删了：对齐那一格只剩一颗按钮，`JobListView.tsx` 头上那段
/// 文档写的是同一件事（"「会删除」那一档没有这一句：那一格只剩一颗"）—— 指着一个界面上
/// 没有的入口，等于交出一份没人接得住的拒绝。
/// 不许出现 `{kind:?}` 的 Debug 写法（`Sync` 是枚举名）或内部命令名。
pub fn transport_kind_gate(kind: &JobKind, name: &str, dry_run: bool) -> Result<(), String> {
    match kind {
        JobKind::Copy => Ok(()),
        JobKind::Sync if dry_run => Ok(()),
        JobKind::Sync => Err(format!(
            "「{name}」是对齐任务：真删之前得先点「让两边对齐」拿到那份预览，把会被删掉的\
             每一行看清、确认之后才会动手。这个入口只受理复制，对齐的实删不在这里跑。")),
        // 非传输类沿用命令层原来那句：人话词来自 `JobKind::word()`。
        JobKind::Gate => Err(format!(
            "「{name}」是{}任务，这个入口只跑复制（对齐那一档要走它自己那颗「让两边对齐」）",
            kind.word())),
    }
}

/// 确认执行这一路上四句拒绝话术的**唯一**出处（`run_mirror` 的控制流中间）。
///
/// 抽成函数不是为了复用，是为了让 `tests/commands_surface.rs` 那份"操作员看到的句子里不许有
/// 内部名词"的扫描够得着：这四句都长在流程中间，测试构造不到那个位置，扫不到就等于这几句
/// 从没被这条规矩管过（那道闸另外两句本来就是纯函数，天然扫得到）。
pub fn mirror_wrong_kind_refusal(name: &str, word: &str) -> String {
    format!("「{name}」是{word}任务，不能按对齐跑")
}

pub fn mirror_confirmation_voided_refusal(name: &str) -> String {
    format!(
        "「{name}」这份确认已经不作数了（用过一次，或者这两个位置上跑过别的任务）。\
         请重新确认一次；要对着逐条名单再看一遍，点「重新预览」。")
}

pub fn mirror_list_changed_refusal(name: &str, confirmed: usize, now: usize) -> String {
    format!(
        "「{name}」这两个位置上的清单变了（确认 {confirmed} 个，现在 {now} 个）——对端在这期间被动过。\
         放弃这次实删，请重新预览再确认一次。")
}

pub fn mirror_side_refused_refusal(name: &str, empty_src: bool, too_much: bool) -> String {
    // 阈值这一句是这一路的**唯一**可读证据：「不预览直接批」
    // （预览那句它从没见过），而它偏偏就是"为什么没删"的那一条。
    // 与预览那一路同一口径：说出线在哪、并标明那是假设值。
    format!(
        "「{name}」复核时这一侧已经不对了（这一侧空={empty_src}，要删的占比超限={too_much}）。\
         动手前列一次这两个位置就看得见这种形状，这次不该跑。\
         占比那条线是一半（阈值 1/{den}，这是假设值、没量过）；先确认位置与排除规则，\
         要对着逐条名单看就点「重新预览」。",
        den = prune::RATIO_DEN)
}
/// 裁定 R48 item 1：**提交成功（或提交结果未知）之后、`job/status` 确证 `finished == true`
/// 之前**，guard 由这个持有者代管，它的 `Drop` 默认把 guard 存进 `abandoned`（那组键继续被占住）。
///
/// 为什么要有这个类型，而不是把 `abandoned.push` 写回 Unknown 那条臂：今天坏掉的就是**顺序**——
/// 两条日志写（`log.kv` / `log.line`）排在 `push` 之前，各自带一个 `?`，于是"日志目录写不下去"
/// 成了"放开这把锁"的充分条件，而 rclone 那边的 job 可能还在删。逐个 `?` 数过去没意义：
/// 下一个 `?` 是 Task 10 加的，注释拦不住它。有了持有者，**任何**提前结束的形状（`?`、panic、
/// 早退 `return`）都落在同一语义上：**没确证跑完就不放开键**。
///
/// 唯一的解除点是 [`Inflight::disarm`]，只许在 `job/status` 答了 `finished == true` 那一刻调
/// （无论成功还是失败，"说完"就是"这一个 job 不会再动这一对目录"）。
///
/// `Drop` 里**没有** `unwrap`/`expect`/打印：本机 console 开着 QuickEdit，一次被挡住的 stderr 写
/// 能把调用方挂住，而展开过程中 panic 是**进程中止**。存 guard 走已有的 `abandoned` 通道，
/// 锁中毒只 `into_inner`（临界区是 `Vec::push`，中毒了也只剩"少一次登记"这种更坏的选项）。
struct Inflight<'a> {
    /// `Some` = 还没有任何东西确证这个 job 结束；`disarm` 把它取走。
    /// 类型是 `MultiGuard`（一批键，Task 7 起两侧各一把），**R33 那套生命周期一个字都没改**：
    /// 没确证跑完就把整批键存进 `abandoned`，绝不就地 drop，也绝不 `mem::forget`。
    guard: Option<MultiGuard>,
    abandoned: &'a StdMutex<Vec<MultiGuard>>,
}

impl<'a> Inflight<'a> {
    fn new(guard: MultiGuard, abandoned: &'a StdMutex<Vec<MultiGuard>>) -> Self {
        Self { guard: Some(guard), abandoned }
    }

    /// 解除默认动作，把 guard 交回调用方。**调用方必须把它绑成有名字的局部量**
    /// （`let _ = inflight.disarm();` 会当场落地，`must_use` 对 `let _` 不管用，
    /// 见 `transfer` 头上那段实测），让它活到函数结束。
    fn disarm(&mut self) -> Option<MultiGuard> {
        self.guard.take()
    }
}

impl Drop for Inflight<'_> {
    fn drop(&mut self) {
        if let Some(guard) = self.guard.take() {
            let mut abandoned = self.abandoned.lock().unwrap_or_else(|e| e.into_inner());
            abandoned.push(guard);
        }
    }
}

/// 一次对齐预览的对外形状（裁定 R60）。`token` 是"人已经看过这份清单"的唯一凭证，
/// **不透明且单次消费**：拿 job id 或时间戳当授权就是 R34 那个 ABA——陈年那份会替现在这份作数。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewView {
    pub token: String,
    /// 这份清单有几个。与下面那一份名字列表一起交出来，是为了让界面上那句
    /// 「删这 N 个」读的是一个数，而不是数组长度（`I5` 同一族：数行数归后台）。
    pub doomed_count: usize,
    /// 逐条：这次对齐会删掉的那些，**相对目标端自己的根**（spec §4.1）。
    pub doomed: Vec<String>,
    /// 同一趟会**补上**的那些有几个（`MirrorPlan::pending`）。层里那句「补上那 M 个」
    /// 读的就是它：那个数不许由界面拿两个计数去减（DESIGN.md 那条"不在 TS 里比较两个数"）。
    pub pending: usize,
    pub src_count: usize,
    pub dst_count: usize,
    /// 这次预览实际发出去多少个 `operations/list`。观测值与比例一起进日志：
    /// `RATIO_DEN` 这个假设值靠这种数字校准，不靠论证。
    pub list_requests: usize,
    /// `"3/4"`。故意不做成除法：让前端算比例就是让它重新解释这个判据（R29 同一条口径）。
    pub doomed_ratio: String,
}

pub struct Runner {
    rcd: Arc<RcdHandle>,
    app: AppConfig,
    defaults: Defaults,
    jobs: Vec<Job>,
    /// **临时配对卡**（裁定 R93）：操作员在跑的那一面选出来的两处位置，只在内存里活到关面板。
    /// 为什么另开一张表而不是往 `jobs` 里塞：`jobs` 是"那一份文件的内存像"，写它就得回答
    /// "要不要写回文件"（那是设置面的通路，一条写路径不该有两个入口）；而 `lookup` 一旦只认它，
    /// 临时任务就只能是"没写进文件的配置"——下一次读文件就散架。
    /// 这里存的是**同一形状的 `Job`**，所以锁（按 `Scope`）、日志（按 id）、状态、预览登记
    /// 全套机器一行不改就能跑临时格 —— 第二条平行实现就是第二份安全机器，那才是真危险。
    drafts: StdMutex<Vec<Job>>,
    /// 下一张临时卡的编号。**只增不减**：收起一张卡会让 `drafts` 里那几条走掉，但这个数不回退。
    /// 理由不是排版整洁——预览凭证的归属是**按 id 认**的（`run_mirror` 里"既认 token 也认归属"那一句），
    /// id 一旦重新发出去，陈年那份凭证就落在新卡的同名格上"作数"，那是 R34 那一族 ABA。
    /// 裁定 R99 的"与配置里的编号比一次，撞了就进位"用的也是这一个游标。
    draft_seq: StdMutex<usize>,
    mutex: JobMutex,
    /// R33：状态未知时存下来的 guard，**故意不释放**。
    /// 键会一直锁到面板重启为止——这不是缺陷：那两棵树上的 rclone job 可能还在跑、还在删，
    /// 而放开锁就等于让另一个写同一棵树的任务（反方向那条，或浏览器的一次 purge）开跑（起点事故）。
    /// 子项目 1 不给这个状态放按钮，
    /// 所以下面的 `force_release` 只作为 runner 内部与测试的口子；界面侧的恢复路径留给子项目 2。
    /// R48 item 1 起，写它的是 `Inflight::Drop`（提交之后任何没确证跑完的出口都走那里），
    /// 所以它是 std 的同步锁而不是 tokio 的——`Drop` 里不能 await，临界区又只有 `Vec::push`。
    /// Task 7 起存的是一批键（`MultiGuard`，两侧各一把），存法与释放语义照旧。
    abandoned: StdMutex<Vec<MultiGuard>>,
    /// 待确认的预览登记（`crate::preview`）：token → 那份清单关于哪个动作、内容本身、以及它
    /// 关于哪些位置。对齐与浏览器共用同一个登记处，所以"看过清单"这套机制只有一份实现。
    /// 为什么要存内容而不是存条数、`scopes` 为什么放两侧，见 `StoredPreview` 的字段文档；
    /// 作废与顶掉的两条判据见 `PreviewRegistry::{invalidate, supersede}` 与本文件的
    /// `invalidate_writes`。
    previews: PreviewRegistry,
    states: Mutex<HashMap<String, TaskState>>,
    logs: Mutex<HashMap<String, PathBuf>>,
    /// 每条任务**上一次**盘点的结果（spec §3：行上显示的是"上次的两个数 + 它是几点算的"）。
    /// 与 `logs` 同一类：用 `std::sync::Mutex`，临界区只有 map 读写、不跑回调也不写日志——
    /// 那条纪律在 `mutex.rs` 的 `drop(map)` 那一行上面（本机 console 的 QuickEdit 会把一次被挡住的写变成整张表的冻结）。
    last_summaries: StdMutex<HashMap<String, SummaryView>>,
}

impl Runner {
    pub fn new(rcd: Arc<RcdHandle>, app: AppConfig, defaults: Defaults, jobs: Vec<Job>) -> Self {
        Self {
            rcd, app, defaults, jobs,
            drafts: StdMutex::new(Vec::new()),
            draft_seq: StdMutex::new(1),
            mutex: JobMutex::new(),
            abandoned: StdMutex::new(Vec::new()),
            previews: PreviewRegistry::default(),
            states: Mutex::new(HashMap::new()),
            logs: Mutex::new(HashMap::new()),
            last_summaries: StdMutex::new(HashMap::new()),
        }
    }

    /// 界面上看得见的**所有**任务：配置文件里那些，加上这一趟临时立起来的那些（R93）。
    /// 交出具体的 `Job` 而不是切片：这两批来自两个地方，合成一份才有"同一把尺子量两边"这件事。
    pub fn jobs(&self) -> Vec<Job> {
        let mut out = self.jobs.clone();
        out.extend(self.drafts.lock().unwrap().iter().cloned());
        out
    }

    /// 按编号找任务：先查配置，再查临时表。两边的形状一模一样，所以调用方不需要知道它是谁。
    pub fn job(&self, id: &str) -> Option<Job> {
        self.jobs.iter().chain(self.drafts.lock().unwrap().iter())
            .find(|j| j.id == id).cloned()
    }

    /// 日志落在哪个目录。顶栏要显示它：操作员出事后第一个问题是"我去哪儿看日志"，
    /// 而**在顶栏接上这一格之前**，这个答案只在配置文件里，屏上没有任何一处说得出路径。
    /// 它同时是一条操作规程的前提——用夹具家跑面板时，必须能一眼看出面板真的走在夹具里，
    /// 而不是落进 `%LOCALAPPDATA%\rclone-deck\logs`（`%APPDATA%` 那边住的是 `jobs.toml`；
    /// 那起事故记在 R58 与 R68）。
    pub fn log_dir(&self) -> String {
        self.defaults.log_dir.clone()
    }

    /// rcd 的身份读数（端口 / PID / 版本 / execute_id），供命令层拼 `RcdStatus`。
    /// 版本是启动时经 `core/version` 回读、存在 `RcdHandle` 里的那一份，不是配置里假设的。
    pub fn rcd_info(&self) -> RcdInfo {
        RcdInfo {
            port: self.rcd.port,
            pid: self.rcd.pid,
            rclone_version: self.rcd.rclone_version.clone(),
            execute_id: self.rcd.execute_id.clone(),
        }
    }

    pub fn mutex(&self) -> &JobMutex {
        &self.mutex
    }

    /// 这一台面板每一趟**按什么算"变了"**：`[defaults] flags` 里有没有那枚 `--size-only`。
    /// 读的是启动时那一份 `Defaults`（`RcdSpec::from_config` 用的就是它，所以与 rcd 的 argv 同源），
    /// 不是磁盘上可能已经被"改设置"写过的那一份 —— 面板不热重载，屏上和日志上说的都得是**在跑的**那一种。
    /// 判定本身住在 `config::jobs::compares_by_size`，这里只是把它接到那两个消费点（顶栏、任务日志）。
    pub fn size_only(&self) -> bool {
        crate::config::jobs::compares_by_size(&self.defaults.flags)
    }

    pub async fn state(&self, id: &str) -> TaskState {
        self.states.lock().await.get(id).cloned().unwrap_or(TaskState::Idle)
    }

    pub async fn locks(&self) -> BTreeMap<String, String> {
        self.mutex.snapshot().await
    }

    /// 这条任务要写的那两棵树正被谁挡着（裁定 R92）——**界面那一行的读数与拒绝动作用的是同一条判据**。
    ///
    /// 交出去的是 `job.write_scopes()`，与 [`Runner::acquire`] 递进锁层的同一个来源，命令层不再自己
    /// 拼一遍"这条任务关于哪些位置"（两处各拼一遍，迟早一处按等号、一处按包含）。
    /// 原来 `get_jobs` 写的是 `j.scope_keys().iter().find_map(|key| locks.get(key))`：登记表的键
    /// 比的是**等号**，而锁早就改成比**包含关系**（R90），于是"占着父目录的那次写"对应的子层任务
    /// 显示成没锁——按钮亮着，点下去拿到 `Busy`。判据本体在 `mutex::conflicts_with`，
    /// 读侧那条查询是 `JobMutex::blocking_owner`。
    /// 它不取代 `locks()`：那条交的是**整张表**的快照（`RcdStatus.locks`，命令层与 `runner_local` 那族
    /// "键还占着没有"的证人是读者），这里问的是**这一条任务**被谁挡着。顶栏并不渲染那张快照。
    pub fn blocking_owner(&self, job: &Job) -> Option<String> {
        self.mutex.blocking_owner(&job.write_scopes())
    }

    /// R33：状态未知之后**唯一**的解锁出路（子项目 1 不接界面按钮，走 runner 内部与测试）。
    /// 清的是登记表，不是那个存起来的 guard——guard 还在 `abandoned` 里，等面板重启才落；
    /// 那时它的 `Drop` 因为**按 token 认所有权**（R33 的门控 + R34 的 token）不会动后来的持有者。
    /// 这两条是配套的：没有按 token 的所有权，这个口子会把下一个合法任务偷偷解锁。
    pub async fn force_release(&self, key: &str, expected_owner: &str) -> Option<String> {
        self.mutex.force_release(key, expected_owner).await
    }

    pub async fn last_log(&self, id: &str) -> Option<PathBuf> {
        self.logs.lock().await.get(id).cloned()
    }

    pub async fn shutdown_rcd(self) -> Result<(), RcdError> {
        match Arc::try_unwrap(self.rcd) {
            Ok(handle) => handle.shutdown().await,
            Err(_) => Ok(()),
        }
    }

    // ---------------------------------------------------------- copy / sync

    pub async fn run_copy(&self, id: &str, dry_run: bool) -> Result<TaskState, RunError> {
        // 交出具体的 `Job`（R93：查表要能同时看配置与临时表），这里借给它 ——
        // 下面几十处 `&Job` 参数与 `job.xxx` 读法一行不改。
        let job = &self.lookup(id)?;
        // 门排在**第一件事**：一次被拒的动作不许留下痕迹。`acquire` 之后的早退会把这把键**放开**
        //（这段还在 `transfer` 之前，`Inflight` 只在 `transfer` 里提交之后才创建，所以走不到
        // `abandoned` 那条登记——但一次被拒的动作连这把键都不该短暂碰过）；`open_log` 之后的早退
        // 会在日志目录里落一个这个任务从没跑过的文件。两者都是"被拒绝的动作产生了副作用"。
        transport_kind_gate(&job.kind, &job.name, dry_run).map_err(RunError::Invariant)?;
        // 排在门**之后**：被拒的那一次仍然一个副作用都不该有（见上面那段）。作废只认**真实写入**——
        // spec §4.1 的作废条件是"那一棵树被真写过"，而试跑（dry run）一个东西都不改，它对端上
        // 承诺的删除范围没有任何影响。更关键的是设计上的交叉核对**要的正是让人在预览之后再跑一次
        // 试读**去比对 rclone 自己算的那份删除计数（见 `transport_kind_gate` 放行 dry run 的理由）：
        // 让一次只读的试跑作废确认，等于把这条被鼓励的核对变成每次都要重新预览的负担。
        if !dry_run {
            // 传的是**被写的那一侧**（`dst`）：这一跑真正落盘的东西全进那一侧，
            // 而任何预览只要有关于这个位置的清单就该作废（判据见 `invalidate_writes`）。
            self.invalidate_writes(&[Scope::of_endpoint(&job.endpoint_dst())]).await;
        }
        // `&job.kind`：`JobKind` 不是 `Copy`（Task 2 的派生只有 Debug/Clone/PartialEq/Eq），
        // 按值 match 会想把 `job` 里的这个字段搬走——`job` 是 `&Job`，E0507。
        let method = match &job.kind {
            JobKind::Copy => "sync/copy",
            JobKind::Sync => "sync/sync",
            // 编译器不知道上面那道门已经把这一档拒干净了（它只看函数签名，不读返回值），
            // 所以这里必须留一条把 match 补穷尽。话术继续走 `JobKind::word()`：
            // 不许退回 `{other:?}` 那种 Debug 写法——那是把枚举名印给操作员。
            other => return Err(RunError::Invariant(format!(
                "「{}」是{}任务，这个入口跑不了", job.name, other.word()))),
        };
        // 锁**交给 transfer 管**：这条任务的真生命周期是 rclone 的 job，不是这个函数体。
        // `previewed` 传 `None`：copy 这条路（含对齐的试跑）都不是走确认进来的，没有"人确认过的那份清单"
        // 可比，`None` 意味着"这不是对齐实删"，绝不能当 0（那会把只增的 copy 也误判成违约）。
        let guard = self.acquire(job).await?;
        let log = self.open_log(job).await?;
        self.transfer(job, method, dry_run, &log, guard, None).await
    }

    pub async fn run_gate(&self, id: &str) -> Result<GateReport, RunError> {
        // 交出具体的 `Job`（R93：查表要能同时看配置与临时表），这里借给它 ——
        // 下面几十处 `&Job` 参数与 `job.xxx` 读法一行不改。
        let job = &self.lookup(id)?;
        self.require_kind(job, JobKind::Gate)?;
        checks::require_local_dir_manifest(job)?;
        let _guard = self.acquire(job).await?;
        let log = self.open_log(job).await?;
        let io = |e: std::io::Error| RunError::Io(e.to_string());

        // 日志里也要走会报错的那条推导：`remote_part()` 对本地 dst 返回 ""，
        // 于是日志把"这个任务根本没有远端"印成一个空字符串，看起来像正常对账。
        let remote = job.remote_for_list().map_err(RunError::Config)?;
        log.kv(Level::Info, "闸门开始对账", &[
            ("job", job.id.clone()), ("src", job.src.clone()),
            ("fs", job.fs_for_list()), ("remote", remote),
        ]).map_err(io)?;

        let expected = checks::expected_keys(job, &self.defaults.exclude)?;
        let before = self.rcd.client.list_request_count();
        // 耗时与请求数一起记：这一前缀列一次要多久，是**下一次撞墙之前的预警位**。
        // 界面上那两个计数只有"列完之后"才存在，而闸是等时间的——先看见趋势的必须是日志。
        let listed_at = std::time::Instant::now();
        let actual = checks::actual_keys(&self.rcd.client, job).await?;
        let list_requests = self.rcd.client.list_request_count() - before;
        let list_secs = listed_at.elapsed().as_secs();
        let report = checks::gate_report(&expected, &actual);

        // 三态而不是 if/else（裁定 R29）：空基准既不是"通过"也不是"缺失 N 个"，
        // 它是"没看住就下结论"。gate 是只读的，所以这里不报错，但必须用 Error 级别说话，
        // 并且 `passed()` 在这一档是 false —— 前端只许读序列化出去的 `verdict`。
        match report.verdict {
            gate::GateVerdict::Pass => {
                log.kv(Level::Info, "闸门通过", &[
                    ("expected", report.expected_count.to_string()),
                    ("actual", report.actual_count.to_string()),
                    ("list_requests", list_requests.to_string()),
                    ("list_secs", list_secs.to_string()),
                ]).map_err(io)?;
            }
            gate::GateVerdict::Missing => {
                log.kv(Level::Error, "闸门未通过", &[
                    ("expected", report.expected_count.to_string()),
                    ("actual", report.actual_count.to_string()),
                    ("missing", report.missing.len().to_string()),
                    ("list_requests", list_requests.to_string()),
                    ("list_secs", list_secs.to_string()),
                ]).map_err(io)?;
                for k in &report.missing {
                    log.line(Level::Error, &format!("缺失 {k}")).map_err(io)?;
                }
            }
            gate::GateVerdict::EmptyBasis => {
                log.kv(Level::Error, "闸门无从对账：本地清单是空的", &[
                    ("expected", report.expected_count.to_string()),
                    ("actual", report.actual_count.to_string()),
                    ("list_requests", list_requests.to_string()),
                    ("list_secs", list_secs.to_string()),
                ]).map_err(io)?;
                log.line(Level::Error,
                    "先检查 src 指向与 exclude 是否放宽过头，再看发布。").map_err(io)?;
            }
        }
        for group in &report.case_conflicts {
            log.line(Level::Warn, &format!("只差大小写：{}", group.join(" , "))).map_err(io)?;
        }
        Ok(report)
    }

    // ---------------------------------------------------------- 盘点（R58：只读，不删）

    /// 「本地没有的云端对象」盘点。裁定 R58 之后这是这条路的**唯一**入口：执行侧
    /// （勾选、三道确认闸、逐 key `operations/deletefile`）已随 prune 功能整体删除——
    /// 用户存在直接上传到云端的对象，这个集合分不出"改名垃圾"与"云-only 正式资产"，
    /// 给它配删除按钮就是把误删做成一键。**任务这一侧**会删东西的只有 `sync`（对齐）；
    /// 浏览器那一面的 `deletefile` / `purge` / `rmdir` 是另一套机器，不走这里，也各有各的闸。
    ///
    /// 形状由两端自己保证：**两侧得有一侧是本地目录、另一侧是 remote**（同型就没什么可"盘点"的），
    /// 但**哪一侧是本地不再由 `src` 决定**（2026-10-09）。原先这里写死"src 是本地、dst 是远端"，
    /// 于是「云端覆盖本地」那个方向**根本没有这一面**——而它恰恰是删本机硬盘的那一档，
    /// 是全app最不该没有盘点的一格。
    ///
    /// 两侧一律走 `checks::side_set`（它按端点类型自己分派：本地走盘、远端一次 LIST），
    /// **不再用** `expected_keys` / `actual_keys`。那一对服务的是 gate 与旧形状，
    /// 键归一到**桶全 key**；`side_set` 两侧都退到各自的根。`checks.rs` 里那一句
    /// 「别把其中一个拿去喂另一个」正是这件事 —— 而 2026-10-09 把盘点那份集合
    /// 当成对齐动手前逐条比对的基准之后，两个形状必须**真的同形**，
    /// 否则那份基准一次都过不了（`run_mirror` 会每次都报"清单变了"）。
    ///
    /// 只读，但仍然拿互斥键：列举期间反向传输会改动两侧集合，交出去的清单会当场失真。
    pub async fn run_inventory(&self, id: &str) -> Result<PruneReport, RunError> {
        // 交出具体的 `Job`（R93：查表要能同时看配置与临时表），这里借给它 ——
        // 下面几十处 `&Job` 参数与 `job.xxx` 读法一行不改。
        let job = &self.lookup(id)?;
        let _guard = self.acquire(job).await?;
        let log = self.open_log(job).await?;
        log.line(Level::Info, "盘点开始（只读，不删任何东西）")
            .map_err(|e| RunError::Io(e.to_string()))?;

        // 计数器差值 = 这一次真发了多少个 LIST，写进日志。远端侧现在恒为 1（整棵树一次调用），
        // 它留着不是为了校准什么，是"这句话是真的"：哪天列举又分叉、或者 rclone 的行为变了，这个数先动。
        // 旁边那个 `list_secs` 是同一族读数：一次调用不再等于一次往返之后（对象存储上递归列举
        // 的真实成本在请求数里），先看见"这一前缀变慢了"的位置就在这一行。
        let before = self.rcd.client.list_request_count();
        let listed_at = std::time::Instant::now();
        let src = checks::side_set(&self.rcd.client, &self.defaults.exclude, &job.endpoint_src()).await?;
        let dst = checks::side_set(&self.rcd.client, &self.defaults.exclude, &job.endpoint_dst()).await?;
        let list_requests = self.rcd.client.list_request_count() - before;
        let list_secs = listed_at.elapsed().as_secs();
        // 报告仍然是「本地 vs 远端」这一帧（字段名与界面一个字不动），只是"哪一端是本地"
        // 由端点类型判，而不是由"src 必须是本地"这个前提逼出来。
        let (local, remote) = match (job.endpoint_src(), job.endpoint_dst()) {
            (Endpoint::Local(_), Endpoint::Remote { .. }) => (&src, &dst),
            (Endpoint::Remote { .. }, Endpoint::Local(_)) => (&dst, &src),
            (Endpoint::Local(l), Endpoint::Local(r)) => {
                return Err(RunError::Invariant(format!(
                    "job {} 的两侧都是本地目录（{} 与 {}）：这一对位置之间没有云端，盘点没有对象。\
                     面板能盘的是「本地 ↔ 远端」这两种形状。",
                    job.id, l.display(), r.display())))
            }
            (Endpoint::Remote { fs: a, remote: ra }, Endpoint::Remote { fs: b, remote: rb }) => {
                return Err(RunError::Invariant(format!(
                    "job {} 的两侧都是远端（{a}:{ra} 与 {b}:{rb}）：盘点那一侧要能在本机读到，\
                     两侧都是云端时这一格给不出可读的那一列。",
                    job.id)))
            }
        };
        let mut report = crate::prune::orphans(local, remote);
        // 方向由这里填（内核只看得到两堆名字）：dst 是本地目录 ⇒ 这一档是「云端覆盖本地」，
        // 它要删的是本机多出来的那一半。判据与界面上那两枚「以本地为主 / 以云端为主」同一个来源
        // （`endpoint_dst` 的类型），只在这里判一次，往下当数据带出去。
        report.dst_is_local = matches!(job.endpoint_dst(), Endpoint::Local(_));

        // 裁定 R29 的空基准拒绝在盘点侧**保留**（R58）：本地清单为空而桶里非空时，
        // 集合差交出的是**整个桶**。清单虽然只读，空基准下"本地没有"这个结论本身照样
        // 不可信（src 指错、exclude 放宽过头、盘没挂上都是这个形状），交出去等于骗人。
        // 拒绝仍然排在逐条打印之前（证人也跟着改名保留：
        // `inventory_refuses_an_empty_local_basis_instead_of_offering_the_whole_remote`）。
        if report.empty_basis {
            log.kv(Level::Error, "盘点被拒绝：本地清单是空的", &[
                ("local", report.local_count.to_string()),
                ("remote", report.remote_count.to_string()),
                ("list_secs", list_secs.to_string()),
            ]).map_err(|e| RunError::Io(e.to_string()))?;
            return Err(RunError::Invariant(format!(
                "job {} 的本地清单一个对象都没有，而桶里有 {} 个。\
                 这不是「整桶都该删」，这几乎一定是 src 指错了或 exclude 放宽过头了。\
                 先确认 src 与 exclude，再来盘点。", job.id, report.remote_count)));
        }

        for k in &report.orphans {
            log.line(Level::Warn, &format!("孤儿 {k}")).map_err(|e| RunError::Io(e.to_string()))?;
        }
        log.kv(Level::Info, "盘点结束", &[
            ("orphans", report.orphans.len().to_string()),
            // 另一半（还没上传的）只记数不记名：一次"云端只装了几个"的盘点能交出六千行，
            // 而这一侧从来不是删除理由——上面那些孤儿逐条落盘是因为它们曾经会是删除候选。
            ("pending_upload", report.pending_upload.len().to_string()),
            ("local", report.local_count.to_string()),
            ("remote", report.remote_count.to_string()),
            ("list_requests", list_requests.to_string()),
            ("list_secs", list_secs.to_string()),
        ]).map_err(|e| RunError::Io(e.to_string()))?;
        // 顺手落一份**可 diff 的名单**（操作员要的"拉一次存成文件，之后自己比对"）。
        // 写不成**不**把这次盘点一起报成失败：它是只读面，交出来的两个数是刚列出来的真东西，
        // 为一个副产品让整次列白做等于让附属品比证据重要。但那一趟必须在日志里说实话，
        // 并且 `list_file` 留空 —— 界面那一行"名单存成了"因此不露，不会指到一个不存在的地方。
        // 上面那三处 `return Err`（两处两侧同型、一处空基准）都到不了这里：被拒绝的那一次一个名单都不许多写。
        // 记下来当对齐动手前的比对基准的，必须是**本条任务那一档要删的那一份**：
        // `report.orphans` 与 `report.pending_upload` 是同一个 `plan` 内核在两个方向上的
        // `doomed`（互为补集），上传方向要删的是云端多出来的那些（`orphans`），下载方向要删的是
        // 本机多出来的那些（`pending_upload`）—— 拿错那一份，动手前那次重列每次都比对不上，
        // 这一档就变成"看过差异反而永远跑不动"。判据读 `endpoint_dst` 是不是本地，与
        // `listed_doomed` 同一套（证人：`a_download_direction_confirmation_reuses_the_seen_inventory_list`）。
        // 人此刻在「差异」那一面已经逐条看过它们了，所以「看差异 → 让两边对齐 → 直接确认」
        // 不必再点一次「重新预览」重新列一遍 —— 省掉的那一次列举正是这个出口存在的全部理由。
        // 登记的**键**也分方向（`endpoint_src` / `endpoint_dst`）：反方向那一条对齐取不到这一份。
        self.previews
            .remember_seen(
                &Scope::of_endpoint(&job.endpoint_src()),
                &Scope::of_endpoint(&job.endpoint_dst()),
                // 本条任务那一档要删的那一份：上传方向删的是云端多出来的（`orphans`），
                // 下载方向删的是本机多出来的（`pending_upload`）。这两半是同一个 `plan` 内核在
                // 两个方向上的 `doomed`（互为补集），拿错那一份就是每次都拒「清单变了」。
                (if report.dst_is_local { &report.pending_upload } else { &report.orphans })
                    .iter().cloned().collect(),
            )
            .await;

        let secrets = self.rcd.secrets();
        let refs: Vec<&str> = secrets.iter().map(|s| s.as_str()).collect();
        match lists::write(log.path(), &report, &refs) {
            Ok(list) => {
                log.line(Level::Info, &format!("名单存成了 {}", list.display()))
                    .map_err(|e| RunError::Io(e.to_string()))?;
                report.list_file = list.display().to_string();
            }
            Err(e) => {
                log.kv(Level::Warn, "名单没存成", &[("原因", e.to_string())])
                    .map_err(|e| RunError::Io(e.to_string()))?;
            }
        }
        let summary = SummaryView {
            pending_upload: report.pending_upload.len(),
            orphans: report.orphans.len(),
            dst_is_local: report.dst_is_local,
            empty_basis: report.empty_basis,
            measured_at: report.measured_at.clone(),
        };
        // 记的是**这一次**盘点的结果，排在成功返回之前：界面拿它当"上次看到的东西"，
        // 而"没跑过"与"跑过但被拒绝"必须是两种读数 —— 上面那三处 `return Err` 因此都不会留下摘要。
        // 这一句不许失败也不许跳过：失败的是面板自己的内存表，不该让一次已经算完的盘点白做。
        self.last_summaries.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.to_string(), summary);
        Ok(report)
    }

    /// 这条任务上一次的盘点摘要；**没跑过就是 `None`**（spec §3：不自动刷，也不许凭空造一个 0/0 当现状）。
    pub fn last_summary(&self, id: &str) -> Option<SummaryView> {
        self.last_summaries.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id).cloned()
    }

    // ---------------------------------------------------------- 对齐：预览（spec §4.1 第 1 步）
    //
    // 对齐这几节自己不出现在 `Endpoint` 上：端点只是交给 `checks::side_set` 的参数，
    // 这里拿的是 `Scope::of_endpoint(...)`。（文件级那条 `use crate::endpoint::Endpoint` 是
    // 盘点那节分形状用的，不归这几节。）

    /// 列两侧、算差、带回这次的 LIST 观测值。**预览与执行前重列都走这里**——
    /// 两处必须是同一段代码，否则"两次一致"只是两次各自正确的巧合。
    async fn listed_doomed(&self, job: &Job) -> Result<(MirrorPlan, usize, u64), RunError> {
        let before = self.rcd.client.list_request_count();
        let listed_at = std::time::Instant::now();
        let src = checks::side_set(&self.rcd.client, &self.defaults.exclude, &job.endpoint_src()).await?;
        let dst = checks::side_set(&self.rcd.client, &self.defaults.exclude, &job.endpoint_dst()).await?;
        Ok((prune::plan(&src, &dst),
            self.rcd.client.list_request_count() - before,
            listed_at.elapsed().as_secs()))
    }

    /// 写过的东西让相关清单作废（R71 / I3 第 3 条）。
    /// 判据从"同一对目录的正反向键"换成 Scope 交集：浏览器没有 job，方向串根本串不到它。
    /// 传进来的是**被写的那一侧**（`touched`）——预览的 scopes 里只要有任何一个与它冲突，
    /// 那份清单就不再作数。
    ///
    /// 试跑（dry run）不作废：它一个东西都不改，一份看过的清单在它跑完之后照样成立。
    /// （以前这里还写着"交叉核对要的就是它" —— 那条入口 2026-10-05 已从对齐那一格撤了，
    /// 对数现在跑在 `tests/mirror_runner.rs` 里，见 `run_copy` 那个 `if !dry_run` 上面的注释。）
    /// 旧实现里"任务已从配置里消失就顺手清掉那份授权"那一支没有搬过来：那不再需要——
    /// 拿着那样的凭证来确认，第一步 `lookup` 就报"没有叫 X 的任务"，走不到授权那一步。
    pub async fn invalidate_writes(&self, touched: &[Scope]) {
        self.previews.invalidate(touched).await;
    }

    /// 只列、不删。返回"会被删掉"的每一行，并登记一个一次性 token。
    pub async fn preview_mirror(&self, id: &str) -> Result<PreviewView, RunError> {
        // 交出具体的 `Job`（R93：查表要能同时看配置与临时表），这里借给它 ——
        // 下面几十处 `&Job` 参数与 `job.xxx` 读法一行不改。
        let job = &self.lookup(id)?;
        if job.kind != JobKind::Sync {
            return Err(RunError::Invariant(format!(
                "「{}」是{}任务，只有对齐任务需要看这份清单", job.name, job.kind.word())));
        }
        // 预览也占键：两次列举之间不许有人插进来动这一对目录。
        let _guard = self.acquire(job).await?;
        let log = self.open_log(job).await?;
        let (plan, list_requests, list_secs) = self.listed_doomed(job).await?;

        log.kv(Level::Info, "对齐预览", &[
            ("src", job.src.clone()), ("dst", job.dst.clone()),
            ("src_count", plan.src_count.to_string()), ("dst_count", plan.dst_count.to_string()),
            ("doomed", plan.doomed.len().to_string()),
            // 两半都记：这一趟删多少与补多少。日志里只有一半，回头读这份日志的人
            // 就会以为对齐只删东西（操作员就是这么停住的）。
            ("pending", plan.pending.to_string()),
            ("ratio", format!("{}/{}", plan.doomed.len(), plan.dst_count)),
            ("list_requests", list_requests.to_string()),
            ("list_secs", list_secs.to_string()),
        ]).map_err(|e| RunError::Io(e.to_string()))?;
        // 逐条清单排在两道拒绝**之前**：被拒绝的那一次也要说清它当时看见的是什么。
        for k in &plan.doomed {
            log.line(Level::Info, &format!("会删掉：{k}")).map_err(|e| RunError::Io(e.to_string()))?;
        }

        if plan.empty_src {
            log.kv(Level::Error, "预览被拒绝：这一侧一个对象都没有", &[
                ("src", job.src.clone()), ("dst_count", plan.dst_count.to_string()),
            ]).map_err(|e| RunError::Io(e.to_string()))?;
            return Err(RunError::Invariant(format!(
                "「{}」这一侧（{}）一个对象都没有，而对端有 {} 个。这不是「整个对端都该删」，\
                 这几乎一定是 src 指错了、exclude 放宽过头了，或者那个盘没挂上。\
                 先确认这两处，再来对齐。", job.name, job.src, plan.dst_count)));
        }
        if plan.too_much {
            log.kv(Level::Error, "预览被拒绝：要删的量像事故不像清理", &[
                ("doomed", plan.doomed.len().to_string()),
                ("dst_count", plan.dst_count.to_string()),
            ]).map_err(|e| RunError::Io(e.to_string()))?;
            return Err(RunError::Invariant(format!(
                "「{}」这次要删 {} 个，而对端总共才 {} 个——达到或超过一半（阈值 1/{}，\
                 这是个假设值、没量过）。正常清理不会一次砍掉一半，这个形状更像挂错了目录\
                 或者这一侧被清空过一半。请先确认两侧路径；真要继续，请用 rclone 手工做——\
                 面板故意不给一个「我知道，就是要删」的绕行按钮。",
                job.name, plan.doomed.len(), plan.dst_count, prune::RATIO_DEN)));
        }

        // 对齐这份清单**关于两侧**：源变了删除候选也会变（I3 第 1 条"对子的两侧都要进 Scope"）。
        let preview_scopes = vec![
            Scope::of_endpoint(&job.endpoint_src()),
            Scope::of_endpoint(&job.endpoint_dst()),
        ];
        // 同配对只留最新那一份：发新预览**不是写**，所以它只顶掉"scope 集合完全相同"的旧清单
        // （判据见 `PreviewRegistry::supersede`，证人 `supersede_only_replaces_the_same_scope_set`）。
        self.previews.supersede(&preview_scopes).await;
        // 这份 doomed 是执行前逐条比对要用的那一份：先攒成集合再存，
        // 下面那个结构体字面量把 `plan.doomed` 移进 `PreviewView`，之后再取就是用后借用。
        let stored_doomed: BTreeSet<String> = plan.doomed.iter().cloned().collect();
        let token = self.previews.issue(StoredPreview {
            intent: PreviewIntent::Mirror { job_id: id.to_string() },
            // 预览过的凭证：`Some` 里就是"人看过的那一份"，动手前逐条比对拿它当参照物。
            doomed: Some(stored_doomed),
            scopes: preview_scopes,
        }).await;
        // 比例先算：下面那个结构体字面量把 `plan.doomed` 移进 `PreviewView`，
        // 之后再 `plan.doomed.len()` 就是用后借用（E0382）。
        let doomed_ratio = format!("{}/{}", plan.doomed.len(), plan.dst_count);
        let doomed_count = plan.doomed.len();
        Ok(PreviewView {
            token,
            doomed_count,
            doomed: plan.doomed,
            pending: plan.pending,
            src_count: plan.src_count,
            dst_count: plan.dst_count,
            list_requests,
            doomed_ratio,
        })
    }

    // ---------------------------------------------------------- 对齐：不预览直接批

    /// 「批准这一趟，没看名单」：登记一份**不带清单**的确认，交回一次性凭证。
    ///
    /// 为什么加这一条（操作员：「对齐操作为什么要列清单，用户点击后等结果就好」）：原来按下
    /// 「让两边对齐」要先付一趟两侧全量 LIST 才按得动红色那颗，而那一趟**不是**安全担保——
    /// 动手前那次重列与那两道拒绝才是。这一条把"看名单"从过路费的形状变回它本来的样子：
    /// 想看就点「重新预览」（那条路连逐条比对一起原样留着），不想看就直接批。
    ///
    /// 这里**一次列举都不发**：不列东西就判不了比例，所以那两道拒绝一道都不许挪到这里，
    /// 它们照旧排在 `run_mirror` 动手之前。证人：
    /// `a_confirmation_without_a_preview_still_refuses_an_emptied_source_side`。
    pub async fn confirm_mirror(&self, id: &str) -> Result<String, RunError> {
        let job = &self.lookup(id)?;
        if job.kind != JobKind::Sync {
            return Err(RunError::Invariant(mirror_wrong_kind_refusal(&job.name, job.kind.word())));
        }
        // 日志排在顶掉旧凭证之前：开日志失败的那一次不该顺手把人手里那份清单作废掉。
        let log = self.open_log(job).await?;
        // 与预览同一组 scopes：反方向那一跑动的是同两棵树，写过之后这份确认同样不作数。
        let scopes = vec![
            Scope::of_endpoint(&job.endpoint_src()),
            Scope::of_endpoint(&job.endpoint_dst()),
        ];
        self.previews.supersede(&scopes).await;
        // 「看差异」那一面**已经把删除候选逐条摆给人看过了**（`run_inventory` 把同一份集合
        // 记进了 `seen`）。有它就直接当基准：动手前那次重列会比对它，而不是"没有名单可比"。
        // 少了这一句，看过差异的那一趟还得再点一次「重新预览」把同一批名字再列一遍 ——
        // 而人已经看过了，那一次列举纯属白花（这正是问出来的那件事）。
        // 方向不匹配（反方向那一条对齐）、从没点过看差异、或超过 `SEEN_CAP` —— 三种情况下
        // 它是 `None`，走"批准这一趟、没看名单"那一档，两道拒绝照旧一道不少。
        let seen = self
            .previews
            .peek_seen(&Scope::of_endpoint(&job.endpoint_src()), &Scope::of_endpoint(&job.endpoint_dst()))
            .await;
        let has_seen = seen.is_some();
        let via = if has_seen { "（基准取自「看差异」那一份）" } else { "" };
        let token = self.previews.issue(StoredPreview {
            intent: PreviewIntent::Mirror { job_id: id.to_string() },
            doomed: seen,     // `None` = 没人看过名单，动手那一刻列出来的就是准（见 `StoredPreview`）
            scopes,
        }).await;
        log.line(Level::Info, &format!(
            "已确认这一趟对齐{}{}", via,
            if has_seen { "；动手前照旧重列一遍并逐条比对" } else { "（没看逐条名单；动手前照旧重列一遍并过两道拒绝）" }
        ))
            .map_err(|e| RunError::Io(e.to_string()))?;
        Ok(token)
    }

    // ---------------------------------------------------------- 对齐：确认执行（spec §4.1 第 2、3 步）

    /// §4.1 的后两步：重列 → 复核两道拒绝 → 逐条一致 → 才真跑。
    ///
    /// 顺序是刻意的：
    /// 1. **先吃 token**（在任何早退之前，包括拿不到锁那条路）——一次被拒/失败/成功的动作都不能
    ///    把授权留在桌上，否则等锁的人可以拿旧清单插队复用；
    /// 2. 再拿这一对目录的锁，走**同一个** `listed_doomed` 重列（预览与重列必须同一段代码，
    ///    否则"两次一致"只是两次各自正确的巧合）；
    /// 3. **两道复核排在逐条比对之前**（spec §4.1 那句"两道拒绝排在比对之前"）。这一档要抓的
    ///    事故形状是"源端在确认那几十秒里没了"（盘掉挂载 / 目录被清空）：那时清单**也**变了，
    ///    先报"清单变了"就会把操作员支去检查对端，而该看的是这一侧。比对守的是剩下的那一档
    ///    ——两侧计数还说得通、但删除候选换了人；
    /// 4. 全过才 `transfer(..., previewed = Some(那份清单的条数))`，把预览数作为数据带出去。
    pub async fn run_mirror(&self, id: &str, token: &str) -> Result<TaskState, RunError> {
        // 交出具体的 `Job`（R93：查表要能同时看配置与临时表），这里借给它 ——
        // 下面几十处 `&Job` 参数与 `job.xxx` 读法一行不改。
        let job = &self.lookup(id)?;
        if job.kind != JobKind::Sync {
            return Err(RunError::Invariant(mirror_wrong_kind_refusal(&job.name, job.kind.word())));
        }
        // 授权**先吃掉**：无论下面走哪条路（拒绝、失败、成功），同一个 token 都不该再用第二次。
        // 放在 acquire 之前是有意的——拿不到锁也不能把授权留着，否则等锁的人可以拿旧清单插队。
        // `consume` 取走即消费（它就是登记表上的 `remove`，见 `PreviewRegistry::consume`）。
        let stored = self.previews.consume(token).await;
        // 既认 token 也认归属：A 的确认不能授权 B（同一对目录同时可能有两条任务）。
        // 归属现在读的是凭证里那份 `intent`（R78：做什么由 token 说，不由参数说），判据一字未改。
        let Some(stored) = stored.filter(|s| matches!(
            &s.intent, PreviewIntent::Mirror { job_id } if job_id == id)) else {
            return Err(RunError::Invariant(mirror_confirmation_voided_refusal(&job.name)));
        };
        // 等锁那一段说「排队中」：它确实是在等，而"正在核对清单"要等真的开始列举才算。
        self.set_state(&job.id, TaskState::Queued).await;
        let guard = self.acquire(job).await?;
        let log = self.open_log(job).await?;

        // 列举那几十秒必须在屏上说话（那句「不该让用户等结果」）。
        // 收口排在 `?` 之前：**任何一条早退的路都不许把状态留在 Running** ——
        // 那是一条幽灵任务，一直挂到面板重启（`runner_local.rs` 里 `Running { stats: None }`
        // 那个形状钉的就是这一族）。
        self.set_state(&job.id, TaskState::Running {
            phase: TaskPhase::Listing, jobid: None, stats: None }).await;
        let listed = self.listed_doomed(job).await;
        self.set_state(&job.id, TaskState::Idle).await;
        let (plan, list_requests, list_secs) = listed?;
        let fresh: BTreeSet<String> = plan.doomed.iter().cloned().collect();

        // 复核两道拒绝：预览时过过一次，不代表现在还能过。把它们当"一次性装饰"就是假担保。
        //
        // 这两臂**不是两条独立判据**：`empty_src` 在数学上被 `too_much` 包住——src 空而 dst 非空
        // 时 doomed 等于整个 dst，`doomed × 2 >= dst` 恒成立。留着它是为了**诊断**：同一句拒绝要
        // 说出它是哪一种形状，而这一臂排在比对之前之后才是可达的（源端掉了就是这一档）。
        if plan.empty_src || plan.too_much {
            log.kv(Level::Error, "对齐被拒绝：复核时这一侧的结论已经不成立", &[
                ("empty_src", plan.empty_src.to_string()), ("too_much", plan.too_much.to_string()),
                ("src_count", plan.src_count.to_string()), ("dst_count", plan.dst_count.to_string()),
            ]).map_err(|e| RunError::Io(e.to_string()))?;
            return Err(RunError::Invariant(mirror_side_refused_refusal(
                &job.name, plan.empty_src, plan.too_much)));
        }
        // 逐条一致才动手（spec §6 那一行）—— 但**这条比对只挂在"有人看过那份名单"上**：
        // `None` 是「批准这一趟、没看名单」那种凭证，它没有参照物，
        // 动手这一刻列出来的就是准，守这一趟的是上面那两道拒绝（它们一道都没挪）。
        // 不一致时不猜哪一份对，把两份的差都写进日志，让事后翻日志的人看得见到底是什么动了。
        // 判据是**集合相等**而不是条数相等：一次等量的调换（看过的那个没了、同时冒出一个没看过的）
        // 数量上看不出来，删掉的却是一个谁都没确认过的东西。
        let confirmed = match &stored.doomed {
            Some(seen) => {
                if fresh != *seen {
                    log.kv(Level::Error, "对齐被拒绝：清单变了", &[
                        ("confirmed", seen.len().to_string()),
                        ("now", fresh.len().to_string()),
                        ("list_requests", list_requests.to_string()),
                        ("list_secs", list_secs.to_string()),
                    ]).map_err(|e| RunError::Io(e.to_string()))?;
                    for k in seen.difference(&fresh) {
                        log.line(Level::Info, &format!("确认时有、现在没了：{k}")).map_err(|e| RunError::Io(e.to_string()))?;
                    }
                    for k in fresh.difference(seen) {
                        log.line(Level::Info, &format!("确认时没有、现在多了：{k}")).map_err(|e| RunError::Io(e.to_string()))?;
                    }
                    return Err(RunError::Invariant(mirror_list_changed_refusal(
                        &job.name, seen.len(), fresh.len())));
                }
                seen.len()
            }
            None => fresh.len(),
        };
        log.kv(Level::Info, if stored.doomed.is_some() { "对齐清单复核一致，开始实删" }
                 else { "对齐开始实删（这份确认没带逐条名单）" }, &[
            ("previewed", confirmed.to_string()),
            // 事后读这份日志的人要能分清"人对着名单按了确认"与"人批的是这一趟"。
            ("confirmed_a_list", if stored.doomed.is_some() { "yes".to_string() } else { "no".to_string() }),
            ("list_requests", list_requests.to_string()),
            ("list_secs", list_secs.to_string()),
        ]).map_err(|e| RunError::Io(e.to_string()))?;

        // spec §4.1：任何一次真跑都作废"关于被写过的那棵树"的确认。上面那份是**吃掉**的，但吃授权
        // 与拿锁之间有个窗口——别的任务（反方向那一条也算：它那份清单的源端就是这一跑写掉的这棵树）
        // 在那一刻插进一次预览的话，它那份清单在我这次删完之后就不成立了。
        // 放在所有判据之后：被拒的那一次仍然一个副作用都不该有。
        self.invalidate_writes(&[Scope::of_endpoint(&job.endpoint_dst())]).await;

        // 删除只走 `sync/sync` 这一条正典路径（spec §4.1：绝不靠 filter/include 猜删除范围——
        // 认不出的 rc 参数会 fail-open，把不该删的放进来）。预览数随 transfer 带进终态与日志。
        self.transfer(job, "sync/sync", false, &log, guard, Some(confirmed)).await
    }

    // ---------------------------------------------------------- 共用部件

    fn lookup(&self, id: &str) -> Result<Job, RunError> {
        self.job(id).ok_or_else(|| RunError::Invariant(format!("没有叫 {id} 的任务")))
    }

    fn require_kind(&self, job: &Job, want: JobKind) -> Result<(), RunError> {
        // 这句话术是给操作员看的：用 `JobKind::word()`，不许 `{job.kind:?}`
        //（评审 Task 10 时发现的同一类问题：内部枚举名会顺着 AppError 渲染到界面上）。
        (job.kind == want).then_some(()).ok_or_else(|| RunError::Invariant(format!(
            "「{}」是{}任务，这个入口只能跑{}任务", job.name, job.kind.word(), want.word())))
    }

    async fn acquire(&self, job: &Job) -> Result<MultiGuard, RunError> {
        // 键从 `src=>dst` 方向串换成 Scope（R71 / I3）：**写哪棵树**才是互斥的单位。
        // 方向串有两个洞：它挡不住"浏览器动了同一棵树"（浏览器没有 job），
        // 又过度粘连（同一对目录的只读盘点会被它算成冲突方）。
        // 两侧都进键：对齐的源被并发改写会让"动手前重列"看到不一样的东西——那正是它要防的事。
        //
        // 交的是 `Vec<Scope>` 而不是键串（裁定 R90，判据本体见 `mutex::conflicts_with`）：
        // 键串只能比等号，于是"占着 `img` 的这一次"挡不住"要写 `img/2026` 的那一次"，
        // 而父子正是这把锁唯一真正要挡的形状——浏览器与对齐不共享任务，它们只是同一棵树的两个层级。
        // 所以现在排除的是**嵌套的两棵树**：谁占着 `decklocal:#img`，`decklocal:#img/2026` 就进不来，
        // 反过来也一样；只有长得像的邻居（`img` 与 `imgprompt`）和另一朵云上的同名树照常并行。
        //
        // 反向不再是锁的一种状态，而是**同一组两棵树**：`up` 与 `down` 交出的集合一模一样
        //（只是互换，`acquire_scopes` 先排序再折叠），所以原来那步"先拿正向、再查反向键"
        // 连同 `（反方向正在跑）` 那半句一起删了——冲突由树的集合本身给出，不由话术补。
        //
        // 两侧**互为父子**的那条任务（`kind = copy` 才写得出，例如 `r2:img` → `r2:img/backup`）由
        // `acquire_scopes` 折到祖先那一层（裁定 R91）：这里照旧交整组 `write_scopes()`，
        // 不在配置层也不在这里加一条"父子不许同批"的拒绝——那种拒绝的形状是 `Busy{owner: 自己}`，
        // 而锁说不出"是谁挡的"就等于在说谎。理由与证据都写在 `mutex::acquire_scopes` 头上。
        self.mutex.acquire_scopes(&job.write_scopes(), &job.name).await
            .map_err(|Busy { owner }| RunError::Busy { owner })
    }

    async fn open_log(&self, job: &Job) -> Result<TaskLog, RunError> {
        // 凭据的完整清单直接交给 TaskLog：擦密只有 `rcd::redact_credentials` 一份正典，
        // runner 这里**不写第二个 scrubber**（裁定 R44），递错了串就等于少擦一类秘密。
        let log = TaskLog::create(&self.app.log_dir, &job.id, self.rcd.secrets())
            .map_err(|e| RunError::Io(e.to_string()))?;
        self.logs.lock().await.insert(job.id.clone(), log.path().to_path_buf());
        Ok(log)
    }

    async fn set_state(&self, id: &str, state: TaskState) {
        self.states.lock().await.insert(id.to_string(), state);
    }

    /// 提交 → 轮询 → 落日志 → 返回终态。锁由调用方持有到本函数返回之后。
    /// `guard` **按值收进来**（R33b）。原来这里没有这个参数，是控制者补 R33 时写错的一处：
    /// Unknown 那条路写的是 `push(_guard)`，可 `_guard` 绑在**调用方**的函数体里，
    /// `transfer` 里根本看不见它——照原文抄会直接 E0425 编译不过。
    /// 按值收有双重好处：正常结束/失败时 guard 随本函数落地，锁按时释放；
    /// 而状态未知时它就在手边，能存进 `abandoned`。Task 7 起 `guard` 是一批键（`MultiGuard`），
    /// 它身上的 `#[must_use]` 与 `LockGuard` 那条同形；属性覆盖到哪一形状，说准（实测三种丢弃形状）：
    /// 裸语句 `x()?;` **会**警告（正是原来
    /// 那条会静默释放的形状）；`let _guard = …` 与 `let guard = …` 不警告（那是正确的持有形状）；
    /// **`let _ = x()?;` 仍然静默**——rustc 把 `let _ =` 当作故意丢弃，must_use 不管它。
    /// 所以这条属性只挡住"事故形状"那一种，剩下的靠 `MultiGuard` 的文档不变量，
    /// 而 `std::mem::forget` 对任何 lint 都隐形。别把它写成"保证谁都丢不掉"。
    ///
    /// **R48 item 1 改了这里的形状**：提交成功（或提交结果未知）之后 guard 立刻交给 `Inflight`，
    /// 上面那条"must_use 只挡住事故形状"的缺口由结构补上——从那一刻起到 `job/status` 确证
    /// `finished == true`（唯一一处 `disarm`）之间，任何 `?`、panic、早退都把 guard 存进
    /// `abandoned` 而不是放开键；提交**之前**的 `?` 仍然让它正常落地，因为那时没有 job 存在。
    ///
    /// 还有一条同族的顺序纪律（R48 item 2）：**每个阶段的状态写入排在它自己的日志写之前**。
    /// 日志目录写不下去（盘满、ACL、路径过长）不许决定任务的状态——那会把界面冻在
    /// `Queued`/`Running`，而操作员看到的是一条和状态无关的 `Io` 错误。
    ///
    /// 裁定 R53：R51 版这里的 `include: &[String]` 参数与它的重复键编码**已删**。
    /// 裁定 R58：prune 执行侧（逐 key `operations/deletefile`）也已整体删除，
    /// `sync/sync` 只服务 `kind = sync` 的整对齐任务 —— **在传输这一条路上**它是唯一的删除路径
    /// （浏览器那一面另有按名删除 / purge / rmdir，它们不经过 `transfer`，各自有闸与证人）。
    async fn transfer(
        &self,
        job: &Job,
        method: &str,
        dry_run: bool,
        log: &TaskLog,
        guard: MultiGuard,
        previewed: Option<usize>,
    ) -> Result<TaskState, RunError> {
        let io = |e: std::io::Error| RunError::Io(e.to_string());
        // 裁定 R49 第 1 档（知道结果的 terminal 臂：失败/成功/不变量那三行）：状态**已经**排在日志
        // 前面落成真结果，日志只是没落盘。这时写不下去**不许**把状态改回去，也不许返回一条和状态无关、
        // 让人以为"任务失败"的错误。这句话术是给调用方和 `tail_log` 的：日志断了，但界面看到的是真结果。
        let log_failed = |e: std::io::Error| RunError::Io(format!(
            "真结果已写入状态、这一行日志没落盘（{e}）：请看界面上的终态，别把这条当成任务失败"));

        log.kv(Level::Info, "任务开始", &[
            ("job", job.id.clone()),
            ("kind", format!("{:?}", job.kind).to_lowercase()),
            ("method", method.to_string()),
            ("src", job.src.clone()),
            ("dst", job.dst.clone()),
            ("dry_run", dry_run.to_string()),
            // 这一趟按什么算"变了"。顶栏那句话只在当下亮着，而"这一趟用了哪种比法"是事后
            // 回溯要用的事实：只比大小那一档会漏掉"内容改了但大小没变"，漏掉的得能被查出来。
            ("compare", if self.size_only() { "size-only".to_string() } else { "size+modtime".to_string() }),
            ("rclone", self.rcd.rclone_version.clone()),
            ("rcd_pid", self.rcd.pid.to_string()),
        ]).map_err(io)?;
        self.set_state(&job.id, TaskState::Queued).await;

        // （R53 起这里不再有 `include` 可 push，`mut` 一并去掉——留着就是死变量修饰。）
        let params: Vec<(&str, String)> = vec![
            ("srcFs", job.src.clone()),
            ("dstFs", job.dst.clone()),
        ];
        // 提交**不重试**，也不带 `_config`：`dry_run` 这个布尔由客户端自己翻成
        // `_config={"dryRun":true}`，`--exclude` / `--fast-list` 走 rcd 的启动参数（实测每调用
        // 传 fs.Config flag 会被静默忽略）。超时的那次可能已经在 rcd 里落地了，
        // 重试就会起两个 job 打同一对目录——那正是本项目要消灭的失败方式。
        let ack: SubmitAck = match self.rcd.client.submit_async(method, &params, dry_run).await {
            Ok(ack) => ack,
            // R48 item 2：`Timeout`/`Transport` 是**没有得到答复**——请求可能已经落地，
            // rcd 那边那个 job 可能已经在跑。这一档按"结果未知"办：guard 交给持有者（键不放开），
            // 状态从 `Queued` 走 `Unknown`，话术必须把后果说清楚。
            Err(e @ (RcError::Timeout { .. } | RcError::Transport(_))) => {
                // 持有者先接上手，这条臂里后面每一步（含那次日志写）都只会走"不放开"。
                // 名字带下划线但仍活到臂结束——`let _ = …` 才是当场落地。
                let _inflight = Inflight::new(guard, &self.abandoned);
                let reason = format!(
                    "提交请求没有得到答复（{e}）：这个 job 可能已经在 rcd 里起来了、可能正在动这两个位置。\
                     所以 {src}=>{dst} 这把键**不会**放开，会锁到面板重启或操作员确认后用 force_release 为止；\
                     在此期间反方向任务一律被拒。",
                    src = job.src, dst = job.dst);
                let state = TaskState::Unknown { reason: reason.clone() };
                self.set_state(&job.id, state.clone()).await;
                log.kv(Level::Error, "提交结果未知", &[
                    ("job", job.id.clone()), ("error", e.to_string()),
                ]).map_err(io)?;
                return Ok(state);
            }
            // rcd **答了**一个错（HTTP 状态码 / 载荷里的 `error`）= 根本没有 job 起来。
            // 这一档正常放开锁（guard 随本函数落地），但状态**不许**冻在 `Queued`：
            // 回到 `Idle` 说的是"这次没跑起来，可以重来"，原因由 `Err` 交给调用方、日志里也有一条。
            Err(e) => {
                self.set_state(&job.id, TaskState::Idle).await;
                log.kv(Level::Error, "提交被拒绝", &[
                    ("job", job.id.clone()), ("error", e.to_string()),
                ]).map_err(io)?;
                return Err(e.into());
            }
        };
        // 从这一行起 job 在 rcd 那边已经存在（或至少无法排除它存在）——锁从此不由日志的成败决定。
        let mut inflight = Inflight::new(guard, &self.abandoned);
        self.set_state(&job.id, TaskState::Running {
            phase: TaskPhase::Transferring, jobid: Some(ack.jobid), stats: None }).await;
        // 裁定 R49 第 2 档（选 (a)：状态先行）：这行"已提交"落在"还不知道结果"的区间里——没有
        // "真结果"可写。日志写不下去时**不许**让 `?` 直接离开 `transfer`：那会把界面冻在
        // `Running { stats: None }`，从此再没人轮询这个 job，直到面板重启。所以先落状态再返回——
        // `Unknown` 的 reason 点名"日志写不下去"，然后 `Err`。
        // 代价（选 (a) 就要付的）：`Inflight` 仍握着 guard，这个 `return` 走的是"存进 abandoned"
        // 那条路，于是这一对 (src,dst) 的键锁到面板重启或操作员 `force_release` 为止。审计断了就
        // 拒绝假装审计过，与本项目"闸门宁可拦住不可放行"的取向一致；一次磁盘满会让面板当天不好用。
        // （(b) best-effort 被排除：`TaskState` 里没有能挂"这轮日志不完整"的落点，加字段又撞本裁定的禁区。）
        if let Err(e) = log.kv(Level::Info, "已提交", &[
            ("jobid", ack.jobid.to_string()),
            ("execute_id", ack.execute_id.clone()),
        ]) {
            let reason = format!(
                "job 已提交（jobid={}）但「已提交」这一行日志写不下去（{e}）：这次传输的结果无从确证，\
                 而这一对 {src}=>{dst} 的键**不会**放开，会锁到面板重启或操作员确认后用 force_release 为止。\
                 请检查日志目录（盘满 / ACL / 路径过长）。",
                ack.jobid, src = job.src, dst = job.dst);
            self.set_state(&job.id, TaskState::Unknown { reason }).await;
            return Err(RunError::Io(format!("已提交那一行日志写不下去，状态已转 Unknown：{e}")));
        }

        let group = format!("job/{}", ack.jobid);
        let mut retry_warned = false;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;

            // 判活只认 `job/status`（R38）。`core/stats group=<不存在的一组>` 是 HTTP 200 +
            // 全零，"读到全零"与"这个 group 从来没存在过"在数据面上不可区分，
            // 所以下面那圈 stats 只用来画数字，**不**参与"还在不在跑"的判定。
            let status: JobStatus = match self.poll_status(ack.jobid).await {
                Ok(s) => s,
                Err(e) => {
                    let reason = format!("连续 {RC_RETRIES} 次读不到 job/status：{e}");
                    // R33：**这条路上不许释放这把锁。** rclone 那边的 job 可能还在跑、还在删，
                    // 而 guard 一随本函数返回就落地 = 键被放开 = 反向任务可以对着同一对目录开跑，
                    // 那正是本项目要消灭的起点事故（R31 之后互斥靠登记表，释放语义是"guard 死即放"，
                    // 所以"持锁的 task 结束"不等于"活干完了"）。存起来而不是 std::mem::forget
                    //（那会留下洗不掉的幽灵持有者）；唯一的正常出路是 `force_release`，
                    // 操作员确认 job 真的没了之后由 runner 调。
                    // R48 item 1：这里**不再**手写 `abandoned.push(guard)`——guard 在 `inflight`
                    // 手里，本臂的 `return` 和下面两次日志写带来的 `?` 都只会走"存进 abandoned"。
                    // 顺序也改了：状态先落，日志写不下去时界面看到的是 `Unknown` 而不是 `Running`。
                    let state = TaskState::Unknown { reason: reason.clone() };
                    self.set_state(&job.id, state.clone()).await;
                    log.kv(Level::Error, "任务状态未知", &[
                        ("jobid", ack.jobid.to_string()), ("reason", reason.clone()),
                    ]).map_err(io)?;
                    log.line(Level::Warn, "这个键会一直锁到面板重启或人工确认为止，防止反向任务同时动同一对目录")
                        .map_err(io)?;
                    return Ok(state);
                }
            };

            if let Ok(stats) = self.rcd.client.core_stats(&group).await {
                let view = stats.view();
                if view.retry_error && !retry_warned {
                    // 裁定 R49 第 2 档（同"已提交"那处，选 (a)：状态先行）：限流警告也落在"还不知道
                    // 结果"的循环里。这条日志写不下去时先转 `Unknown`（reason 点名日志）再返回 `Err`，
                    // 绝不让界面冻在 `Running`。代价同上：`Inflight` 握着 guard，这条 `return` 走
                    // "存进 abandoned"，键锁到重启或 force_release。
                    if let Err(e) = log.line(Level::Warn, "云端限流中，rclone 在自行重试，不是卡死") {
                        let reason = format!(
                            "job 正在跑（jobid={}）但限流警告这一行日志写不下去（{e}）：这次传输的结果无从确证，\
                             而这一对 {src}=>{dst} 的键**不会**放开，会锁到面板重启或操作员确认后用 force_release 为止。\
                             请检查日志目录（盘满 / ACL / 路径过长）。",
                            ack.jobid, src = job.src, dst = job.dst);
                        self.set_state(&job.id, TaskState::Unknown { reason }).await;
                        return Err(RunError::Io(format!("限流警告那一行日志写不下去，状态已转 Unknown：{e}")));
                    }
                    retry_warned = true;
                }
                self.set_state(&job.id, TaskState::Running {
                    phase: TaskPhase::Transferring, jobid: Some(ack.jobid), stats: Some(view),
                }).await;
            }

            if !status.finished {
                continue;
            }

            // **唯一**一处 `disarm`（R48 item 1）：`job/status` 确证这个 job 说完了。
            // "说完"就够了，成功失败都一样——它不会再动这一对目录。从这里到函数结束，
            // guard 随本函数正常落地（包括下面那次终结 `core/stats` 读失败那条路）。
            // 绑成有名字的局部量让它活到本臂结束：`let _ = inflight.disarm();` 会当场放开。
            let _confirmed_finished = inflight.disarm();

            // 结束后再读一次数字，只用于画数字和核对 copy 不变量。这一读**不走** `?`
            //（R48 item 2）：读失败时锁已经可以放开，但状态不许冻在 `Running`。
            // 这里选 `Unknown` 而不是 `Done`，理由是 `Done` 得伪造一份 stats，而全零那份
            // 会替操作员说出"确认没删东西"——数字没读到就不能说这句话，尤其是 copy。
            // 代价：一次真的成功的传输会因为这一次读失败被报成"状态未知"，需要人工核对
            // rclone 日志或补跑一次闸门；而且这一档的 `Unknown` 与提交/轮询那两条不同，
            // **不**锁键（job 已确证结束），所以话术里要把它说清楚，别让人去等 force_release。
            let stats = match self.rcd.client.core_stats(&group).await {
                Ok(s) => s,
                Err(e) => {
                    let reason = format!(
                        "job/status 已确证这个 job 结束（success={}），但结束时那次 core/stats 读不到：{e}。\
                         删除计数与 copy 不变量都没有读到，所以这一次**没有**被确认为「没删东西」，\
                         请核对 rclone 日志或补跑一次闸门。键不锁住：那个 job 已经结束了。",
                        status.success);
                    let state = TaskState::Unknown { reason: reason.clone() };
                    self.set_state(&job.id, state.clone()).await;
                    log.line(Level::Error, &reason).map_err(io)?;
                    return Ok(state);
                }
            };
            let view = stats.view();
            let duration = format!("{:.3}", status.duration);

            if !status.success {
                let state = TaskState::Failed { jobid: ack.jobid, error: note_for_text(&status.error) };
                self.set_state(&job.id, state.clone()).await;
                log.kv(Level::Error, "任务失败", &[
                    ("jobid", ack.jobid.to_string()),
                    ("duration", duration),
                    ("error", status.error.clone()),
                ]).map_err(log_failed)?;
                return Ok(state);
            }

            if let Some(violation) = transfer_invariant_violation(&job.kind, &view, previewed) {
                let state = TaskState::Failed { jobid: ack.jobid, error: violation.clone() };
                self.set_state(&job.id, state.clone()).await;
                log.kv(Level::Error, "不变量被破坏", &[
                    ("jobid", ack.jobid.to_string()),
                    ("short", violation.short.clone()),
                    ("detail", violation.detail.clone()),
                ]).map_err(log_failed)?;
                return Ok(state);
            }

            let state = TaskState::Done { jobid: ack.jobid, stats: view.clone(), dry_run, previewed };
            self.set_state(&job.id, state.clone()).await;
            log.kv(Level::Info, "任务结束", &[
                ("jobid", ack.jobid.to_string()),
                ("dry_run", dry_run.to_string()),
                ("previewed", previewed.map(|n| n.to_string()).unwrap_or_else(|| "-".into())),
                ("duration", duration),
                ("transfers", view.transfers.to_string()),
                ("total_transfers", view.total_transfers.to_string()),
                ("checks", view.checks.to_string()),
                ("bytes", view.bytes.to_string()),
                ("deletes", view.deletes.to_string()),
                ("deleted_dirs", view.deleted_dirs.to_string()),
                ("errors", view.errors.to_string()),
            ]).map_err(log_failed)?;
            return Ok(state);
        }
    }

    /// 只有**读**按 spec §6 退避重试，而且只有超时/传输故障值得再试一次：
    /// rcd 用 200 包一个 `error`（例如未知 jobid 的 "job not found"）时，
    /// `job_status` 已经把它折成 `RcError::Rc` —— 那**不是**一次"读不到"，而是一次
    /// "读到了：这个 job 不在了"，重试三次只会把这句结论再推迟近两秒（600ms + 1200ms 那两次退避）。
    /// 所以这里只等 Timeout/Transport。
    /// 耗尽之后由 `transfer` 走 `Unknown`；guard 那时不在 `transfer` 的手上而在 `Inflight` 的
    /// 手上，所以这条路的返回**无论走哪一步**都会把它存进 `abandoned`（R33 + R48 item 1）。
    async fn poll_status(&self, jobid: i64) -> Result<JobStatus, RcError> {
        let mut attempt = 0u32;
        loop {
            match self.rcd.client.job_status(jobid).await {
                Ok(s) => return Ok(s),
                Err(_e @ (RcError::Timeout { .. } | RcError::Transport(_))) if attempt + 1 < RC_RETRIES => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(300 * 2u64.pow(attempt))).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}
