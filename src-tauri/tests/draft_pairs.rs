//! 临时配对卡（spec `2026-10-05-rclone-deck-adhoc-pair-card-design.md`，裁定 R93–R99）。
//!
//! 这个二进制里有两类证人：读源文本的**形状**证人（D1 那一步），和真对着一台后台夹具跑的
//! **行为**证人（D2 那一步）。分开放是因为它们要防的是两件事：形状证人防"以后有人把临时任务
//! 塞回那条不动的路"，行为证人防"临时格绕开了删除的那三道闸"。

#![cfg(windows)]

pub mod localdaemon;
mod common;

use common::{count_in, read, strip_comments};

/// **R93**：查表必须交出**具体的** `Job`，配置与临时任务分开存，`jobs()` 交合成的一份。
/// 钉形状不钉行为：把 drafts 塞回那个 `Vec<Job>` 在行为上完全看不出来（它要改 `Runner::new`
/// 或整表加锁），而那条路一旦走通，"临时任务"就变成"没写进文件的配置"——下一次读文件就散架。
#[test]
fn the_lookup_hands_out_an_owned_job() {
    let t = strip_comments(&read("src-tauri/src/runner/mod.rs"));
    assert!(t.contains("fn lookup(&self, id: &str) -> Result<Job, RunError>"),
        "查表还在交引用：那条不动的 `Vec` 装不下内存里的临时任务（R93）");
    assert!(!t.contains("Result<&Job, RunError>"), "交引用的那一条又回来了");
    assert!(t.contains("drafts"), "没有 drafts 那张表：临时任务无处可存");
    assert!(t.contains("pub fn jobs(&self) -> Vec<Job>"),
        "jobs() 还在交切片：合成不出「配置 + 临时」那一份，`get_jobs` 也就看不见临时卡");
}

/// **R93 的另一半**：临时格与配置格必须是**同一套机器**跑出来的——同一把锁（按 `Scope`）、
/// 同一份日志、同一个预览登记处。判据落在"没有第二条通路"上：不许出现 `run_draft` /
/// `preview_draft_mirror` 这种平行实现，也不许 drafts 自己拿一份 `PreviewRegistry`。
#[test]
fn a_draft_runs_through_the_same_machinery_as_a_configured_job() {
    let t = strip_comments(&read("src-tauri/src/runner/mod.rs"));
    for banned in ["run_draft", "preview_draft", "drop_draft_guard", "draft_mutex", "draft_previews"] {
        assert!(!t.contains(banned), "冒出一条只给临时任务用的平行通路（{banned}）：{t}");
    }
    // 锚点认的是**字段声明**而不是 `drafts` 这个子串：`pub mod drafts;` 也含那五个字，
    // 拿它当锚点的话，加一行模块声明就能让这条证人红在一份永远不会跑的文本上。
    let drafts_at = t.find("drafts: StdMutex").expect("找不到 drafts 那张表");
    let body = &t[drafts_at..drafts_at + 240];
    assert!(body.contains("Vec<Job>"), "drafts 存的不是 `Job`：那它就不是\"内存里的任务\"，而是另一套东西：{body}");
}

// ============================================================ 行为证人（对着真后台夹具跑）

use std::path::PathBuf;

use localdaemon::*;
use rclone_deck_lib::config::jobs::{Job, JobKind};
use rclone_deck_lib::runner::{Runner, TaskState};
use rclone_deck_lib::pair::{pair_key_of, pair_slot_of};

const KEEP: &[&str] = &["k1.txt", "k2.txt", "k3.txt"];

/// 云端那一侧比本地多一个文件：`sync` 以本地为主时，那个就是唯一的删除候选。
/// 三个共享 + 一个孤儿（1/4）够不到"一次砍掉一半"那条线（`prune::RATIO_DEN`），
/// 否则预览自己就把这一档拒了，测不到凭证那一步。
fn with_orphan() -> Vec<&'static str> {
    KEEP.iter().copied().chain(std::iter::once("orphan.txt")).collect()
}

