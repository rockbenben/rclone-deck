use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::config::jobs::{Defaults, Job, JobsFile};
use crate::config::{AppConfig, ConfigError};
use crate::error::AppError;
// 配对那三条判据（键、槽位、单侧键）的定义在 `pair`：同一处同时被 `config`（对齐那一档能不能建）
// 与 `runner::drafts`（这张卡上还缺哪几格）读着。视图层与它们共用一份，界面上那张卡才与锁同坐标系。
use crate::pair::{pair_key_of, pair_slot_of};
use crate::note::ShortNote;
use crate::rcd::{RcdSpec, RcdSupervisor};
use crate::runner::Runner;

/// 出厂配置：**别人装上时拿到的第一份**。2026-10-09 起它只留一条示范任务。
///
/// 改之前的这一份是**作者本机那套**（`r2:img` / `qiniu:7zj5r8` 那些桶、
/// `D:\vault\...` 那些目录、`rclone_bin` 是他那台机的绝对路径）。别人装上之后：
/// `rclone_bin` 指向一个不存在的文件 → 后台起不来 → 顶栏写「还没连上后台」；
/// 而屏上摆着 10 条任务，其中 4 条写着会删东西，桶名是他从没配过的。
/// 那是**指向别人家**的配置，不是出厂该有的样子。
///
/// 现在这一份的三条取舍：
///  * **`rclone_bin` 留一个绝对路径的占位**，不自动探测 PATH ——
///    `config::AppConfig::from_defaults` 特意不赌「PATH 上按顺序第一个」（那句注释在
///    `config/mod.rs`），而**占位比空串好**：空串在解析侧直接被拒，占位能解析、
///    能进设置面那一格，由人填。后台起不来时顶栏那句已经把去处说了。
///  * **`log_dir` 落在 `%LOCALAPPDATA%\rclone-deck\logs`** —— 与 `AppConfig` 算 pid
///    路径时用的那个根是同一处，新用户不必改就能建出来（D 盘不是人人都有）。
///  * **一条 `copy` 示范、零条会删的**。删除那一档（`sync`）不随出厂发 ——
///    它必须由人自己配出来、配对了、看清了才按。会删的东西不该由别人替他放进列表。
///    需要多对位置 / 多条对账 / 会删那几档的证人改读 `tests/fixtures/full-jobs.toml`。
pub const DEFAULT_JOBS_TOML: &str = r#"# rclone-deck 任务定义。纯文本，可手工编辑，可进 git 备份。
# 这里不放任何密钥；remote 的凭据留在 rclone 自己的配置里。
#
# 第一次用先做两件事：
#   1) 把下面 rclone_bin 改成**你这台机器上** rclone.exe 的完整路径
#      （在终端里敲 where rclone，第一条就是）；
#   2) 把下面那条示范任务的 src / dst 换成你自己的两个位置
#      （云端那一侧的 `r2:img` 要先在 rclone config 里配好这个 remote）。
# 改完**重开面板**才生效。

[defaults]
rclone_bin = "C:\\rclone\\rclone.exe"
flags = ["--fast-list", "--transfers", "32", "--checkers", "64"]
exclude = ["Thumbs.db", "*.DS_Store"]
# 日志放哪。**留空 = %LOCALAPPDATA%\rclone-deck\logs**（跟着当前用户走，谁装都成立）。
# 这里没法写 %LOCALAPPDATA% —— jobs.toml 是静态文本，没有变量可展开，
# 所以出厂那份不写死任何人的路径（改之前写的是作者那台的 D:\Sync\...，没有 D 盘的人连启动都过不去）。
log_dir = ""

# 一条示范：只把缺的补上去，不删任何东西。
# 想要「让两边一样」（会删掉多余的），在界面上配出来 —— 那一档不会随出厂发。
[[job]]
id = "demo"
name = "示范：只补，不删"
kind = "copy"
src = "D:\\我的图片"
dst = "r2:img"
"#;

pub fn write_default_jobs_toml(path: &Path) -> Result<(), ConfigError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, DEFAULT_JOBS_TOML)?;
    Ok(())
}

