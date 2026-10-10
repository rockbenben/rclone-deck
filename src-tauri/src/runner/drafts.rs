//! 临时配对卡（spec `2026-10-05-rclone-deck-adhoc-pair-card-design.md`，裁定 R93–R99）。
//!
//! 这里只做两件事：把操作员交出来的**两处位置**摊成这张卡上还缺的那几格，以及把这张卡删掉。
//! 传输、预览、实删一概不在这里 —— 那些走 `Runner` 上原来那几条。理由不是整洁：
//! 一条只给临时任务用的通路就是**第二份安全实现**，而删除的唯一路径（预览 → 一次性凭证 →
//! 动手前重列逐条比对）一旦有两份，人们只会记得修其中一份（R93）。
//!
//! 反过来说，"同一套机器"是有代价的，那些代价都长在这个模块的判据上：
//! - 临时格与配置格**抢同一把锁**（锁按 `Scope`，见 `Runner::acquire`），一次写互相作废凭证；
//! - 位置判据必须是**同一段代码**（`Job::check_positions()`），否则面板能跑一条配置侧会拒的任务；
//! - 编号**不回收**（`Runner::draft_seq`），否则旧凭证落在新卡的同名格上"作数"（R34 那一族 ABA）。
use std::collections::BTreeSet;

use crate::config::jobs::{Job, JobKind};
use crate::pair::{mirror_sides_ok, pair_key_of, pair_slot_of, same_position};

use super::{RunError, Runner, TaskState};

/// 卡上名字那一行写的东西（R96：临时的这件事必须说出来，不许只记在我们心里）。
/// 它也出现在锁的持有者名与「正在跑：谁」那一格里 —— 这是故意的：一次写在临时格上出事时，
/// 界面上说得清"是那张没存进配置的卡在写"，而不是一个看起来像配置项的名字。
pub const DRAFT_NAME: &str = "临时（没存进配置）";