/// `files_in` 交的是 `Vec<String>`：断言那份目录内容时要按它的形状比，不许各写一遍映射。
fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// 一对**没配过**的位置：本地目录 ↔ 具名 remote（alias 指向 `fx.dst()`，见 localdaemon 那段）。
/// `mk` 拿那两侧的写法建配置任务——写配置侧用例要按方向摆 src/dst，而哪一侧是 `a` 由后台判，
/// 测试不该自己猜（这正是 R94 把占格判据交给 `pair_key_of`/`pair_slot_of` 的理由）。
async fn pair(
    sub: &str, local_names: &[&str], cloud_names: &[&str],
    mk: impl FnOnce(&str, &str) -> Vec<Job>,
) -> (Fx, Runner, DaemonGuard, String, String) {
    let fx = Fx::new();
    let local_dir: PathBuf = if sub.is_empty() { fx.src() } else { fx.src().join(sub) };
    let cloud_dir: PathBuf = if sub.is_empty() { fx.dst() } else { fx.dst().join(sub) };
    std::fs::create_dir_all(&local_dir).unwrap();
    std::fs::create_dir_all(&cloud_dir).unwrap();
    seed(&local_dir, local_names);
    seed(&cloud_dir, cloud_names);
    let (local, cloud) = (fwd(&local_dir), fx.remote(sub));
    let (r, g) = setup_with_conf(fx.root(), Some(fx.conf()), mk(&local, &cloud)).await;
    (fx, r, g, local, cloud)
}

/// 界面上那张卡看得见的所有编号（配置的 + 临时的）。交**owned** 的串：`jobs()` 每次合成一份新的
/// `Vec<Job>`，返回它的引用就是引用一个临时值。
fn ids_of(r: &Runner) -> Vec<String> {
    r.jobs().iter().map(|j| j.id.clone()).collect()
}

/// 那张卡上"哪一格站着哪一条"：`"模式:槽位=编号"`，排好序。
/// 格子问的是后台那两枚判据（`kind` 与 `pair_slot_of`），跟界面读的是同一把尺子；
/// 用它是为了看得见**没建出来的那一格**——清单里少一行就是缺档，而那正是"框里不许给出
/// 没有任务的选项"这条判据的形状（原来由 `state::pair_views` 那四个 `Option` 担保，产物已删）。
fn cells_of(r: &Runner) -> Vec<String> {
    let mut v: Vec<_> = r.jobs().iter()
        .map(|j| format!("{}:{}={}", j.kind.toml_word(), pair_slot_of(&j.src, &j.dst), j.id))
        .collect();
    v.sort();
    v
}

/// 这一趟立起来的卡是一张还是几张（配置的格与临时的格混在同一对上时必须是 1）。
fn card_count(r: &Runner) -> usize {
    let mut keys = std::collections::BTreeSet::new();
    for j in r.jobs() { keys.insert(pair_key_of(&j.src, &j.dst)); }
    keys.len()
}

/// 六句拒绝话术的**唯一**出处（与对齐那几条同一个理由：长在流程中间的句子里，
/// 测试构造不到那个位置，扫不到就等于这几句从没被"不许漏内部名词"那条规矩管过）。
/// 这里先把它们点名读一遍，钉的是"这几句说得出人话"。**这一条不钉调用点** —— 那六个函数被
/// `pair_draft` / `drop_draft` 用着，是靠那几条端到端用例（拿到那句拒绝才会红）间接钉住的。
#[test]
fn the_draft_refusals_never_leak_internal_names() {
    use rclone_deck_lib::runner::drafts::{
        draft_absent_refusal, draft_all_taken_refusal, draft_running_refusal,
        draft_same_position_refusal, draft_unconfirmed_refusal, draft_unknown_endpoint_refusal,
    };
    // 内部名词名单与 `tests/commands_surface.rs` 那份同族，但**少一项** `run_job`：这一面发不出
    // 那条入口的句子，抄全了反而会把"这里没提到 run_job"钉成应有行为。两份要合流就先合话术。
    const BANNED: &[&str] = &["Sync", "Copy", "Gate", "JobKind", "run_copy", "run_mirror", "preview_mirror"];
    let said = [
        draft_same_position_refusal(),
        draft_all_taken_refusal(),
        draft_unknown_endpoint_refusal(),
        draft_running_refusal(),
        draft_unconfirmed_refusal(),
        draft_absent_refusal(),
    ];
    for s in &said {
        for b in BANNED {
            assert!(!s.contains(b), "临时卡的拒绝话术漏了内部名词 {b}：{s}");
        }
    }
    // 句子说的是"这张卡"，不是"这个 job"：操作员读得懂的那一档（spec §5）。
    for s in &said {
        assert!(s.contains("卡") || s.contains("位置") || s.contains("这一侧"),
            "那句没指着界面上看得见的东西：{s}");
    }
    assert!(draft_all_taken_refusal().contains("每一格都已经有任务"));
    assert!(draft_same_position_refusal().contains("同一个位置"));
    assert!(draft_running_refusal().contains("正有事在跑"));
}

