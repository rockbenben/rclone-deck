use std::collections::BTreeSet;

use rclone_deck_lib::{gate, prune};

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn diff_reports_the_keys_the_bucket_is_missing() {
    let expected = set(&["p/a.webp", "p/b.webp", "p/c.webp"]);
    let actual = set(&["p/a.webp"]);

    let report = gate::diff(&expected, &actual, vec![]);
    assert!(!report.passed());
    assert_eq!(report.missing, vec!["p/b.webp".to_string(), "p/c.webp".to_string()]);
    assert_eq!(report.expected_count, 3);
    assert_eq!(report.actual_count, 1);
}

#[test]
fn diff_passes_when_the_bucket_holds_extra_objects() {
    // 桶里多出来的东西是 prune 的事，不该让 gate 拦住发布。
    let expected = set(&["p/a.webp"]);
    let actual = set(&["p/a.webp", "p/stale.webp"]);

    let report = gate::diff(&expected, &actual, vec![]);
    assert!(report.passed(), "{report:?}");
    assert!(report.missing.is_empty());
}

#[test]
fn diff_passes_on_an_exact_match() {
    let keys = set(&["p/a.webp", "p/b.webp"]);
    assert!(gate::diff(&keys, &keys, vec![]).passed());
}

#[test]
fn case_conflicts_are_reported_but_do_not_fail_the_gate() {
    let expected = set(&["p/a.webp"]);
    let actual = set(&["p/a.webp", "p/A.webp"]);

    let report = gate::diff(&expected, &actual, vec![vec!["p/A.webp".into(), "p/a.webp".into()]]);
    assert!(report.passed());
    assert_eq!(report.case_conflicts.len(), 1);
}

/// 三个数字必须互不相同，否则"缺失"那格填错也测不出来。
/// 上一版用 `expected={a,b}` / `actual={a}`：2 / 1 / 1，`缺失 {actual_count}` 与
/// `缺失 {missing.len()}` 输出**完全一样**，`缺失 {expected_count}` 也因为话术里别处有 '1' 而照过——
/// 这条是给操作员看的、决定他发不发发布的话，数字错了整个闸门就白装。
#[test]
fn summary_names_the_missing_count() {
    let report = gate::diff(&set(&["a", "b", "c"]), &set(&["a"]), vec![]);
    let s = report.summary();
    assert!(s.contains("应有 3 个对象"), "{s}");
    assert!(s.contains("桶内 1 个"), "{s}");
    assert!(s.contains("缺失 2 个"), "{s}");
}

/// 通过分支以前**没有任何测试**：`summary()` 那两行可以改成任何话（包括改成失败分支那句），
/// 8 条测试照样全绿。锚点必须写 `闸门通过：`——不能锚裸的 `通过`，因为"闸门未通过"里也含它。
#[test]
fn summary_reports_both_counts_when_it_passes() {
    let keys = set(&["p/a.webp", "p/b.webp"]);
    let s = gate::diff(&keys, &keys, vec![]).summary();
    assert!(s.starts_with("闸门通过："), "{s}");
    assert!(s.contains("应有 2 个对象"), "{s}");
    assert!(s.contains("桶内 2 个"), "{s}");
}

/// R29 的核心一条：本地什么都没扫到**绝不是绿信号**。
/// `missing` 在数学上必然为空，所以判据不能是"再算一遍差集"，必须是 `expected_count`。
#[test]
fn an_empty_expectation_is_not_a_passing_gate() {
    let report = gate::diff(&set(&[]), &set(&["p/a.webp", "p/b.webp"]), vec![]);
    assert_eq!(report.verdict, gate::GateVerdict::EmptyBasis, "{report:?}");
    assert!(!report.passed(), "空对账基准不能放行：{report:?}");
    assert!(report.missing.is_empty(), "缺失集合本身仍是空的（笨的差集不变）");
    assert!(report.summary().contains("无从对账"), "{}", report.summary());
}

