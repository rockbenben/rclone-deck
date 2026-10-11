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
    /// 「两边都有、但不是同一份」的那一批（2026-10-11 新增，详见 [`diverged`]）。
    ///
    /// 它**不在**上面那两个差集里：两边都有的名字在两个差集中同时消失。这是第三类，
    /// 也是唯一一类跑一次 `sync` 就会被**静默覆盖**的——所以它必须单独上线。
    ///
    /// 序列化形状是 `Vec<DivergedEntry>`，而那一族在界面上是**一格一格看**的（`verdict`
    /// 与 `newer` 各占一列），不是两列名字。`empty` 时是空数组而不是缺席。
    pub diverged: Vec<DivergedEntry>,
}

/// 一条「两边都在、但不是同一份」。**这两枚字段缺一不可**：
/// `verdict` 说"是不是同一份"，`newer` 说"哪边比较新" —— 而后者才是人做决定要的那一半。
///
/// `verdict` 上线是 `differs` / `unknown` 两个值。**没有 `same`**：相同的那些根本不该
/// 占一行（真实图库里绝大多数文件两边一样，全列出来这一屏就没法看了）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DivergedEntry {
    /// 相对**本地**根的名字（`side_meta` 两边都退到各自的根，与 `orphans` 同一坐标系）。
    pub name: String,
    /// `differs` = 对不上且说得出谁新；`unknown` = 对不上但说不出（时间戳缺或相等）。
    pub verdict: &'static str,
    /// `remote` = 云端那份更新（**按上传会覆盖它**，最要紧的那一格）；
    /// `local` = 本地更新；`null` = 说不出（与 `verdict: unknown` 同时出现）。
    pub newer: Option<&'static str>,
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
        // 这一份是**盘点时的值**，而 `orphans()` 这个内核只看得到两堆名字、算不出它
        // （算它要 stat，见 `runner::run_inventory`）。所以这里交空数组，由调用方填 ——
        // 与 `list_file` / `dst_is_local` 同一族：谁有信息谁填。
        // 空数组而不是缺席：前端 `report.diverged.length` 在缺席时会读到 undefined。
        diverged: Vec::new(),
    }
}

// ───────────────────────── 同一个文件，两边是不是同一份 ─────────────────────────

/// 一个文件在**两侧的元数据**。名字是坐标，别的字段是判据 —— 这就是与上面
/// `BTreeSet<String>` 那几行的全部差别：集合差只认"在不在"，而本类型回答"是不是同一份"。
///
/// 只喂**两边都有名字**的那批（见 [`diverged`] 的入参口径）。`mod_time` 是 `Option`，
/// 拿不到就是拿不到，不拿"相同"顶替 —— 见 [`Verdict`] 里 `Unknown` 那一档。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub name: String,
    pub size: u64,
    pub mod_time: Option<String>,
}

/// 一个"两边都在、但不是同一份"的文件，**以及哪一边比较新**。
///
/// `who` 是这一份存在的理由：光说"这两个不一样"没用，界面上要能说"云端那份比你本地新，
/// 按上传会把云端的改动冲掉"。而没有 `who` 的话，界面只能让人自己去比时间戳 ——
/// 那是把判断推回给人，而这一层存在的意义就是不推。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Newer {
    /// 云端那份更新 —— **按上传会覆盖它**，这是最危险的一格。
    Remote,
    /// 本地那份更新 —— 按上传是对的。
    Local,
}

/// 三档判定，不是两档。
///
/// `Unknown` 单独立一档是有意的：时间戳读不到（某些后端不给、或 stat 失败）时，
/// 把它读成"相同"就是放行一次覆盖，读成"不同"则是无中生有一份要人看的清单。
/// 唯一不撒谎的位置是第三档：既不放过，也不假装看得见。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// 大小与修改时间都对得上 —— 这一份**不**进名单。
    Same,
    /// 对不上，且比得出哪边新。
    Differs { newer: Newer },
    /// 比不出（时间戳缺一边）：**要人知道**，但说不出谁新。
    Unknown,
}

/// 两级判定（2026-10-11）。
///
/// 第一级：大小或修改时间对不上，就已经不是同一份，**不必**再比哈希。
/// 第二级：只有第一级判成"一样"、而用户要求确认的那批，才去算哈希 —— 全量哈希要下载
/// 每一个对象，真实图片库动辄几千张，那是分钟级的代价，而绝大多数文件在第一级就被放行了。
///
/// **为什么先比时间而不是先比哈希**：改动一次内容，mtime 几乎必然变；而"内容变了但
/// mtime 没变"要同时踩两个坑（时钟精度、或复制保留了时间戳），在真实图库里远少于
/// "两边都是老照片、mtime 被 touch 过"的噪声。粗筛宁可多送几个去比哈希，也不能漏。
///
/// 哈希这一级是**留给调用方的钩子**：`hash_of` 传 `None` 就只跑第一级（默认），
/// 传 `Some(闭包)` 才在"第一级说一样、但你要求确认"的那一批上调用它。
/// 内核不自己算哈希：那是 IO 与后端相关的事，属于 `runner` 那一层。
/// 哈希钩子的形状：给一个名字，问"本地那份与云端那份的哈希各是什么"。
///
/// 返回 `(None, _)` / `(_, None)` = 拿不到，`None` 整体 = 这个名字没问成。
/// 三种都**不是**"两边一样" —— 判据在 [`Verdict`] 的注释里。
pub type HashOf<'a> = &'a dyn Fn(&str) -> Option<(Option<String>, Option<String>)>;