/// **位置判据只有一份**：配置文件解析与临时卡都得过同一段代码。
/// 两处各写一遍的话，少写的那一处就是"面板跑得成一条配置侧会拒绝的任务"——
/// 而那一族任务恰恰是删除范围算错的那一族（反斜杠、点段、两侧同型）。
#[test]
fn the_draft_and_the_config_check_positions_with_one_code() {
    let cfg = strip_comments(&read("src-tauri/src/config/jobs.rs"));
    let dr = strip_comments(&read("src-tauri/src/runner/drafts.rs"));
    assert_eq!(count_in(&cfg, "pub fn check_positions"), 1, "位置判据不是只有一份定义：{cfg}");
    assert!(cfg.contains("check_positions()"), "解析器没走那一处：{cfg}");
    assert!(dr.contains("check_positions()"), "临时卡没走那一处：{dr}");
    assert!(!dr.contains("is_remote() =="), "临时卡自己写了一份「什么算合法位置」（R94 要复用配置那条判据）");
    // 编号只增不减：id 一旦被回收，那张卡留下的旧凭证就可能在新卡上"作数"（R34 那一族 ABA）。
    assert!(dr.contains("draft_seq"), "编号没有只增的账：{dr}");
}

#[tokio::test]
async fn a_draft_shows_up_in_jobs_and_lookup() {
    let (_fx, r, _g, local, cloud) = pair("pair", KEEP, &[], |_, _| vec![]).await;
    let key = r.pair_draft(&local, &cloud).unwrap();
    assert_eq!(key, pair_key_of(&local, &cloud), "返回的键要跟界面上那张卡同一把尺子算出来");
    assert_eq!(ids_of(&r), ["adhoc-1-copy-a", "adhoc-1-copy-b", "adhoc-1-sync-a", "adhoc-1-sync-b"]);
    let j = r.job("adhoc-1-copy-a").expect("临时格查不到：lookup 还只认配置那张表");
    assert_eq!(j.name, "临时（没存进配置）");
    assert!(j.open_dir.is_none() && j.manifest.is_none(), "临时格不该带出对账与打开目录那两档");
    // a 侧是本地那一侧（恰好一侧本地），所以 copy-a 与 sync-a 的 src 都是本地
    assert_eq!(r.job("adhoc-1-copy-a").unwrap().src, local);
    assert_eq!(r.job("adhoc-1-sync-a").unwrap().src, local);
    assert_eq!(r.job("adhoc-1-copy-b").unwrap().src, cloud);
    assert_eq!(r.job("adhoc-1-sync-b").unwrap().dst, local);
}