/// 裁定 R56 item 1 的证人。旧代码在这一格给的正是 `Pass`：判据写着
/// `expected.is_empty() && !actual.is_empty()`，于是"本地一个引用都没扫到、桶里也正好是空"
/// 绕过了 `EmptyBasis`，`summary()` 说出"全部就位"——而 `EmptyBasis` 那档的注释点名的
/// 就是这句假话，它对两边都空同样成立。判据只看 `expected`：闸门交出去的是**发布许可**，
/// `expected_count == 0` 时它没有确证过任何东西，无论桶里有没有东西。
/// 破坏一次（必须变红）：把 `src/gate.rs` 的判据改回 `expected.is_empty() && !actual.is_empty()`。
#[test]
fn both_sides_empty_is_still_not_a_passing_gate() {
    let report = gate::diff(&set(&[]), &set(&[]), vec![]);
    assert_eq!(report.verdict, gate::GateVerdict::EmptyBasis, "{report:?}");
    assert!(!report.passed(), "两边都空同样什么都没确证，不许放行：{report:?}");
    // 那句"桶内 0 个"是照实说出来的，不是省略出来的。
    let s = report.summary();
    assert!(s.contains("无从对账"), "{s}");
    assert!(s.contains("桶内 0 个"), "{s}");
    assert!(!s.contains("全部就位"), "{s}");
}

/// 判据的另一半：`actual` **不参与**判定。桶是空的而本地确实扫到了东西，那是彻头彻尾的
/// 缺失（一个都没传上去），不是"没有基准可对"。这一条挡住把判据写过头的写法
/// （`expected.is_empty() || actual.is_empty()`），那种写法会把最该拦住的场景报成无从对账。
#[test]
fn an_empty_bucket_with_a_real_expectation_is_missing_not_empty_basis() {
    let report = gate::diff(&set(&["p/a.webp", "p/b.webp"]), &set(&[]), vec![]);
    assert_eq!(report.verdict, gate::GateVerdict::Missing, "{report:?}");
    assert!(!report.passed());
    assert_eq!(report.missing, vec!["p/a.webp".to_string(), "p/b.webp".to_string()]);
}

/// `expected` 非空且全命中——上面那条把判据收紧之后，这一格**必须还是** `Pass`，
/// 否则闸门就在拒绝它唯一该放行的东西。`verdict` 写死而不是靠 `passed()`，
/// 是为了让它落错档时（比如落进 `EmptyBasis`）也变红。
#[test]
fn a_non_empty_expectation_that_all_hits_is_a_pass() {
    let keys = set(&["p/a.webp", "p/b.webp", "p/c.webp"]);
    let report = gate::diff(&keys, &keys, vec![]);
    assert_eq!(report.verdict, gate::GateVerdict::Pass, "{report:?}");
    assert!(report.summary().starts_with("闸门通过："), "{}", report.summary());
}

/// R29 的另一半：本地清单为空、桶里却有东西时，差集会交出**整个桶**。
/// 内核照旧给全量（不许在这里过滤名字），但要打上 `empty_basis`，
/// 让 Task 9 能拒绝、让 UI 渲染不出"这就是删除清单"。
#[test]
fn a_local_scan_that_found_nothing_marks_the_basis_empty() {
    let report = prune::orphans(&set(&[]), &set(&["p/a.webp", "p/b.webp"]));
    assert!(report.empty_basis, "{report:?}");
    assert_eq!(report.orphans.len(), 2, "集合差不被改写——过滤不是这里的职责");
    assert_eq!(report.local_count, 0);
    assert_eq!(report.remote_count, 2);
}

/// 两边都空才是"真的没什么可清"。上面那条不许顺手把它也标成基准缺失。
///
/// 这一条和上面 `both_sides_empty_is_still_not_a_passing_gate` **对同一个输入给出相反的结论，
/// 是有意为之**（裁定 R56 item 1），不是这里漏改：gate 交出去的是发布许可，`expected` 为空
/// 就等于什么都没确证；prune 守的是"整个桶被变成删除候选"，两边都空时孤儿集本来就是空集。
/// 两条同时钉着，谁想把两把尺子对齐就会红一条。
#[test]
fn both_sides_empty_is_not_an_empty_basis_for_prune() {
    let report = prune::orphans(&set(&[]), &set(&[]));
    assert!(!report.empty_basis, "{report:?}");
    assert!(report.orphans.is_empty());
}

#[test]
fn orphans_are_the_remote_keys_that_are_no_longer_local() {
    let local = set(&["p/a.webp"]);
    let remote = set(&["p/a.webp", "p/002.webp", "p/old-name.webp"]);

    let report = prune::orphans(&local, &remote);
    assert_eq!(report.orphans, vec!["p/002.webp".to_string(), "p/old-name.webp".to_string()]);
    assert_eq!(report.local_count, 1);
    assert_eq!(report.remote_count, 3);
}

