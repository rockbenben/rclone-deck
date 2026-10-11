use std::collections::{BTreeMap, BTreeSet};

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

// ───────────────── 同一个文件、两边是不是同一份（2026-10-11 新增） ─────────────────

fn meta(name: &str, size: u64, t: Option<&str>) -> prune::FileMeta {
    prune::FileMeta { name: name.into(), size, mod_time: t.map(|s| s.to_string()) }
}

fn side(items: &[prune::FileMeta]) -> BTreeMap<String, prune::FileMeta> {
    items.iter().map(|m| (m.name.clone(), m.clone())).collect()
}

/// **这一条是整套判据存在的理由**，而它的形状是"集合差看不见这件事"：
///
/// `a.webp` 两边都有，所以它在 `local \ remote` 与 `remote \ local` 里**同时消失** ——
/// 盘点不说它、待上传不说它、会被删也不说它。可它的大小变了，也就是说**云端那份和你本地那份
/// 不是同一张图**。按上传会静默覆盖云端，而这一整轮盘点从头到尾没提过它一个字。
///
/// 所以"两边都有但不是同一份"必须是**另一条线**（[`prune::diverged`]），不是差集的第三半。
/// 哪天有人想把这里并回 `plan`（"加个 changed 集合不就行了"），这一条会红。
#[test]
fn a_file_that_exists_on_both_sides_is_invisible_to_the_set_difference() {
    let names = set(&["a.webp", "only-local.webp"]);
    let r = prune::orphans(&names, &set(&["a.webp", "only-remote.webp"]));

    // 集合差这一侧：`a.webp` 两边都有，一个差集都不在它里面。
    assert!(!r.orphans.contains(&"a.webp".to_string()), "不该被当成云端多出来的");
    assert!(!r.pending_upload.contains(&"a.webp".to_string()), "也不该被当成待上传的");
    assert_eq!(r.orphans, vec!["only-remote.webp"]);
    assert_eq!(r.pending_upload, vec!["only-local.webp"]);

    // 而新内核认得它 —— 并且**认得出哪边比较新**（云端那份更大 = 云端更新）。
    let local = side(&[meta("a.webp", 100, Some("2026-10-01T10:00:00Z"))]);
    let remote = side(&[meta("a.webp", 200, Some("2026-10-09T10:00:00Z"))]);
    let out = prune::diverged(&local, &remote, None);
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].0, "a.webp");
    assert_eq!(out[0].1, prune::Verdict::Differs { newer: prune::Newer::Remote },
        "云端那份更大更新：按上传会覆盖它，这是最要紧的方向");
}

/// 大小一样、只有修改时间动过 —— 也要认出来。同一张图重新导出可能恰好同尺寸，
/// 只比大小的那种实现会把它漏掉（真人照片库并不罕见）。
#[test]
fn same_size_but_newer_modtime_counts_as_diverged() {
    let local = side(&[meta("p.webp", 5_000, Some("2026-10-01T10:00:00Z"))]);
    let remote = side(&[meta("p.webp", 5_000, Some("2026-10-08T10:00:00Z"))]);
    let out = prune::diverged(&local, &remote, None);
    assert_eq!(out, vec![("p.webp".into(), prune::Verdict::Differs { newer: prune::Newer::Remote })]);
}

/// 两边完全一致：不许出现在名单里。这条守着粗筛别把噪声全放出来 ——
/// 真实图库里绝大多数文件是相同的，名单里混进一条"这个也一样"就等于没用。
#[test]
fn identical_files_stay_out_of_the_list() {
    let one = [meta("p.webp", 5_000, Some("2026-10-01T10:00:00Z"))];
    let out = prune::diverged(&side(&one), &side(&one), None);
    assert!(out.is_empty(), "相同的文件不该进名单：{out:?}");
}

/// 拿不到时间戳时**不许**判成"相同"。这是三档里 `Unknown` 那档存在的理由：
/// 大小相同、时间未知 —— 读成 Same 就是放行一次静默覆盖，读成 Differs 是无中生有。
#[test]
fn missing_modtime_is_unknown_never_same() {
    let local = side(&[meta("p.webp", 5_000, None)]);
    let remote = side(&[meta("p.webp", 5_000, Some("2026-10-08T10:00:00Z"))]);
    let out = prune::diverged(&local, &remote, None);
    assert_eq!(out, vec![("p.webp".into(), prune::Verdict::Unknown)],
        "时间未知时方向也是未知的，不许猜一个");
}