/// **R94**：只补空缺的格子。配置里已经有「从本地补到云端」那一格时，那一格必须**还是配置那条**——
/// 建出第二条就是影子：界面上看不见的那条才是真会跑的那条。
#[tokio::test]
async fn a_draft_fills_only_the_missing_cells() {
    let (_fx, r, _g, local, cloud) = pair("pair", KEEP, &[],
        |l, c| vec![job_at("cfg-up", JobKind::Copy, l, c)]).await;
    r.pair_draft(&local, &cloud).unwrap();
    assert_eq!(card_count(&r), 1, "临时格把一对位置裂成了两张卡：{:?}", cells_of(&r));
    // 那一格**还是配置那条**：临时的顶掉它就成了影子——界面上看不见的那条才是真会跑的。
    assert_eq!(cells_of(&r), [
        "copy:a=cfg-up", "copy:b=adhoc-1-copy-b",
        "sync:a=adhoc-1-sync-a", "sync:b=adhoc-1-sync-b",
    ], "临时格只该补空缺的那三格");
}

/// 四格都被占住时不建任何一条，并且那句话要说得出"不用再立一张"。
#[tokio::test]
async fn a_pair_with_every_cell_taken_is_refused_and_says_so() {
    let (_fx, r, _g, local, cloud) = pair("pair", KEEP, &[], |l, c| vec![
        job_at("c1", JobKind::Copy, l, c), job_at("c2", JobKind::Copy, c, l),
        job_at("s1", JobKind::Sync, l, c), job_at("s2", JobKind::Sync, c, l),
    ]).await;
    let err = r.pair_draft(&local, &cloud).unwrap_err().to_string();
    assert!(err.contains("每一格都已经有任务"), "那句要说的是这一对已经齐了：{err}");
    assert_eq!(ids_of(&r), ["c1", "c2", "s1", "s2"], "被拒的那一次还是建出了格");
}

/// 两处**规范化之后**是同一棵树 ⇒ 拒。写法不同也照样拒：比的从来不是字符串。
#[tokio::test]
async fn the_same_position_on_both_sides_is_refused() {
    let (fx, r, _g, local, _cloud) = pair("", KEEP, &[], |_, _| vec![]).await;
    // 同一目录的另一种写法（反斜杠 + 结尾斜杠 + 大小写换一档）：`Scope` 三种归一都在这一发里
    let mut other = fx.src().display().to_string();
    other.push('\\');
    if let Some(pos) = other.find('C') {
        other.replace_range(pos..pos + 1, "c");
    }
    let err = r.pair_draft(&local, &other).unwrap_err().to_string();
    assert!(err.contains("同一个位置"), "同一棵树两种写法没被认出来：{err}");
    assert!(r.pair_draft(&local, &local).unwrap_err().to_string().contains("同一个位置"));
    assert!(r.jobs().is_empty(), "被拒的那一次建出了格");
}

/// 两侧都本地 ⇒ 只有「只补，不删」两格，`sync` 一格都不建（没有权威的一端就没有删除范围）。
/// 这一档**不是拒绝**：卡照样立起来，只是「要做什么」那枚框里只剩「只补，不删」那一格。
#[tokio::test]
async fn two_local_directories_get_no_deleting_cell() {
    let (fx, r, _g, local, _cloud) = pair("", KEEP, &[], |_, _| vec![]).await;
    let other = fx.root().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let second = fwd(&other);
    r.pair_draft(&local, &second).unwrap();
    assert_eq!(ids_of(&r), ["adhoc-1-copy-a", "adhoc-1-copy-b"]);
    assert_eq!(card_count(&r), 1, "两侧都本地的一对裂成了两张卡：{:?}", cells_of(&r));
    // 清单里只有「只补，不删」那一档的两格：没有权威的一端就没有删除范围，
    // 那一档**一格都不建**（不是猜一个当权威）。缺格看得见，靠的是这里摆的是整张清单。
    assert_eq!(cells_of(&r), ["copy:a=adhoc-1-copy-a", "copy:b=adhoc-1-copy-b"],
        "两侧都本地却建出了「会删除」那一格");
}

/// **R93 的实际效果**：临时格与配置格走的是同一条传输通路——这里真跑一次上传。
#[tokio::test]
async fn a_draft_copy_cell_actually_transfers() {
    let (fx, r, _g, local, cloud) = pair("pair", KEEP, &[], |_, _| vec![]).await;
    r.pair_draft(&local, &cloud).unwrap();
    let st = r.run_copy("adhoc-1-copy-a", false).await.unwrap();
    assert!(matches!(st, TaskState::Done { .. }), "临时格跑完不是 Done：{st:?}");
    assert_eq!(files_in(&fx.dst().join("pair")), names(KEEP));
    // 试跑那一档照旧放行（copy 本来就允许），并且一个东西都不落
    let st = r.run_copy("adhoc-1-copy-b", true).await.unwrap();
    assert!(matches!(st, TaskState::Done { dry_run: true, .. }));
}