#[test]
fn orphans_never_lists_a_key_that_is_only_local() {
    // 本地有、桶里还没有 = 待上传。把待上传的东西列进"可删除"是本工具最坏的失败方式。
    let local = set(&["p/new.webp", "p/kept.webp"]);
    let remote = set(&["p/kept.webp"]);

    let report = prune::orphans(&local, &remote);
    assert!(report.orphans.is_empty(), "{:?}", report.orphans);
}

#[test]
fn orphans_is_empty_when_both_sides_match() {
    let keys = set(&["p/a.webp"]);
    assert!(prune::orphans(&keys, &keys).orphans.is_empty());
}

/// R29 的结论靠这两个名字传出去。改派生属性就会让 Task 11 读到 `undefined`，
/// 而 `emptyBasis` 读成 `undefined` 是 falsy——保险自动失效，所以这条必须有。
/// 在这条之前全 crate 从没序列化过 `GateReport` / `PruneReport`，于是
/// `#[serde(rename_all = "camelCase")]` 与 enum 的 `snake_case` 都是没人看着的字段名：
/// 删掉派生属性、或把 `snake_case` 换成 `kebab-case`，其余测试照样全绿。
/// 破坏一次（必须变红）：把 `src/prune.rs` 里 `PruneReport` 上的
/// `#[serde(rename_all = "camelCase")]` 删掉 → `empty_basis` 而不是 `emptyBasis`。
#[test]
fn the_r29_verdict_fields_reach_the_wire_under_those_exact_names() {
    let report = gate::diff(&set(&[]), &set(&["p/a.webp"]), vec![]);
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"verdict\":\"empty_basis\""), "{json}");
    assert!(json.contains("\"expectedCount\":0"), "{json}");
    assert!(json.contains("\"actualCount\":1"), "{json}");

    let prune = prune::orphans(&set(&[]), &set(&["p/a.webp"]));
    let json = serde_json::to_string(&prune).unwrap();
    assert!(json.contains("\"emptyBasis\":true"), "{json}");
    assert!(json.contains("\"orphans\":[\"p/a.webp\"]"), "{json}");
    assert!(json.contains("\"remoteCount\":1"), "{json}");
    // §4.2 的另一半也得上线。字段名同样钉死：改名等于让前端读到 `undefined`，
    // 而 `undefined.length` 在 `report.pendingUpload.length` 那一句里是 TypeError，不是"空清单"。
    assert!(json.contains("\"pendingUpload\":[]"), "pendingUpload 没上线：{json}");
    // 那一份名单文件的路径也得上线：界面那一行"名单存成了 …"读的就是它。
    // 内核自己不知道路径（写盘是 runner 的事），所以这一条同时钉住"没填时是空串而不是缺席"——
    // 缺席的话前端 `report.listFile !== ''` 会读成 `undefined !== ''` = true，凭空一行。
    assert!(json.contains("\"listFile\":\"\""), "listFile 没上线（或空的时候不是空串）：{json}");
}

/// 两半是同一个 kernel 的两次调用（spec §4.2）。这条钉的是"另一侧不是新功能"：
/// 同夹具下 `orphans` 与 `pending_upload` 必须正好互为反方向。
#[test]
fn the_two_halves_are_the_same_difference_turned_around() {
    let local = set(&["a.webp", "only-local.webp"]);
    let remote = set(&["a.webp", "stale.webp"]);
    let r = prune::orphans(&local, &remote);
    assert_eq!(r.orphans, vec!["stale.webp"]);
    assert_eq!(r.pending_upload, vec!["only-local.webp"]);
    assert_eq!((r.local_count, r.remote_count), (2, 2));

    // 空 src 那一档：orphans 不可信（emptyBasis=true），但"还没上传的"仍然可信——
    // 它是 `local \ remote`，本地为空时它就是空集，没有骗人的东西。
    let empty_local = prune::orphans(&set(&[]), &remote);
    assert!(empty_local.empty_basis);
    assert!(empty_local.pending_upload.is_empty(), "本地为空却说得出要上传什么，那是假的");

    // 桶是空的：孤儿集为空、没有危险（R56 那一条口径不许动），而"还没上传的"就是本地全部。
    let empty_remote = prune::orphans(&local, &set(&[]));
    assert!(!empty_remote.empty_basis);
    assert_eq!(empty_remote.pending_upload, vec!["a.webp", "only-local.webp"]);
}
