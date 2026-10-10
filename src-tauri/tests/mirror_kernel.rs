//! 有方向的集合差（spec §4.1 第 1 步、§4.2）。这一层的价值全在**方向**上：
//! `dst \ src` 是"这次对齐会删掉什么"，`src \ dst` 是"还没上传的是什么"。
//! 只测一侧等于没测方向——对齐与盘点共用同一个函数，测不到就会一起错。

use rclone_deck_lib::prune::{orphans, plan, RATIO_DEN};
use std::collections::BTreeSet;

fn set(v: &[&str]) -> BTreeSet<String> { v.iter().map(|s| s.to_string()).collect() }

#[test]
fn the_doomed_set_is_dst_minus_src_in_a_stable_order() {
    // 夹具形状是被比例守卫决定的，不是随手取的：这条要的是"正常形状不触发任何拒绝"，
    // 而守卫的判据是 `doomed × RATIO_DEN >= dst_count` 就拦，所以要它不触发就得满足反向的不等式
    // `doomed × RATIO_DEN < dst_count`（判据本身见 `RATIO_DEN` 的注释）。这里取"共享 3 个 + 会删
    // 2 个"：2×2 = 4 < 5，够不到线，又留得下真孤儿——两头都要，不然这条就成了空转的证人。
    let src = set(&["a.webp", "s1.webp", "s2.webp", "s3.webp"]);
    let dst = set(&["s1.webp", "s2.webp", "s3.webp", "stale.webp", "gone.webp"]);
    let p = plan(&src, &dst);
    assert_eq!(p.doomed, vec!["gone.webp", "stale.webp"], "会删的必须是 dst 独有的，且顺序稳定");
    assert_eq!((p.src_count, p.dst_count), (4, 5));
    assert!(!p.empty_src && !p.too_much, "正常形状不该触发任何拒绝：{p:?}");
}

#[test]
fn turning_the_sides_around_gives_the_pending_upload_list() {
    // §4.2 的另一半不是新功能，是同一个差的另一侧。
    let local = set(&["a.webp", "only-local.webp"]);
    let remote = set(&["a.webp"]);
    assert_eq!(plan(&remote, &local).doomed, vec!["only-local.webp"]);
    assert_eq!(plan(&local, &remote).doomed, Vec::<String>::new(), "本地是超集时云端没有要删的");
}

#[test]
fn an_empty_src_refuses_rather_than_offering_the_whole_destination() {
    // R29 推广到两个方向：src 全空而 dst 非空时 doomed == dst。
    let p = plan(&set(&[]), &set(&["a.webp"]));
    assert!(p.empty_src);
    assert_eq!(p.doomed, vec!["a.webp"], "内核不改写集合差，只把诊断带出去（口径与 R29 一致）");
    // 两边都空：确实没有东西，不是基准缺失（`orphans` 的这条老口径不许动）。
    assert!(!plan(&set(&[]), &set(&[])).empty_src);
    // 反过来那一档同理：dst 空而 src 非空，是"要传东西"，不是危险。
    let q = plan(&set(&["a.webp"]), &set(&[]));
    assert!(!q.empty_src && q.doomed.is_empty());
}

#[test]
fn the_ratio_guard_trips_at_exactly_half_of_the_destination() {
    let dst = set(&["d1", "d2", "d3", "d4"]);
    assert!(!plan(&dst, &dst).too_much, "doomed=0 永远不拦");
    assert!(!plan(&set(&["d1", "d2", "d3"]), &dst).too_much, "1/4 不拦");
    assert!(plan(&set(&["d1", "d2"]), &dst).too_much, "正好一半就该拦（spec 写的是「达到一半」）");
    assert!(plan(&set(&[]), &dst).too_much, "全部 doomed 当然拦（这一档还有 empty_src 同时为真）");
}

#[test]
fn an_empty_destination_is_a_first_upload_not_a_danger() {
    // 守卫里 `!dst.is_empty()` 那一半只为这一档存在：dst 空时 doomed 必是空集，可整数算术下
    // `0 × RATIO_DEN >= 0` 恰好**成立**，不先排掉它，守卫就会把"目标端一个对象都没有"读成
    // "整个目标端都要删"。而这一档其实一个文件都不会删，它是对齐的**起点**：第一次上传，
    // 或 download 方向刚落到的那个空目录。被永久拦下，就永远传不成第一批。
    let q = plan(&set(&["a.webp", "b.webp"]), &set(&[]));
    assert!(!q.too_much, "空目标端 = 没有东西会被删，这是第一次上传的形状，不是危险：{q:?}");
    // 两边都空同理：候选集还是空的，拦它等于拒绝"无事可做"。
    let z = plan(&set(&[]), &set(&[]));
    assert!(!z.too_much, "两侧都空时 doomed 也是空集，同样不该拦：{z:?}");
}

#[test]
fn a_short_destination_is_not_an_empty_one() {
    // dst 只有一个对象、doomed 就是那一个：比例 1/1 = 100%，拦。
    // 这条存在的意义是把"分母为零"和"分母很小"分开——前者不能拦（见
    // `an_empty_destination_is_a_first_upload_not_a_danger`），后者必须拦。
    let p = plan(&set(&["other"]), &set(&["x"]));
    assert!(p.too_much);
    assert!(!p.empty_src);
}

#[test]
fn orphans_delegates_so_the_existing_inventory_witnesses_keep_their_meaning() {
    let remote = set(&["p/a.webp", "p/b.webp"]);
    let local = set(&["p/a.webp"]);
    let o = orphans(&local, &remote);
    assert_eq!(o.orphans, vec!["p/b.webp"]);
    assert!(!o.empty_basis);
    assert_eq!((o.local_count, o.remote_count), (1, 2));
    assert!(orphans(&set(&[]), &remote).empty_basis, "换实现不许把 R56 那一档改味");
    assert!(!orphans(&local, &set(&[])).empty_basis);
}

#[test]
fn the_ratio_denominator_is_declared_not_measured() {
    // 这是**假设值**（真桶从没量过）。证人钉住数字只是为了它被改动时有人出来问一句。
    assert_eq!(RATIO_DEN, 2);
}

/// `pending` 是"这一趟会补上的那些"。它与反方向的 `doomed` 是同一个数，而确认层那颗红色的
/// 按钮要说"删这 N 个，同时补上那 M 个"就靠它 —— 那个数不许由界面拿两个计数去减（DESIGN.md
/// 那条"不在 TS 里比较两个数"）。
#[test]
fn the_plan_carries_the_other_side_as_a_count() {
    let src = set(&["a.webp", "only-src.webp"]);
    let dst = set(&["a.webp", "stale.webp", "other.webp"]);
    let p = plan(&src, &dst);
    assert_eq!(p.doomed, vec!["other.webp", "stale.webp"]);
    assert_eq!(p.pending, 1, "src 有 dst 没有的那一个，就是这一趟要补上的");
    // 同一对目录反过来：两个数互换。这一半挡住"把 pending 写成 doomed 的补集"那种顺手错。
    let r = plan(&dst, &src);
    assert_eq!((r.pending, r.doomed.len()), (2, 1), "方向反过来时两个数必须换位");
}
