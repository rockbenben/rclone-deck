//! 一对位置的坐标系：哪两条任务算"同一对"、这一对里谁站 a、以及这一对能不能当对齐用。
//!
//! 为什么住在这一层而不是 `state.rs`（它原来就在那里）：这套判据的读者不止界面那一张卡——
//! `runner::drafts` 要按它决定"这张卡上还缺哪几格"（裁定 R94），`config/jobs.rs` 要按其中
//! 那一条拒绝没有权威一端的对齐。而 `state` 是**视图**那一层，`runner` 不许反过来依赖它
//! （那条规矩写在 `config/jobs.rs` 的 `JobKind::word()` 头上）。放在这里，三个读者读的是同一处。
//!
//! 为什么键必须由后台算：这把键问的是"是不是同一对目录"，而**锁问的是同一件事**
//! （`Scope::conflicts`：同一棵树、大小写不敏感、两种斜杠都归一）。两份口径迟早会漂，
//! 漂开的那一次就是"两张卡按下去动的是同一棵树"，或者"同一对目录裂成两张卡"。
use crate::endpoint::endpoint_of;
use crate::scope::Scope;

/// 一侧位置的键（`Scope` 的键串）。与 [`pair_key_of`]、[`pair_slot_of`] 必须同一条规则，
/// 所以它在这里而不在调用方各拼一遍。
pub fn endpoint_key(raw: &str) -> String {
    Scope::of_endpoint(&endpoint_of(raw)).key()
}

/// 一对位置的键：两侧各走一遍 `Scope`，取键、排序、拼接。
/// **方向与 kind 都不进键** —— 方向是这一对上的一个选择，不是身份
/// （「以本地为主、以云端为主应该是选择框，而不是 N 条不同的条目」）。
/// 排序本身就是"方向不进键"这件事。
pub fn pair_key_of(src: &str, dst: &str) -> String {
    let mut sides = [endpoint_key(src), endpoint_key(dst)];
    sides.sort();
    sides.join("|")
}

/// 这一条站 a 还是 b：恰好一侧是本地时，本地那一侧是 a；两侧同型（都本地或都 remote）时
/// 键小的那一侧是 a。与 [`pair_key_of`] 同一条规则，所以两者必须一起改。
///
/// 它跟着 `JobView` 上线，是因为前端只做**划分**——它若自己再判一次"哪侧是本地、谁排前面"，
/// 就是第二份口径，而漂开的那一次是"同一对目录裂成两张卡"或"两张卡动同一棵树"。
pub fn pair_slot_of(src: &str, dst: &str) -> &'static str {
    let (se, de) = (endpoint_of(src), endpoint_of(dst));
    let (sl, dl) = (se.as_local().is_some(), de.as_local().is_some());
    if sl != dl {
        return if sl { "a" } else { "b" };
    }
    if endpoint_key(src) <= endpoint_key(dst) { "a" } else { "b" }
}

/// 这一对位置**能不能当对齐用**：恰好一侧是云端。
///
/// 判据的出处是 `config/jobs.rs` 那条拒绝（原来写在那里内联比较两侧的 `is_remote()`）。
/// 抽到这一处不是为了好看：临时配对卡（`runner::drafts`）也要按同一件事决定"这张卡上建不建
/// 「会删除」那两格"（裁定 R94），两处各比一遍的话，漂开的那一次就是"面板上多出一个没有权威
/// 一端的删除入口"。
/// 两侧都是本地、或两侧都是云端，对 rclone 都合法，但面板交不出一份可核对的删除清单
/// ——没有权威的一端就没有删除范围，所以两边都不建，而不是猜一个当权威。
pub fn mirror_sides_ok(src: &str, dst: &str) -> bool {
    endpoint_of(src).is_remote() != endpoint_of(dst).is_remote()
}

/// 这一对是不是**同一处**位置（两种斜杠、大小写、结尾斜杠归一之后）。
/// 判据就是 [`endpoint_key`] 相等——不许再拿 `Scope` 比一遍：那两份"是不是同一棵树"
/// 迟早漂，而漂开的那一次是一张卡按下去动了另一张卡的树。
/// 临时卡拿它拒绝"自己对自己对齐"：那种形状既没有"多余"可删，也没有要补的东西，
/// 而界面上它会是一张标题把同一个目录写两遍的卡。
pub fn same_position(src: &str, dst: &str) -> bool {
    endpoint_key(src) == endpoint_key(dst)
}