/// **R95**：临时卡允许「会删除」那一档，但一步不少——没凭证就拒，看完清单才删，
/// 并且那个预览数作为数据带到终态。
#[tokio::test]
async fn a_draft_mirror_cell_deletes_only_with_a_token() {
    let (fx, r, _g, local, cloud) =
        pair("m", KEEP, &with_orphan(), |_, _| vec![]).await;
    r.pair_draft(&local, &cloud).unwrap();
    let err = r.run_mirror("adhoc-1-sync-a", "prev-00000000000000000000000000000000")
        .await.unwrap_err().to_string();
    assert!(err.contains("确认"), "没有凭证那一句要说缺的是确认：{err}");
    assert!(files_in(&fx.dst().join("m")).contains(&"orphan.txt".to_string()),
        "被拒的那一次还是删了东西");

    let p = r.preview_mirror("adhoc-1-sync-a").await.unwrap();
    assert_eq!(p.doomed, ["orphan.txt".to_string()]);
    let st = r.run_mirror("adhoc-1-sync-a", &p.token).await.unwrap();
    match st {
        TaskState::Done { stats, previewed, .. } => {
            assert_eq!(stats.deletes, 1);
            assert_eq!(previewed, Some(1), "预览数要作为数据带进终态（不是 0、不是 None）");
        }
        other => panic!("镜像跑完不是 Done：{other:?}"),
    }
    assert_eq!(files_in(&fx.dst().join("m")), names(KEEP));
}

/// **R95 的另一半**：凭证认任务，临时格的那份不能拿去授权配置任务（同一张卡上的另一格也一样）。
#[tokio::test]
async fn a_draft_token_does_not_authorize_a_configured_job() {
    let (fx, r, _g, local, cloud) =
        pair("m", KEEP, &with_orphan(),
            |l, c| vec![job_at("cfg-down", JobKind::Sync, c, l)]).await;
    r.pair_draft(&local, &cloud).unwrap();
    let p = r.preview_mirror("adhoc-1-sync-a").await.unwrap();
    let err = r.run_mirror("cfg-down", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("不作数"), "越权的那一句要说凭证不成立：{err}");
    assert_eq!(files_in(&fx.dst().join("m")), names(&with_orphan()),
        "临时格的凭证在配置任务上动了手");
    // 本地那一侧 = 夹具里的 `src/m`（`pair("m", …)` 两侧同时下潜一层，两端才同坐标系）
    assert_eq!(files_in(&fx.src().join("m")), names(KEEP), "配置任务被凭证推着删了本地");
}

/// 反方向同样要拒（两处各一篇：这条判据的两个方向坏起来是两件事）。
#[tokio::test]
async fn a_configured_token_does_not_authorize_a_draft() {
    let (fx, r, _g, local, cloud) =
        pair("m", KEEP, &with_orphan(),
            |l, c| vec![job_at("cfg-up", JobKind::Sync, l, c)]).await;
    r.pair_draft(&local, &cloud).unwrap();
    let p = r.preview_mirror("cfg-up").await.unwrap();
    let err = r.run_mirror("adhoc-1-sync-b", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("不作数"), "{err}");
    assert_eq!(files_in(&fx.dst().join("m")), names(&with_orphan()));
    assert_eq!(files_in(&fx.src().join("m")), names(KEEP), "配置任务的凭证推着临时格删了本地");
}

