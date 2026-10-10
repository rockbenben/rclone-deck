use std::collections::BTreeSet;

use serde::Serialize;

/// 三态而不是 bool：`passed()` 是方法、serde 不序列化方法，前端一旦自己从 `missing.length`
/// 推结论，Rust 侧的判断就被绕过了（裁定 R29）。`verdict` 是**序列化出去的结论**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateVerdict {
    /// 该有的都在，且本地确实扫到了东西。
    Pass,
    /// 有缺失，拦住发布。
    Missing,
    /// 本地一个对象都没扫到 —— 这不是"都齐了"，这是 src 或 exclude 配错了。
    /// 桶里空不空都一样：这一档问的是"有没有基准可对"，不是"桶长什么样"。
    EmptyBasis,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateReport {
    pub verdict: GateVerdict,
    pub missing: Vec<String>,
    pub case_conflicts: Vec<Vec<String>>,
    pub expected_count: usize,
    pub actual_count: usize,
    /// 这份结论是**什么时候查的**（`moment::label` 那一档：今天 14:22 / 昨天 21:40 / 10-01 09:12）。
    /// 它与 `prune::PruneReport::measured_at` 同一族，也守同一条理由：交出去的是**发布许可**，
    /// 一次旧许可被当成现状就是一次带 404 的发布。界面不许自己取"现在"——那是第二处口径。
    pub measured_at: String,
}

/// 判定口径是"桶里存不存在这个 key"，不发 HTTP 探测——
/// `img.aishort.top` 的 404 带一年缓存头，探测结果不可信（spec §5）。
pub fn diff(
    expected: &BTreeSet<String>,
    actual: &BTreeSet<String>,
    case_conflicts: Vec<Vec<String>>,
) -> GateReport {
    let missing: Vec<String> = expected.difference(actual).cloned().collect();
    // 差集照旧是笨的差集；verdict 只额外看"有没有基准可对"。
    //
    // 裁定 R56 item 1：判据**只看 `expected.is_empty()`，不看 `actual`**。闸门交出去的是
    // **发布许可**，它的价值全在"我确证过了"；本地一个引用都没扫到时，无论桶里有没有东西，
    // 它一样都没确证过。带 `!actual.is_empty()` 的旧判据会把"两边都空"那一格放进 `Pass`，
    // 于是下面 `EmptyBasis` 注释点名的那句假话（"全部就位"）就由这个内核自己说了出来。
    //
    // `prune::orphans` 的 `empty_basis` 是**另一把尺子，故意不跟着改**（见那边注释）：
    // 那里守的是"整个桶被当成删除候选"，两边都空时孤儿集本来就是空集，没有危险。
    let verdict = if expected.is_empty() {
        GateVerdict::EmptyBasis
    } else if missing.is_empty() {
        GateVerdict::Pass
    } else {
        GateVerdict::Missing
    };
    GateReport {
        verdict,
        missing,
        case_conflicts,
        expected_count: expected.len(),
        actual_count: actual.len(),
        // 取"现在"的这只手在后台：这份报告是这一次对账的产物，谁转手存起来带的都该是同一时刻。
        measured_at: crate::moment::now_label(),
    }
}

impl GateReport {
    pub fn passed(&self) -> bool {
        self.verdict == GateVerdict::Pass
    }

    pub fn summary(&self) -> String {
        match self.verdict {
            GateVerdict::Pass => format!(
                "闸门通过：应有 {} 个对象，桶内 {} 个，全部就位",
                self.expected_count, self.actual_count),
            GateVerdict::Missing => format!(
                "闸门未通过：应有 {} 个对象，桶内 {} 个，缺失 {} 个",
                self.expected_count, self.actual_count, self.missing.len()),
            // "全部就位"在这种输入下是假话：一个都没扫到，所以什么都不知道。
            // 这一句因此把 `actual_count` 照实报出来，两边都空时就写着"桶内 0 个"——
            // 不许为了好看把它省掉或换成别的数（裁定 R56 item 1）。
            GateVerdict::EmptyBasis => format!(
                "闸门无从对账：本地一个对象都没有（应有 0 个），桶内 {} 个。\
                 这不是「都齐了」，先检查 src 指向与 exclude 是否过宽",
                self.actual_count),
        }
    }
}