/// 时间戳完全相等、而内容不同（只能靠哈希发现）：**方向仍然给不出**，落 Unknown。
///
/// 哈希能证明"不是同一份"，证明不了"谁更新"——哈希没有次序。给一个方向就是编的，
/// 而这一份是给人当"往哪边合"的依据看的，编错方向比不判更坏。
#[test]
fn equal_modtime_with_differing_hash_reports_unknown_not_a_guess() {
    let local = side(&[meta("p.webp", 5_000, Some("2026-10-01T10:00:00Z"))]);
    let remote = side(&[meta("p.webp", 5_000, Some("2026-10-01T10:00:00Z"))]);
    let hashes = |n: &str| Some((Some(format!("{n}-L")), Some(format!("{n}-R"))));
    let out = prune::diverged(&local, &remote, Some(&hashes));
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(out[0].1, prune::Verdict::Unknown,
        "哈希说得出'不一样'，说不出'谁更新'——那就不许给方向");
}

/// 哈希确认出"拿不到"的那几种形状，**一律落 `Unknown`**，不许落到 `Same`。
///
/// 这一条钉的是 2026-10-11 修掉的那个洞：早先的 `_ => {}` 会让 `Verdict::Same` 接住
/// "两边都没问出来"，于是**问不出来**被读成**两边一样** —— 那是一次放行，正是三档
/// 设计要关掉的那扇门。早先那条哈希证人只测了 `(Some, Some)` 这一个能工作的形状，
/// 所以它照样绿。四种形状全钉在这儿。
#[test]
fn an_unobtainable_hash_is_never_reported_as_same() {
    let one = [meta("p.webp", 5_000, Some("2026-10-01T10:00:00Z"))];
    let (l, r) = (&one[0], &one[0]);
    // 两边都问不到 / 只有一边问不到 / 整体没问成 —— 四种都是"判据不足"。
    for (label, f) in [
        ("两边都拿不到", (|_: &str| Some((None, None))) as fn(&str) -> Option<(Option<String>, Option<String>)>),
        ("只拿到本地", |_: &str| Some((Some("x".into()), None))),
        ("只拿到云端", |_: &str| Some((None, Some("x".into())))),
        ("没问成", |_: &str| None),
    ] {
        assert_eq!(prune::verdict_of(l, r, Some(&f)), prune::Verdict::Unknown,
            "{label}：判据不足却报成了「两边一样」，那是放行一次静默冲掉");
    }
}

/// 两侧时间戳的归一化必须落在**同一个刻度**上（2026-10-11 修）。
///
/// 早先本地那一侧用 `DateTime::from_timestamp(secs, 0)` 把亚秒位丢干净、远端那一侧留着毫秒，
/// 于是**同一个时刻**在两边归一化成两串不同的字面量 —— 一个完全没动过的文件会被报成
/// 「云端那份更新」。这一条就是钉住"同一刻度的同一时刻归一化成同一个字符串"。
#[test]
fn the_two_sides_normalize_to_the_same_string_for_the_same_instant() {
    use rclone_deck_lib::runner::checks::normalize_stamp;
    // 本地：`SystemTime` 侧带毫秒；远端：rc 给的 RFC3339 侧带毫秒。
    let from_local = normalize_stamp(1_789_223_000, 750);
    let from_remote = normalize_stamp(1_789_223_000, 750);
    assert_eq!(from_local, from_remote, "同一时刻两侧归一化成了两串不同的字面量");

    // 真的不同的一刻才不等（否则上面那条就是个恒真的空断言）。
    assert_ne!(normalize_stamp(1_789_223_000, 750), normalize_stamp(1_789_223_001, 750));
    // 亚秒位真的要参与比较 —— 这一条是那个洞的直接判据。
    assert_ne!(normalize_stamp(1_789_223_000, 0), normalize_stamp(1_789_223_000, 750),
        "亚秒位被丢掉了：本地截到整秒、远端留毫秒，同一刻会被读成不同");
}

/// **调用点**也得钉住 —— 只测 `normalize_stamp` 这个纯函数是不够的（2026-10-11 补）。
///
/// 实测过：把 `side_meta` 里的 `d.subsec_millis()` 改回字面量 `0`，
/// 上面那条证人**照样绿** —— 因为它只喂了手写的 `(秒, 毫秒)`，压根没经过 `side_meta`。
/// 也就是说那个洞可以在证人全绿的情况下回来。这条钉住本地那一侧真的把毫秒交进去了。
#[test]
fn the_local_side_really_passes_millis_into_the_shared_normalizer() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runner/checks.rs")).unwrap();
    assert!(src.contains("normalize_stamp(d.as_secs() as i64, d.subsec_millis())"),
        "本地那一侧没把毫秒交给归一化：本地截到整秒、远端留毫秒，同一时刻会被读成不同 \
         （而纯函数那条证人抓不到这个，它只测函数本身）");
    // 远端那一侧也要走同一个函数 —— 两处各拼各的，就是那个洞的来路。
    assert!(src.contains("normalize_stamp(t.timestamp(), t.timestamp_subsec_millis())"),
        "远端那一侧没有走同一个归一化：两条路各拼各的，形状就会不一致");
}