pub fn verdict_of(
    local: &FileMeta,
    remote: &FileMeta,
    hash_of: Option<HashOf<'_>>,
) -> Verdict {
    // 第一级：任一能拿到的对不上，就是它。真不同了，哈希只会再确认一遍，付那份代价不划算。
    // 注意大小对不上时**仍然要比时间**：大小变了、方向却说不出来，是 `Unknown` 而不是 `Differs`。
    if local.size != remote.size {
        return match newer_of(local, remote) {
            Some(n) => Verdict::Differs { newer: n },
            None => Verdict::Unknown,
        };
    }
    match (&local.mod_time, &remote.mod_time) {
        (Some(l), Some(r)) if l != r => {
            return match newer_of(local, remote) {
                Some(n) => Verdict::Differs { newer: n },
                None => Verdict::Unknown,
            }
        }
        // 一边缺时间戳：大小相同，时间未知 —— 这一档**不许**判成 Same。
        (None, _) | (_, None) => return Verdict::Unknown,
        _ => {}
    }
    // 第二级：大小与时间都一致。要不要花代价去确认，由调用方决定。
    //
    // 哈希确认出来的"不一样"**落 `Unknown` 而不是 `Differs`**：哈希没有次序，它能证明
    // 两份内容不同，证明不了哪一份更晚。而 mtime 恰好相等恰恰是"无法判断方向"的那一档 ——
    // 给一个 `Newer` 就是把推断当实测，而这一份是要给人当"往哪边合"看的，编错方向比不判更坏。
    //
    // **拿不到哈希的那几种形状一律落 `Unknown`，不落 `Same`**（2026-10-11 修）：
    // 本函数这一段的契约是"判据不足就不给结论"，而早先写的 `_ => {}` 会让下面那句
    // `Verdict::Same` 接住它——于是"问了两边都没问出来"被读成"两边一样"，那是一次放行，
    // 正是三档设计要关掉的那扇门。三种形状：`(None, None)` / `(None, Some)` /
    // `(Some, None)` / 整体 `None`（这个名字没问成）。
    if let Some(f) = hash_of {
        match (f(&local.name), f(&remote.name)) {
            // 两边都问到且不同 → 内容确实不是同一份，但方向给不出。
            (Some((Some(lh), Some(rh))), _) if lh != rh => return Verdict::Unknown,
            // 两边都问到且相同 → 这才是真的 Same。
            (Some((Some(_), Some(_))), _) => return Verdict::Same,
            // 其余全是"问不出来"，与 Same 无关。
            _ => return Verdict::Unknown,
        }
    }
    Verdict::Same
}

/// 哪一边更新。**拿不出就是拿不出** —— 返回 `None` 的那一档由调用方落成 `Unknown`。
///
/// 时间戳按字符串比较：rc 给的是 RFC3339（`2026-10-11T07:36:23.000000000+08:00` 那一族），
/// 同一时区下字典序与时间序一致。**跨时区或精度不同则不可比**，那种情况宁可交 `Unknown`：
/// 这一份的产物是要给人当"往哪边合"的依据的，方向错比方向缺更坏。
fn newer_of(local: &FileMeta, remote: &FileMeta) -> Option<Newer> {
    match (&local.mod_time, &remote.mod_time) {
        (Some(l), Some(r)) => {
            if l == r {
                // 时间戳完全相等：方向真的分不出来。哈希也不提供次序。
                None
            } else if r > l {
                Some(Newer::Remote)
            } else {
                Some(Newer::Local)
            }
        }
        _ => None,
    }
}

/// 「两边都在、但不是同一份」的名单。第一版是**纯 Rust 的纯名单**，界面上没有它的位置
/// （`PruneReport` 那两张表摆的是"谁在谁不在"，而这一份是"谁在谁也在但内容变了"）。
///
/// 为什么这一份不能由上面那个 `plan` 推出来：集合差里"两边都有"的名字会**在两个
/// 差集里同时消失**。也就是说，一个被云端改过、本地还是旧版的文件，在 `plan` 眼里
/// 完全不存在——它既不是"待上传"，也不是"会被删"，跑一次 `sync` 就被静默覆盖了。
/// 这不是漏报，这是那套口径**看不见这一类**。
///
/// 入参是两个 `FileMeta` 的映射，**只含两边都有名字的交集**（差集那两半由 `plan` 负责）。
pub fn diverged(
    local: &std::collections::BTreeMap<String, FileMeta>,
    remote: &std::collections::BTreeMap<String, FileMeta>,
    hash_of: Option<HashOf<'_>>,
) -> Vec<(String, Verdict)> {
    let mut out: Vec<(String, Verdict)> = local
        .iter()
        .filter_map(|(name, l)| {
            let r = remote.get(name)?;
            match verdict_of(l, r, hash_of) {
                Verdict::Same => None,
                v => Some((name.clone(), v)),
            }
        })
        .collect();
    // 名字升序：这一份是给人读的（"先看这几个"），顺序必须稳定，否则两次盘点对不上。
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