/// 首次运行时把出厂配置落到 %APPDATA%/rclone-deck/jobs.toml；已存在就不动。
pub fn ensure_jobs_toml() -> Result<PathBuf, ConfigError> {
    let path = AppConfig::jobs_toml_path()?;
    if !path.exists() {
        write_default_jobs_toml(&path)?;
    }
    Ok(path)
}

/// 从**同一份** `Defaults` 同时导出 rcd 的启动参数（`--exclude` / `--fast-list`）与 runner 的
/// 排除清单。这两侧是同一份清单的两个匹配器：rcd 用启动参数版，`checks::expected_keys`
/// 用 runner 里的 `self.defaults.exclude` 版。拆成这个纯函数不是为了好看——它让"同源"成为
/// `bootstrap` 唯一的组装点，也就成了可被无 rcd 的测试证人的一件事（见 `tests/app_state.rs`
/// 的 `the_rcd_and_the_runner_get_their_exclude_from_one_and_the_same_defaults`）。
/// 若有人把它拆回"两处各自 `load_jobs()`"，同一份 jobs.toml 就能喂出两套 exclude，漂移的下游
/// 是一次删除。
pub fn shared_defaults(app: &AppConfig, defaults: Defaults, jobs: Vec<Job>)
    -> (RcdSpec, Defaults, Vec<Job>) {
    let spec = RcdSpec::from_config(app, &defaults);
    (spec, defaults, jobs)
}

/// 加载配置 → 拉起 rcd → 组装 runner。失败时把原因带回去给界面显示，
/// **不静默换端口、不静默降级**（spec §6 第一行）。这一条今天要说准：
/// 「有人在听这个号」那一档确实不换 —— 按 `reclaim` 自证、并说出占用者是谁；
/// 而「绑不上又没人听」（动态排除区）那一档由 `deck_port()` 现挑一个号，那不算静默：
/// 顶栏「详情」与实际端口同源（判据见 `config::AppConfig::pick_port`）。
pub async fn bootstrap() -> Result<Arc<Runner>, AppError> {
    ensure_jobs_toml()?;
    let file: JobsFile = AppConfig::load_jobs()?;
    let app = AppConfig::from_defaults(&file.defaults)?;
    // 2026-10-09：出厂配置瘦身成一条示范之后，`rclone_bin` 留的是一个**占位路径**
    // （`DEFAULT_JOBS_TOML` 顶上那段）。别人装上第一次启动，后台必然起不来 ——
    // 而"起不来"这句话本身不告诉他**该去哪改**，那等于把新用户卡在第一屏。
    //
    // 为什么在这里查、而不是让 spawn 去失败：`AppConfig::from_defaults` 只验**绝对路径**
    // （那是它该管的，见 `config/mod.rs` 里面那句"本机 PATH 上有两个 rclone，靠 PATH 顺序选
    // 是赌博"），**不验文件在不在**。于是占位路径能过解析、走到 spawn 才炸，那时交上来的
    // 是 OS 的话术（"系统找不到指定的文件"），读的人知道坏了，不知道要改哪一格。
    //
    // 这一句因此**先说坏在哪、再说去哪改**，且**不静默替代**：面板不自动探测 PATH、不猜
    // 换成别的 rclone（那正是 `config/mod.rs` 里拒绝"按顺序选"的原因），只是把去处说出来。
    if !app.rclone_bin.is_file() {
        return Err(AppError::Startup(format!(
            "那个位置上没有 rclone 可执行文件：{}。去「设置」里「rclone 程序在哪」那一格，\
             填你这台机器上的完整路径（终端里敲 where rclone，第一条就是），填完重开面板。",
            app.rclone_bin.display()
        )));
    }
    // 只解析这一次（`AppConfig::load_jobs`），`file.defaults` 一个值同时喂给 rcd 与 runner：
    // `shared_defaults` 从这份 `Defaults` 导出 spec，并把**同一个** `Defaults` 交回给 Runner。
    let (spec, defaults, jobs) = shared_defaults(&app, file.defaults, file.jobs);
    let handle = Arc::new(RcdSupervisor::start(spec).await?);
    Ok(Arc::new(Runner::new(handle, app, defaults, jobs)))
}

pub struct AppInner {
    pub runner: Option<Arc<Runner>>,
    pub startup_error: Option<String>,
}

pub struct AppState(pub Mutex<AppInner>);