/// 哈希那一级的本地侧**必须走 `local_fs_id`**（2026-10-11 修，同日用真 rcd 实测坐实）。
///
/// 早先 `check_pair` 发的是 `("/", "D:\\pics")` —— `fs` 传 `/`、盘符路径塞在 `remote` 里。
/// 用真 `rclone rcd` 量过，两种写法的差别是决定性的：
///
/// ```text
/// fs=/            remote=D:\...\local  => 404  error in ListJSON: directory not found
/// fs=D:/.../local  remote=(空)          => OK   2 条 [a.txt, b.txt]
/// ```
///
/// 原来那种写法**一条都读不出来**，而汇总照样打一行「哈希那一级：跑了」——一个报成功而
/// 什么都没做的开关。`local_fs_id` 的注释早记着这形状不能当读数用（F16），是这里没复用它。
#[test]
fn the_hash_level_addresses_the_local_side_the_way_the_browser_does() {
    use rclone_deck_lib::runner::browser::local_fs_id;
    // 盘符 + 绝对路径 → `D:/pics`，**不带前导斜杠**，remote 那一段留空。
    assert_eq!(local_fs_id(std::path::Path::new(r"D:\pics")).as_deref(), Some("D:/pics"));
    // 这一条钉住"不复用就会退回去"的那个形状：`/` 开头的写法实测 404。
    assert_ne!(local_fs_id(std::path::Path::new(r"D:\pics")).as_deref(), Some("/D:/pics"));
    // 没量过的形状老实交 None（交一个 rc 不认的 fs 出去比交不出来更坏）。
    assert!(local_fs_id(std::path::Path::new(r"\\srv\share\pics")).is_none(), "UNC 没量过，不许猜一个");
}

/// **调用点**也要钉（和上面那条同源的教训）：只测 `local_fs_id` 这个纯函数是不够的 ——
/// 早先那条就是纯函数全绿、`check_pair` 却发着错形状的 `/` + 盘符路径。
#[test]
fn the_hash_pair_really_goes_through_local_fs_id() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/runner/mod.rs")).unwrap();
    assert!(src.contains("local_fs_id(p)"),
        "check_pair 没有走 local_fs_id：本地那一侧的发法会退回实测 404 的那种形状");
    // 旧写法那两个字面量不许回来。
    assert!(!src.contains(r#"Cow::Borrowed("/"),"#),
        "又发回 fs=/ + remote=盘符路径 那种写法了（实测 404，汇总却会说「跑了」）");
}

/// 浏览面板那条预览路的 `fs` 反解：**必须是 `local_fs_id` 的正逆一对**。
///
/// 反解错了的后果很具体：要么整列看不了（形状对不上），要么更糟——反解出一个
/// **错的根**，于是那道"规范化后仍在根之下"的闸在一个错误的根上放行。
/// 所以这一条把正反两边的输出钉在一起，任何一边改了形状，另一边跟着红。
#[test]
fn the_pane_preview_undoes_exactly_what_local_fs_id_does() {
    use rclone_deck_lib::runner::browser::{local_fs_id, preview_pane_image};
    let orig = std::path::Path::new(r"D:\pics\2026");
    let id = local_fs_id(orig).expect("量过的形状");
    assert_eq!(id, "D:/pics/2026");
    // 反解走的是同一个函数，所以正逆必然配对；这一条钉住"不许另写一份"。
    assert!(preview_pane_image(&id, "", "a.webp").is_err(),
        "目录里没有 a.webp 时该说读不出来，而不是别的错");
    // 没量过的形状老实拒，不硬拼一个根出来。
    assert!(local_fs_id(std::path::Path::new(r"\\srv\share")).is_none());
}
#[test]
fn the_list_is_sorted_by_name() {
    let local = side(&[
        meta("z.webp", 1, Some("2026-01-01T00:00:00Z")),
        meta("a.webp", 1, Some("2026-01-01T00:00:00Z")),
        meta("m.webp", 1, Some("2026-01-01T00:00:00Z")),
    ]);
    let remote = side(&[
        meta("z.webp", 2, Some("2026-02-01T00:00:00Z")),
        meta("a.webp", 2, Some("2026-02-01T00:00:00Z")),
        meta("m.webp", 2, Some("2026-02-01T00:00:00Z")),
    ]);
    let names: Vec<String> = prune::diverged(&local, &remote, None).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, vec!["a.webp", "m.webp", "z.webp"]);
}