/// **R98**：正在跑（以及"跑完但没确证"）的时候收不起来。
/// 时序不靠抢：先在有 `Running` 状态的那一段里问一次，再杀掉后台把它推进 `Unknown` 那一档
/// （与 `tests/runner_local.rs` 那几条同一套夹具，理由写在那份文件顶部）。
#[tokio::test]
async fn a_running_draft_cell_cannot_be_folded_away() {
    let (fx, r, g, local, cloud) = pair("pair", KEEP, &[], |_, _| vec![]).await;
    let key = r.pair_draft(&local, &cloud).unwrap();
    let (res, ()) = tokio::join!(
        r.run_copy("adhoc-1-copy-a", false),
        async {
            wait_until_running(&r, "adhoc-1-copy-a").await;
            let err = r.drop_draft(&key).await.unwrap_err().to_string();
            assert!(err.contains("正有事在跑"), "跑着的时候把卡收了：{err}");
            g.kill_daemon();
        }
    );
    // 状态未知是一条 **Ok** 的路（job 没答上来不等于这次调用失败），那一档也要拒。
    match res.unwrap() {
        TaskState::Unknown { .. } => {}
        other => panic!("杀掉后台之后该走 Unknown，实际 {other:?}"),
    }
    let err = r.drop_draft(&key).await.unwrap_err().to_string();
    assert!(err.contains("没确证"), "状态不明那一档也要拒，并且要照实说：{err}");
    assert_eq!(r.jobs().len(), 4, "被拒的那一次还是把卡收了");
    assert!(matches!(r.state("adhoc-1-copy-b").await, TaskState::Idle), "同一张卡的其他格跟着一起被拒了");
    // 那一趟写过的东西仍然在（收卡被拒不该打断它）：只在本地侧留一个文件的话目标侧就得有它
    assert!(files_in(&fx.dst().join("pair")).len() >= 1, "被拒的收卡顺手把在跑的那趟也弄丢了");
}

/// **R99**：编号与配置里的撞了就进位。
#[tokio::test]
async fn draft_ids_do_not_collide_with_configured_ones() {
    let (_fx, r, _g, local, cloud) = pair("pair", KEEP, &[], |l, c| vec![
        // 配置里真有一条长得像临时格编号的任务。它站在**另一对**位置上（`{l}2` / `{c}2`），
        // 所以本对的四格全缺、四格都要建 —— 但 1 号已经被占了，只能进位。
        // R99 说的就是这一档：配置侧只查重、不限字符集，所以"造一个不可能撞的前缀"不成立。
        job_at("adhoc-1-copy-a", JobKind::Copy, &format!("{l}2"), &format!("{c}2")),
    ]).await;
    r.pair_draft(&local, &cloud).unwrap();
    assert_eq!(ids_of(&r), [
        "adhoc-1-copy-a",                 // 配置里那一条，一个字节没动
        "adhoc-2-copy-a", "adhoc-2-copy-b", "adhoc-2-sync-a", "adhoc-2-sync-b",
    ]);
}

/// 删掉的卡上，那份旧凭证彻底没牙；而**下一张卡拿不到回收的编号**（R99 不回收的实际效果）。
/// 编号回收是 ABA 的入口：凭证的归属按 id 认，id 一旦重新发出去，陈年那份就替新卡那一格作数
/// （R34 那一族）。
#[tokio::test]
async fn a_folded_draft_leaves_a_stale_token_powerless() {
    let (fx, r, _g, local, cloud) =
        pair("m", KEEP, &with_orphan(), |_, _| vec![]).await;
    let key = r.pair_draft(&local, &cloud).unwrap();
    let p = r.preview_mirror("adhoc-1-sync-a").await.unwrap();
    // 只预览过、没在跑 ⇒ 收得起来（R98 拒的是"有事在跑"，不是"看过清单"）
    r.drop_draft(&key).await.unwrap();
    assert!(r.job("adhoc-1-sync-a").is_none(), "卡没删掉");
    assert!(r.jobs().is_empty(), "收一张卡把别的卡上的格子一起删了");
    let err = r.run_mirror("adhoc-1-sync-a", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("没有叫"), "旧凭证还找得到那条已销毁的任务：{err}");

    let other = fx.root().join("second");
    std::fs::create_dir_all(&other).unwrap();
    r.pair_draft(&fwd(&other), &fx.remote("second")).unwrap();
    assert_eq!(ids_of(&r), ["adhoc-2-copy-a", "adhoc-2-copy-b", "adhoc-2-sync-a", "adhoc-2-sync-b"],
        "编号被回收了：那张卡的旧凭证就落在这一张上");
}