impl AppState {
    pub fn new(runner: Option<Arc<Runner>>, startup_error: Option<String>) -> Self {
        Self(Mutex::new(AppInner { runner, startup_error }))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AppInner> {
        self.0.lock().expect("AppState 的锁中毒了")
    }

    pub fn runner(&self) -> Result<Arc<Runner>, AppError> {
        let inner = self.lock();
        match &inner.runner {
            Some(r) => Ok(Arc::clone(r)),
            None => Err(AppError::Startup(
                inner.startup_error.clone().unwrap_or_else(|| "未知原因".into()))),
        }
    }

    pub fn take_runner(&self) -> Option<Arc<Runner>> {
        self.lock().runner.take()
    }

    pub fn startup_error(&self) -> Option<String> {
        self.lock().startup_error.clone()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RcdStatus {
    pub ok: bool,
    pub port: u16,
    pub pid: u32,
    pub rclone_version: String,
    pub execute_id: String,
    /// 起不来那一句，**已经分成两段**（`note_for_text`）：顶栏是常驻面，
    /// 把 `无法启动 rclone rcd：…` 这种原文直接摆上去，等于界面上常驻一个内部名词。
    /// `AppInner` 里存的仍是原文（那一份是记录），转换只在这个边界上做。
    pub startup_error: Option<ShortNote>,
    pub locks: std::collections::BTreeMap<String, String>,
    /// 日志目录。**空串表示"还不知道"**（rcd 没起来时配置可能压根没读到），
    /// 界面在没有时不显示这一项——显示一个空路径比不显示更像骗人。
    pub log_dir: String,
    /// 配置文件在哪。**空串 = 还不知道**（与 `log_dir` 同一档口径）。
    /// 那句"改配置要重开面板"以前把路径写死在界面里，而面板可以跑在别的家上
    /// （夹具规程 R68 就是为此存在的）—— 路径该由后台交出来，界面只渲染。
    pub config_path: String,
    /// 现在有几件事正在做（去重后的持有者数）。
    ///
    /// 为什么由后台数：界面数 `locks` 的键就是自己下了一份判定（`I5`），而键是按 `Scope` 分的 ——
    /// 一次动作可以申领多个键（`acquire_scopes`），数键会把一件事报成两件。
    /// 顶栏那一格读的就是这个数，`0` 时整段不渲染。
    pub active_count: usize,
    /// 这一台面板每一趟**按什么算"变了"**：`[defaults] flags` 里点了 `--size-only` 就是只比大小。
    ///
    /// 又是一枚"判定作为数据走出去"（与 `active_count`、`config_path` 同一族）：那串 flags 从来没
    /// 上过屏，界面自己从里面猜就是第二份口径。它读的是**启动时那一份** `Defaults`（与 rcd 的 argv
    /// 同源，见 `app_state.rs` 里"exclude 必须同一份"那条证人），所以改完配置没重开面板时，
    /// 这里说的是**正在跑的那一种**，不是文件里写着的那一种。
    /// 后台没起来那一档填 `false`：那一刻没有任何一趟在按什么比法跑，说一句"只比大小"就是骗。
    pub size_only: bool,
}

/// 一条任务交给界面那一份。**只交条目**：界面上那张"配对卡"是 `src/pairs.ts::groupJobs` 按
/// `pairKey`/`pairSlot`/`kind` 划分出来的，后台不再另交一份算好的四格表（`PairView`，已删）——
/// 同一批任务被两处各算一遍，迟早会漂，而漂开的那一次是"四格里有一格按下去动的是另一条任务"。
/// 划分要用的那把判据（哪两条算同一对、这一条站 a 还是 b、这一对能不能当对齐）住在 `crate::pair`，
/// 被 `config`（对齐那一档能不能建）与 `runner::drafts`（这张卡还缺哪几格）一起读着。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub src: String,
    pub dst: String,
    pub open_dir: Option<String>,
    pub locked_by: Option<String>,
    /// src 这一侧是不是本地目录（`endpoint_of` 算出来的）。前端要知道「哪一侧是全集」，
    /// 但不该自己再拿正则判一遍端点：那是同一件事的第二份口径，而它已经在
    /// `endpoint.rs` 与 `types.ts` 里漂开过一次（终审 Important 2）。
    pub src_is_local: bool,
    /// dst 那一侧同样由 `endpoint_of` 判定，和 `src_is_local` 一起把"哪侧是本地/哪侧是 remote"
    /// 变成数据，前端的两处判断（盘点显隐、对齐确认前提）都只读这两个字段。
    pub dst_is_local: bool,
    /// 上一次盘点的那两个数 + 它是几点算的（spec §3，裁定 R71 的"列举不自动跑"那一半）。
    /// `None` = 这台面板上这条任务还没盘过 —— 界面要说"还没算过"，不许显示 0/0，
    /// 也不许因为缺了就自己发起一次列举（那正是每次开窗几十次计费 LIST 的来路）。
    pub last_summary: Option<crate::runner::SummaryView>,
    /// 这一条属于哪**对**位置：两侧端点各走一遍 `Scope`（同一棵树、大小写不敏感、两种斜杠都归一），
    /// 取键、排序、拼接。**方向与 kind 都不进键** —— 方向是这一对上的一个选择，不是身份
    /// （「以本地为主、以云端为主应该是选择框，而不是 N 条不同的条目」）。
    ///
    /// 为什么在后台算而不在前端分组：这把键问的是"是不是同一对目录"，而**锁问的是同一件事**
    /// （`Scope::conflicts`）。两份口径迟早会漂，漂开的那一次就是"两张卡按下去动同一棵树"。
    /// 键相等在界面上就等于"它们是同一张卡"，所以它必须与锁同源。
    pub pair_key: String,
    /// 这一条在这对里站 a 还是 b（`pair_slot_of`，与键同一条规则）。界面上的两个选择框
    /// 就按 (kind, 槽位) 摆四格，缺的那格不出现。
    pub pair_slot: &'static str,
    /// a 那一侧是不是本地目录。它说的是**这一对的 a 面**，不是这一条任务的 src：一条把本地写在
    /// dst 的对齐任务站 b，这一枚仍然为真。卡上「以本地为主 / 以云端为主」那两句挑哪一枚
    /// **读这个标志**，不拿路径正则再认一遍（裁定 R69 同一条）。
    /// b 那一面没有自己的字段，但也不是"a 取反"推出来的：界面按这一条的槽位去 `src_is_local`/
    /// `dst_is_local` 里拿另一头那一枚，两面各自一个标志（只带一枚去取反，两侧都本地的那一对
    /// 会被说成"一本地一云端"，而那一对根本没有云端可说）。两面相等就是两侧同型，
    /// 那一档界面不许说"云端"，得退回用两条路径本身说话。
    pub a_is_local: bool,
    /// 这一条是不是**这一趟临时立起来**的那一张卡上的一格（裁定 R96）。
    /// 界面上三处读它：那句"这张卡是这一趟临时立的，重开面板就没了"、「存成常用」、「删掉这张临时卡」。
    /// 判定在 `Runner::is_draft`（它只认那张内存表），这里只把它作为数据带出去 ——
    /// 界面若按编号前缀猜（`adhoc-`），就有两份口径，而漂开的那一次收掉的是一条真配置项。
    /// `adhoc-` 那一段前缀的用处是**日志文件名**（出事时认得出是哪张卡哪一格，R99），不是判据。
    pub is_draft: bool,
}

impl JobView {
    pub fn new(job: &Job, locked_by: Option<String>,
        last_summary: Option<crate::runner::SummaryView>, is_draft: bool) -> Self {
        let (src_ep, dst_ep) = (
            crate::endpoint::endpoint_of(&job.src),
            crate::endpoint::endpoint_of(&job.dst),
        );
        let (src_is_local, dst_is_local) = (src_ep.as_local().is_some(), dst_ep.as_local().is_some());
        let slot = pair_slot_of(&job.src, &job.dst);
        Self {
            id: job.id.clone(),
            name: job.name.clone(),
            kind: job.kind.toml_word().to_string(),
            src: job.src.clone(),
            dst: job.dst.clone(),
            open_dir: job.open_dir.clone(),
            locked_by,
            src_is_local,
            dst_is_local,
            last_summary,
            pair_key: pair_key_of(&job.src, &job.dst),
            pair_slot: slot,
            a_is_local: if slot == "a" { src_is_local } else { dst_is_local },
            is_draft,
        }
    }
}