/// 哈希那一级（第二级）**默认不跑**，而这条钉的是那个默认值的理由与形状。
///
/// `Defaults::check_by_hash` 默认 `false`，不是因为"不信任哈希"，而是因为
/// `operations/check` 在后端不支持该哈希时要**下载每一个文件**——一个几千张图的库
/// 开着它就是分钟级等待，而第一级（大小 + 修改时间）已经抓到绝大多数。
/// 那一级必须由人显式打开；这一条钉住"旧配置文件里没有这一项时读成不开"，
/// 免得哪天把 default 改成 true，所有人的盘点都会突然变慢一个数量级。
#[test]
fn the_hash_level_is_off_unless_asked_for() {
    let text = "[defaults]\nrclone_bin = \"C:\\\\rclone\\\\rclone.exe\"\n";
    let cfg = rclone_deck_lib::config::jobs::JobsFile::parse(text).unwrap();
    assert!(!cfg.defaults.check_by_hash,
        "没写这一项却默认跑哈希：那一级可能要下载每一个文件");
    // 写了才开——这是它作为显式开关的唯一作用，不带别的副作用。
    let on = rclone_deck_lib::config::jobs::JobsFile::parse(
        "[defaults]\nrclone_bin = \"C:\\\\rclone\\\\rclone.exe\"\ncheck_by_hash = true\n").unwrap();
    assert!(on.defaults.check_by_hash, "显式写了却没生效");
}

// ───────────────── 本地预览的安全边界（2026-10-11） ─────────────────

/// `safe_relative_name` 那一道闸。这一条钉的是**具体那些形状**，不是"它会拒绝某些东西"：
/// 将来有人为了少写一次检查把它简成"看起来像路径就行"，下面每一行都会红。
#[test]
fn a_relative_name_may_not_carry_a_path_anywhere_in_it() {
    use rclone_deck_lib::runner::safe_relative_name;
    for good in ["a.webp", "img/2026/a.png", "图 1.jpg", "a.b.webp"] {
        assert!(safe_relative_name(good), "正常的图片名被拒了：{good}");
    }
    // 三样都不许有：上跳、绝对、盘符/UNC、外加空串与 NUL。
    for bad in [
        "..", "../secrets.txt", "a/../../b.webp", "/etc/passwd",
        r"C:\Windows\win.ini", "a\\..\\b", "", "a\0b",
    ] {
        assert!(!safe_relative_name(bad), "这一格被放行了：{bad:?}");
    }
}

/// 扩展名是**白名单**。这一条钉的是"只有图片能读"，而不是"不在黑名单里就能读"——
/// 后者少一条就要更新一次，而漏一个 `exe` 就是一次任意文件读取。
#[test]
fn only_image_extensions_are_previewable() {
    use rclone_deck_lib::runner::IMAGE_EXTS;
    for good in ["a.png", "a.JPG", "a.jpeg", "a.webp", "a.gif", "a.avif"] {
        let ext = format!("x.{good}");
        assert!(IMAGE_EXTS.iter().any(|e| ext.to_ascii_lowercase().ends_with(e)),
            "正常的图片扩展名不在白名单里：{good}");
    }
    for bad in ["a.exe", "a.txt", "a.toml", "a.zip", "noext", "a.png.exe"] {
        assert!(!IMAGE_EXTS.iter().any(|e| bad.to_ascii_lowercase().ends_with(e)),
            "这一类被当成图片了：{bad}");
    }
}

/// base64 与标准实现逐字节一致 —— 自己写的编码器，**必须**有一个对照。
/// 三个长度档（补 2 档 + 不补 1 档）都过一遍，少测一档就正好漏掉那档的补位。
#[test]
fn the_hand_rolled_base64_matches_the_standard_vectors() {
    use rclone_deck_lib::runner::base64_encode;
    // RFC 4648 §10 的官方测试向量
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    // 二进制档（0x00 / 0xFF 那一族，最容易在移位时错）
    assert_eq!(base64_encode(&[0x00, 0xff, 0xfe]), "AP/+");
}
