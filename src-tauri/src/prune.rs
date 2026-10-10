//! 盘点内核（裁定 R58 之后**只读**）：`orphans` = 远端 − 本地，就是「本地没有的云端对象」。
//! 它曾经是 prune 功能的集合差内核；prune 的执行侧（勾选、确认闸、逐 key 删除）已整体删除，
//! 因为这个集合分不出"改名垃圾"与"用户直接传上云的正式资产"。内核留下，继续喂只读的盘点视图；
//! **任务这一侧**删除的唯一入口是 `sync`（对齐）；浏览器那一面另有按名删除 / `purge` / `rmdir`
//! 三条，不经过这里（各自的闸在 `runner::browser`）。模块名与 `PruneReport` 类型名保持原样：
//! `tests/gate.rs` 的序列化证人钉着 `emptyBasis`/`orphans` 这些线上字段名，改名没有收益。
//!
//! 裁定 R60 把这里升格成**有方向的集合差** `plan(src, dst)`（spec §4.1 第 1 步、§4.2）：
//! `dst \ src` 是"这次对齐会删掉什么"，两侧对调就是"还没上传的是什么"。`orphans` 现在是
//! `plan(local, remote)` 的薄封装，语义一字未动；对齐与盘点的另一半直接走 `plan`。
//! `RATIO_DEN` 是**假设值**，别把它当实测结论引用（校准的出口在它自己的注释上）。
use std::collections::BTreeSet;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneReport {
    /// 本地什么都没扫到、桶里却有东西。见裁定 R29：这时 `orphans` 等于整个桶，
    /// 空基准下的集合差不可信（几乎一定是 src/exclude 配错了）。
    /// 内核**不**因此改写集合差（过滤名字不是这里的职责），只负责把这个诊断随集合一起带出去，
    /// 让 `run_inventory` 能硬拒绝、让 UI 渲染不出"这就是本地没有的全部对象"。
    pub empty_basis: bool,
    pub orphans: Vec<String>,
    /// 本地有、桶里还没有 = **还没上传的**（spec §4.2 的另一半）。它绝不是删除理由，
    /// 列出来只是为了让人看得见 copy 要传什么——从前这一侧刻意不显示，因为界面上
    /// 一旦列出"多出来的东西"就总有人想给它配个删除按钮（R58 的整件事）。
    pub pending_upload: Vec<String>,
    pub local_count: usize,
    pub remote_count: usize,
    /// 这两个位置里 **dst 那一侧**是不是本地目录。内核只看得到两堆名字、看不到端点，所以这里
    /// 交出来恒为 `false`，由 `run_inventory` 按端点类型填上（与下面 `list_file` 同一族：谁有信息
    /// 谁填）。`orphans` 与 `pending_upload` 的字段名按**位置**走、不按方向走，而「这一档要删的是
    /// 哪一半」是方向的事 —— 读它的人不许在界面里拿路径再判一遍（R69 的禁令）。
    pub dst_is_local: bool,
    /// 这份差集是**什么时候**算的（spec §3：行上显示的是"上一次的结果 + 它是几点算的"）。
    /// 少了这一句，一次旧读数就会被当成现状 —— 而"看差在哪"是按需点的，两次点击之间
    /// 两侧都可能已经变了。长什么样由 `crate::moment::label` 一处定义
    /// （今天 14:22 / 昨天 21:40 / 10-01 09:12：面板可能整夜开着，只带时分的读数跨过午夜就说假话，
    /// 而秒那一档只会让行变得更难读——判据要稳，装饰不要）。
    ///
    /// 证人只断"非空"，**不**断具体时刻：一条需要挂钟的测试会随时区与跨分钟抖动而红
    ///（那是噪声，不是判据）。那一档的**形状**由 `moment.rs` 自己那三条测试守着。
    pub measured_at: String,
    /// 这一次顺手落的那份名单文件在哪（`runner::lists`）。内核**不知道**路径，所以交出来的是
    /// 空串，由 `run_inventory` 写完之后填上——界面上那一行"名单存成了 …"读的就是它，
    /// 空串那一档说"这一份没落盘"（拒绝的那一次就是空串，见 `runner_local` 那三条证人）。
    pub list_file: String,
}

/// 比例守卫的分母：`doomed` 达到 `dst` 侧的 `1 / RATIO_DEN` 就拒绝（阈值只在这一处定义）。
/// **这是假设值**，没标定过：真桶从没对齐过一次（真桶 dry-run 的许可仍未给，见 spec §4.1）。
/// 取这个数的理由是"正常对齐极少真要砍掉目标端一半以上"，而这一条抓不到的形状是"本地目录被误
/// 清空了一半 / 挂上来的只装了几张图"。校准数据**打算**取自预览时那份 doomed 与 dst 的比例
/// （spec §4.1 要的那条记录；它落在哪个字段、什么时候开始落，以预览实现为准，这里不钉名字）。
/// 在那条记录真的存在之前，它就是个没被量过的假设。
pub const RATIO_DEN: usize = 2;