/// **R95 的锁与作废**：一次写在临时格与配置格之间互相作废凭证——读的是同一个 `Scope` 坐标系。
/// 这里配置侧那份清单先站着，临时格真写一次目标侧，那份清单当场不作数。
#[tokio::test]
async fn a_draft_write_voids_a_configured_preview() {
    let (_fx, r, _g, local, cloud) = pair("m", KEEP, KEEP,
        |l, c| vec![job_at("cfg-down", JobKind::Sync, c, l)]).await;
    r.pair_draft(&local, &cloud).unwrap();
    let p = r.preview_mirror("cfg-down").await.unwrap();
    assert_eq!(p.doomed.len(), 0, "夹具形状不对：两侧一样时不该有删除候选");
    // 临时格真跑一次写（copy-a：本地 → 云端）。它传不传东西不重要——写动作本身就作废凭证。
    r.run_copy("adhoc-1-copy-a", false).await.unwrap();
    let err = r.run_mirror("cfg-down", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("不作数"), "临时格写过的东西没作废配置侧那份凭证：{err}");
    assert!(!err.contains("清单变了"), "那句该是凭证作废，不是清单比对：{err}");
}

/// **R69 同一条口径落在临时卡上**：这一张是不是临时的，由后台判、作为数据走出去。
/// 界面上那颗「存成常用」与那句"重开就没了"都读这一枚；前端若自己按编号前缀猜，
/// 漂开的那一次就是把一条真配置项当成临时卡收掉了（或反过来）。
#[tokio::test]
async fn the_backend_marks_only_the_draft_cells() {
    let (_fx, r, _g, local, cloud) = pair("pair", KEEP, &[],
        |l, c| vec![job_at("cfg-up", JobKind::Copy, l, c)]).await;
    let key = r.pair_draft(&local, &cloud).unwrap();
    assert!(r.is_draft("adhoc-1-copy-b"), "临时格没被标出来：后台交不出这一判");
    assert!(r.is_draft("adhoc-1-sync-a"));
    assert!(!r.is_draft("cfg-up"), "配置里那一条也被标成临时的了");
    r.drop_draft(&key).await.unwrap();
    assert!(!r.is_draft("adhoc-1-copy-b"), "已经不在的编号还被认成临时格");
}

/// 认不出端点的那一侧（空 / 只有空格）拒，并且句子说得出该怎么办。
#[tokio::test]
async fn an_unrecognized_position_is_refused_and_says_what_to_write() {
    let (_fx, r, _g, local, cloud) = pair("pair", KEEP, &[], |_, _| vec![]).await;
    let err = r.pair_draft("   ", &cloud).unwrap_err().to_string();
    assert!(err.contains("认不出"), "{err}");
    assert!(err.contains("名称:路径"), "那句要给出能用的写法：{err}");
    assert!(r.pair_draft(&local, "").unwrap_err().to_string().contains("认不出"));
    assert!(r.jobs().is_empty(), "被拒的那一次建出了格");
}

/// 云端那一侧写成反斜杠 ⇒ 拒（配置侧本来就会拒；临时卡不许绕过它，
/// 因为那种写法的删除范围会静默放宽到整个桶）。
#[tokio::test]
async fn a_cloud_side_written_with_backslashes_is_refused() {
    let (fx, r, _g, local, _cloud) = pair("pair", KEEP, &[], |_, _| vec![]).await;
    let bad = format!("decklocal:pair\\{}", fx.root().file_name().unwrap().to_string_lossy());
    let err = r.pair_draft(&local, &bad).unwrap_err().to_string();
    assert!(err.contains("反斜杠"), "那句要说出是反斜杠的问题：{err}");
    assert!(r.jobs().is_empty(), "被拒的那一次建出了格：{:?}", ids_of(&r));
}