/// 一张卡的四个格子，规范顺序：两格「只补，不删」在前、两格「让两边对齐（会删）」在后。
/// 每一项是 `(哪一档, 这一格的 src 站 a 还是 b)` —— 编号后缀与"这一格被占了没有"
/// 读的都是同一对值（`JobKind::toml_word()` 与 `pair_slot_of`），而界面上那四格是
/// `src/pairs.ts` 按后台带出去的 `kind`/`pairSlot` 摆出来的，用的也是这一对值。
const CELLS: [(JobKind, &'static str); 4] = [
    (JobKind::Copy, "a"), (JobKind::Copy, "b"), (JobKind::Sync, "a"), (JobKind::Sync, "b"),
];

// ---- 拒绝话术：给操作员看的句子，不是内部名词（spec §5）。
// 抽成函数与对齐那四条同一个理由：长在流程中间的句子测试构造不到那个位置，
// 扫不到就等于这几句从没被"不许漏内部名词"那条规矩管过。

pub fn draft_same_position_refusal() -> String {
    "这两处是同一个位置：自己对自己对齐没有「多余」可删，也没有要补的东西。".to_string()
}

pub fn draft_all_taken_refusal() -> String {
    "这一对位置每一格都已经有任务了：卡就在上面，换一格用就行，不用再立一张。".to_string()
}

/// 那句里的写法示例用反引号包着：标点的判据不许拿半角冒号进中文句子，而 `名称:路径` 里的
/// 冒号是**照抄 rclone 的语法**，与 `config/jobs.rs` 那句"形如 `名称:路径`"同一口径（DESIGN.md 的
/// "标点"那条把反引号里的内容列为豁免，正是为了这种字符示例）。
pub fn draft_unknown_endpoint_refusal() -> String {
    "这一侧我认不出是本地目录还是云端（云端写成 `名称:路径`）。".to_string()
}

pub fn draft_running_refusal() -> String {
    "这张卡上正有事在跑：等它跑完再收（收掉就看不见它在写哪两棵树了）。".to_string()
}

/// `Unknown` 那一档单独一句：说"正有事在跑"是撒谎——那一趟**可能**还在写，而面板确知的是
/// "没确证跑完、那两个位置还被后台占着"（R33 的语义）。指错方向的拒绝比不拒绝更坏。
pub fn draft_unconfirmed_refusal() -> String {
    "这张卡上有一趟没确证跑完（那两个位置还被后台占着）：现在收掉，就没人知道它在写哪两棵树了。".to_string()
}

/// 收一张界面上已经不存在的卡（重开过面板、或者已经被收过一次）。
/// 这一句同时是 R96 在后台侧的落点：临时卡从来不进那份文件，所以它**不可能**还在。
pub fn draft_absent_refusal() -> String {
    "这一对位置上没有可收的临时卡：临时卡只活在这一趟面板里。".to_string()
}

impl Runner {
    /// 立一张临时卡：把两处位置摊成这张卡上**缺的那几格**（R94），交回那张卡的 `pair_key`
    /// （界面拿它把右列指过去，与 `JobView.pairKey` 同一把尺子算出来的那一个）。
    ///
    /// 一格都不缺 ⇒ 拒；两处归一之后是同一棵树 ⇒ 拒；某一侧连形状都认不出 ⇒ 拒。
    /// 「让两边对齐」那一档只在恰好一侧是云端时才建（`mirror_sides_ok`），两侧都本地时这张卡
    /// 照样立起来，只是模式框里只有一格 —— 那不是拒绝，界面上看得见（spec §5 最后那一行）。
    pub fn pair_draft(&self, src: &str, dst: &str) -> Result<String, RunError> {
        // 空的那一侧连"哪一棵树"都说不出，所以排在归一比较之前：`Scope` 会把 "" 归成一个真实的键，
        // 于是"自己对自己"那一句会先响，把操作员支去检查路径写法。
        for side in [src, dst] {
            if side.trim().is_empty() {
                return Err(RunError::Invariant(draft_unknown_endpoint_refusal()));
            }
        }
        // 同一处位置按 `Scope` 的键认，不按字符串（`D:\a` 与 `d:/a/` 是同一棵树）。
        if same_position(src, dst) {
            return Err(RunError::Invariant(draft_same_position_refusal()));
        }
        let key = pair_key_of(src, dst);
        // a/b 由 `pair_slot_of` 判：恰好一侧是本地时本地那侧是 a，两侧同型时键小的那侧是 a。
        // 这里不自判第二遍，是因为界面上那张卡的 a/b（`src/pairs.ts` 读 `JobView.pairSlot`）
        // 与锁用的是同一个函数。
        let src_is_a = pair_slot_of(src, dst) == "a";
        let (a, b) = if src_is_a { (src, dst) } else { (dst, src) };
        let mirror_ok = mirror_sides_ok(src, dst);

        // 已经占住的格子不建（R94）。反面是影子：两张任务写同一格时界面上先到先得，
        // 看不见的那条才是真会跑的那条 —— 那一族在本仓有点名的历史。
        // 占没占按 (那一档的词, 它的 src 站的槽) 认，与界面摆那四格同一对值（`src/pairs.ts`）。
        let taken: BTreeSet<(&str, &str)> = self.jobs().iter()
            .filter(|j| j.kind != JobKind::Gate && pair_key_of(&j.src, &j.dst) == key)
            .map(|j| (j.kind.toml_word(), pair_slot_of(&j.src, &j.dst)))
            .collect();
        let legal: Vec<&(JobKind, &'static str)> = CELLS.iter()
            // `sync` 那一档要恰好一侧是云端；`copy` 两向恒合法（两侧都本地、两侧都云端都跑得动）。
            .filter(|(kind, _)| *kind != JobKind::Sync || mirror_ok)
            .filter(|(kind, slot)| !taken.contains(&(kind.toml_word(), *slot)))
            .collect();
        if legal.is_empty() {
            return Err(RunError::Invariant(draft_all_taken_refusal()));
        }

        // 编号（R99）：与配置里的比一次，撞了就进位；游标**只增不减**（`draft_seq` 头上那段是为什么）。
        // 顺序有意：编号先分配完、才做下面的位置检查，所以检查失败时游标已经进过位。
        // 宁可跳号，不许撞号 —— 跳号没人看得见，撞号是两张卡共用一份状态与一份凭证。
        let mut seq = self.draft_seq.lock().unwrap();
        let ids: Vec<String> = loop {
            let no = *seq;
            let candidate: Vec<String> = legal.iter()
                .map(|(kind, slot)| format!("adhoc-{no}-{}-{slot}", kind.toml_word()))
                .collect();
            *seq = no + 1;
            if candidate.iter().all(|id| !self.id_taken(id)) { break candidate; }
        };

        let built: Vec<Job> = legal.iter().zip(ids.iter())
            .map(|((kind, slot), id)| Job {
                id: id.clone(),
                name: DRAFT_NAME.to_string(),
                kind: kind.clone(),
                src: if *slot == "a" { a.to_string() } else { b.to_string() },
                dst: if *slot == "a" { b.to_string() } else { a.to_string() },
                // `open_dir` 与 `manifest` 故意留空：那是"这一条在配置里怎么写的"那一类信息，
                // 而临时卡没有那一份文件。对账要 manifest，所以临时卡压根没有对账那一档（spec §6）。
                open_dir: None, manifest: None,
            })
            .collect();
        // 位置本身的判据走配置侧那一段代码（非空之后还有：云端那一侧不许混反斜杠、不许有点段、
        // 以及 `sync` 那一档要恰好一侧是云端 —— 这一条在这里由 `mirror_ok` 决定建不建，不靠它报错）。
        for job in &built {
            job.check_positions()?;
        }
        self.drafts.lock().unwrap().extend(built);
        Ok(key)
    }

    /// 收起一张临时卡（R98）：把这一对位置上的那几条从内存里去掉。
    /// 正在跑（以及"跑完但没确证"）的那一张拒收 —— 收掉一张正在写的卡，等于让一次写动作
    /// 在界面上失去落脚的卡，而看不见的写是本仓最贵的那一类事故。
    ///
    /// 这条判据读的是状态表，与"刚开始的那一趟"之间有几十微秒的窗口；界面上那颗按钮是**人**点的，
    /// 而这一族真正的护栏是那把锁：收卡不会打断已经提交的动作（它拿着自己的 guard 跑完），
    /// 只让它从卡上消失。要的是"消失"这件事本身被拒。
    pub async fn drop_draft(&self, pair_key: &str) -> Result<(), RunError> {
        let mine: Vec<Job> = self.drafts.lock().unwrap().iter()
            .filter(|j| pair_key_of(&j.src, &j.dst) == pair_key)
            .cloned().collect();
        if mine.is_empty() {
            return Err(RunError::Invariant(draft_absent_refusal()));
        }
        for job in &mine {
            let st = self.state(&job.id).await;
            // 判据是**排除式**的：只有"确证结束了"那三档才放行（`Idle` = 这台面板上没跑过，
            // `Done`/`Failed` = `job/status` 确证过 finished）。`Unknown` 单列一句，因为它要说的是
            // "没确证"而不是"正有事在跑"；而**将来新增的任何状态**落在排除式外面 = 拒。
            // 写成 match 穷尽那六个变体再加一条 `_` 的话，那条 `_` 今天就不可达（警告），
            // 而它的用处恰恰是明天多出一个在跑的形状时别走进"允许收"那一支。
            if matches!(st, TaskState::Unknown { .. }) {
                return Err(RunError::Invariant(draft_unconfirmed_refusal()));
            }
            let settled = matches!(st, TaskState::Idle | TaskState::Done { .. } | TaskState::Failed { .. });
            if !settled {
                return Err(RunError::Invariant(draft_running_refusal()));
            }
        }
        self.drafts.lock().unwrap().retain(|j| pair_key_of(&j.src, &j.dst) != pair_key);
        Ok(())
    }

    /// 这一条是不是这一趟临时立起来的（R96 的判据本体）。
    ///
    /// 为什么由后台说而不是界面按编号前缀猜：界面上「存成常用」「删掉这张临时卡」那两颗与那句
    /// "重开面板就没了"都读这一枚，而 `adhoc-` 那一段是**日志文件名**的用处（出事时认得出
    /// 是哪张卡哪一格，R99），不是给界面用的判据。两处各认一遍，漂开的那一次就是
    /// 把一条真配置项当成临时卡收掉。
    pub fn is_draft(&self, id: &str) -> bool {
        self.drafts.lock().unwrap().iter().any(|j| j.id == id)
    }

    /// 这个编号有没有人用过：配置里那一批，以及这一趟已经立起来的临时格。
    /// 用 `self.jobs` / `drafts` 而不是 `Self::job()`，是为了不在 `draft_seq` 那把锁里再绕一层合成。
    fn id_taken(&self, id: &str) -> bool {
        self.jobs.iter().any(|j| j.id == id)
            || self.drafts.lock().unwrap().iter().any(|j| j.id == id)
    }
}