/// 一次"有方向的集合差"的全部结论。对齐用它圈定删除范围，盘点用它的另一侧列"还没上传的"。
/// 这是内核内部的类型，**不上线**：走线的是 runner 的 `PreviewView`（字段名是给前端的），
/// 所以这里不派生 Serialize——派生了就等于承诺一个没人消费的 JSON 形状。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorPlan {
    /// dst 有、src 没有 = **这次对齐会删掉的那些**（相对 dst 自己的根，见 spec §4.1）。
    pub doomed: Vec<String>,
    /// src 有、dst 没有 = **这一趟会补上的那些**（对齐跑起来 copy 的那一半）。
    /// 它绝不是删除理由（那归 `doomed`），但它是"按下去会发生什么"的另一半：确认层只说
    /// "删这 N 个"，那一层就读起来像"这一档只能删东西"，于是没人按它（实测：
    /// 那 17 个连着几天没上去）。这里只带**个数**不带名字：名字列表归盘点那一份
    /// （`runner::lists`），而界面不许拿 `srcCount - (dstCount - doomed)` 去减它。
    pub pending: usize,
    pub src_count: usize,
    pub dst_count: usize,
    /// src 一个都没有而 dst 非空：`doomed` 等于整个 dst，这份结论不可信（R29 的推广）。
    ///
    /// 两边都空**不算**基准缺失：那是"确实没有东西"，孤儿集本来就是空集、没有任何危险。
    /// 裁定 R56 item 1：这把尺子和 `gate::diff` 的**故意不同**，别把它对齐过去——闸门交出去的是
    /// 发布许可，`expected` 为空就等于什么都没确证，所以那边不看 `actual`；这一侧守的是"整个
    /// 目标端被变成删除候选"，所以 `!dst.is_empty()` 这一半必须留着。
    pub empty_src: bool,
    /// `doomed` 够到了 `RATIO_DEN` 那条线（线画在哪只看那个常量）：形状像事故不像清理（R60）。
    pub too_much: bool,
}

/// 有方向的集合差。**这一层不做拒绝**，只把两个诊断位随集合一起带出去——拒绝是要写日志、
/// 要说得出该检查什么的动作，归 `runner`（与 R29 同一条口径：内核保持"笨的集合差"）。
pub fn plan(src: &BTreeSet<String>, dst: &BTreeSet<String>) -> MirrorPlan {
    let doomed: Vec<String> = dst.difference(src).cloned().collect();
    // 乘而不除：整数下 `doomed * RATIO_DEN >= dst` 就是"达到 1/RATIO_DEN"，不引浮点。
    // `>=` 不是 `>`：spec §4.1 写的是"达到一半就拒绝"，正好一半必须拦。
    // `!dst.is_empty()` 排的是"分母为零"那一档：dst 空时 doomed 必是空集，而 0 × RATIO_DEN >= 0
    // 是成立的，不排掉它，"往空目录里传第一批"就会被当成"整个目标端都要删"永远拦死。
    // 这一行排在结构体字面量**之前**：`doomed` 会被移进第一个字段，之后再 `doomed.len()` 就是用后借用。
    let too_much = !dst.is_empty() && doomed.len() * RATIO_DEN >= dst.len();
    // 反方向那一侧只取个数：与 `plan(dst, src).doomed.len()` 同值，就地算一次差集，
    // 不再调一次 `plan`（那会连两个诊断位一起白算，还多一份"哪一份才是真的"的疑问）。
    let pending = src.difference(dst).count();
    MirrorPlan {
        doomed,
        pending,
        src_count: src.len(),
        dst_count: dst.len(),
        empty_src: src.is_empty() && !dst.is_empty(),
        too_much,
    }
}

/// 孤儿 = 桶里有、本地没有。方向不能反：本地有桶里没有的是"待上传"，
/// 交给 copy，绝不是删除的理由。
///
/// 这里只是把 `plan` 的结果换个名字：参数顺序就是方向的定义（`src` = 本地、`dst` = 远端），
/// 所以 `empty_basis` 就是 `plan` 的 `empty_src`，它的判据（含"两边都空不算基准缺失"、
/// 以及它为什么与闸门那把尺子故意不同）记在 `MirrorPlan::empty_src` 上，别在这里另立一套。
pub fn orphans(local: &BTreeSet<String>, remote: &BTreeSet<String>) -> PruneReport {
    let p = plan(local, remote);
    // 反方向直接再调一次同一个 kernel：两个 doomed 互为这里的 orphans / pending_upload。
    // 不另写一份 `local.difference(remote)`——那才是真正会漂移的地方。
    let reverse = plan(remote, local);
    PruneReport {
        empty_basis: p.empty_src,
        orphans: p.doomed,
        pending_upload: reverse.doomed,
        // 内核不知道方向（参数就是两堆名字），这一枚留给调用方填，见字段文档。
        dst_is_local: false,
        local_count: p.src_count,
        remote_count: p.dst_count,
        // 时刻在**这里**取，而不是在界面层取：这份报告是这一次盘的，
        // 谁转手把它存起来，带的都该是同一个时刻（行上那句"几点算的"说的就是这一行代码跑过的时间）。
        // 那一档长什么样归 `moment::label`（今天/昨天/带月-日），与对账报告同一处定义。
        measured_at: crate::moment::now_label(),
        list_file: String::new(),
    }
}
