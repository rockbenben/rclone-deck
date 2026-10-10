//! 对齐的预览与两道拒绝（spec §4.1）。这里每条都在问同一件事：
//! "给你看的那批"和"真会删的那批"是不是同一批。全本地对本地，零云端。

#![cfg(windows)]

pub mod localdaemon;
use localdaemon::*;

use rclone_deck_lib::config::jobs::JobKind;
use rclone_deck_lib::prune::RATIO_DEN;
use rclone_deck_lib::runner::{RunError, TaskState};

/// **夹具形状是被两道拒绝决定的，不是随手取的**：预览要过守卫就得
/// `doomed × RATIO_DEN < dst_count`。小夹具（1 个孤儿配 2 个目标端文件）正好踩在"一半"这条线上，
/// 会被比例守卫先拒——那不是 bug，是 50% 这一档的必然形状。所以下面统一用
/// **3 个共享 + 2 个孤儿**（doomed 2、dst 5 → 4 < 5 ✓）这类形状，要更多孤儿时把共享数一起加上去。
/// **预览之后还要改动对端、并且期望那次改动被逐条比对拒掉**的用例，这条不等式必须在
/// **改动前后两个状态**都成立：复核那两道拒绝排在比对之前（spec §4.1），改动一旦把占比推过线，
/// 拿到的就是占比那句而不是"清单变了"。形状取 `K5` + 一个孤儿：改动前 doomed 1 / dst 6，
/// 加一个文件后 2 / 7（`2×2=4 < 7` ✓），比例守卫全程安静。
/// 真库是几千比几，够不到这条线；够不到的那一档由确认层管。
const SHARED: &[&str] = &["s1.webp", "s2.webp", "s3.webp"];
const ORPHANS: &[&str] = &["gone.webp", "stale.webp"];

/// 逐条比对那几条用例的底座：5 个共享 + 1 个孤儿，改完仍然够不到"一半"那条线（见上面那段）。
const K5: &[&str] = &["k1.webp", "k2.webp", "k3.webp", "k4.webp", "k5.webp"];

/// 目标端 = 全部共享 + 给定孤儿。返回 `Vec<&str>` 以直接喂给 `mirror()`。
fn dst_side<'a>(shared: &'a [&'a str], orphans: &'a [&'a str]) -> Vec<&'a str> {
    shared.iter().copied().chain(orphans.iter().copied()).collect()
}

/// 一个对齐夹具：本地 `src/` ↔ 具名 remote `decklocal:`（alias 指向 `dst/`）。
/// `sub` 非空时两边都用带子路径的端点——出厂四条都是桶根，只有这里能测到前缀处理。
async fn mirror(sub: &str, src_names: &[&str], dst_names: &[&str])
    -> (Fx, rclone_deck_lib::runner::Runner, DaemonGuard)
{
    let fx = Fx::new();
    // alias 的 remote 指向 fx.dst()，所以"云端有什么"就是写进这个目录的文件。
    // 子路径那一档要**两侧同时下潜一层**（src/imgprompt 与 dst/imgprompt），两端才同坐标系；
    // 出厂四条都在桶根，测不到这一档——这就是本夹具存在的理由。
    let src_dir = if sub.is_empty() { fx.src() } else { fx.src().join(sub) };
    let dst_dir = if sub.is_empty() { fx.dst() } else { fx.dst().join(sub) };
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&dst_dir).unwrap();
    seed(&src_dir, src_names);
    seed(&dst_dir, dst_names);
    let local = fwd(&src_dir);
    let remote = fx.remote(sub);
    let runner_job = job_at("m", JobKind::Sync, &local, &remote);
    let (runner, guard) = setup_with_conf(fx.root(), Some(fx.conf()), vec![runner_job]).await;
    (fx, runner, guard)
}

#[tokio::test]
async fn a_preview_lists_every_doomed_name_and_reports_the_ratio() {
    // 3 个共享 + 2 个孤儿：doomed 2 / dst 5 → 2×2=4 < 5，守卫不触发（形状见上面那段注释）。
    let (_fx, runner, _g) = mirror("", SHARED, &dst_side(SHARED, ORPHANS)).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert_eq!(p.doomed.len(), 2, "逐条名字，不是只给计数：{:?}", p.doomed);
    assert_eq!((p.src_count, p.dst_count), (3, 5));
    assert_eq!(p.doomed_ratio, "2/5");
    assert!(!p.token.is_empty());
    assert!(p.list_requests >= 1, "这次预览真发出去多少个 LIST 要有观测值");
}

/// 那一趟**要补多少**也得跟着预览走：层里那颗红色的要说"同时补上那 M 个"，
/// 而那个数不许界面自己减（`src_count - (dst_count - doomed)` 这一族在 TS 里是禁的）。
/// 本地 4（3 共享 + 1 只在这边）、云端 5（3 共享 + 2 只在那边）→ doomed 2 / pending 1。
#[tokio::test]
async fn the_preview_reports_how_many_the_same_run_will_fill_in() {
    let (_fx, runner, _g) = mirror("",
        &["a.webp", "b.webp", "c.webp", "only-here.webp"],
        &["a.webp", "b.webp", "c.webp", "x1.webp", "x2.webp"]).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert_eq!(p.doomed.len(), 2, "删那一半还是 2：{:?}", p.doomed);
    assert_eq!(p.pending, 1, "本地有、云端没有的那一个要说得出数：{p:?}");
    // 两半都进日志：只记一半的日志，回头读它的人会以为对齐只删东西。
    let log = std::fs::read_to_string(runner.last_log("m").await.unwrap()).unwrap();
    assert!(log.contains("pending=1"), "预览那行没把补上的数记下来：{log}");
}

#[tokio::test]
async fn a_subpath_mirror_reports_names_relative_to_its_own_side() {
    // 端点模型与"相对各自根"那套归一化的端到端证人：出厂四条都是桶根，测不到这一档。
    let (_fx, runner, _g) = mirror("imgprompt",
        &["a.webp", "b.webp", "c.webp"], &["a.webp", "b.webp", "c.webp", "stale.webp", "sub/gone.webp"]).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert_eq!(p.doomed, vec!["stale.webp", "sub/gone.webp"], "不许带 imgprompt/ 前缀：{:?}", p.doomed);
}

#[tokio::test]
async fn an_emptied_source_side_refuses_instead_of_offering_the_whole_destination() {
    let (_fx, runner, _g) = mirror("", &[], &["a.webp", "b.webp"]).await;
    let err = runner.preview_mirror("m").await.unwrap_err().to_string();
    assert!(err.contains("一个对象都没有"), "{err}");
    assert!(err.contains("src 指错") || err.contains("exclude"), "话术要说出该检查什么：{err}");
    assert!(!err.contains("Sync"), "枚举名不是人话：{err}");
    assert_eq!(files_in(_fx.dst().as_ref()), vec!["a.webp", "b.webp"], "拒绝就一个都不该少");
}

#[tokio::test]
async fn a_majority_of_the_destination_doomed_refuses_the_preview() {
    // src 只保住 1 个，dst 有 3 个 → doomed 2/3，达到一半。
    let (_fx, runner, _g) = mirror("", &["a.webp"], &["a.webp", "b.webp", "c.webp"]).await;
    let err = runner.preview_mirror("m").await.unwrap_err().to_string();
    assert!(err.contains("一半"), "{err}");
    assert!(err.contains(&RATIO_DEN.to_string()), "要带阈值，操作员才知道那是假设值：{err}");
    assert!(err.contains("rclone"), "要说得出替代做法，不是只说不：{err}");
}

#[tokio::test]
async fn a_preview_refuses_a_non_mirror_job() {
    let fx = Fx::new();
    seed(&fx.src(), &["a.webp"]);
    let local = fwd(&fx.src());
    let remote = fx.remote("");
    let (runner, _g) = setup_with_conf(fx.root(), Some(fx.conf()),
        vec![job_at("c", rclone_deck_lib::config::jobs::JobKind::Copy, &local, &remote)]).await;
    let err = runner.preview_mirror("c").await.unwrap_err().to_string();
    assert!(err.contains("复制"), "{err}");
    assert!(!err.contains("Copy"), "{err}");
}

/// 下载方向（`src` 是 remote、`dst` 是本地目录）。上面所有用例都是上传方向那侧；
/// spec §8 第 ④⑤ 条点名要**两个方向各红一次**，因为这个功能的全部新东西就是"方向可以反"。
/// 目录形状与 `mirror()` 相同，只是把两端在任务里对调。
async fn mirror_back(sub: &str, remote_names: &[&str], local_names: &[&str])
    -> (Fx, rclone_deck_lib::runner::Runner, DaemonGuard)
{
    let fx = Fx::new();
    let src_dir = if sub.is_empty() { fx.src() } else { fx.src().join(sub) };
    let dst_dir = if sub.is_empty() { fx.dst() } else { fx.dst().join(sub) };
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&dst_dir).unwrap();
    // alias 指向 fx.dst()，所以"云端有的"仍然是写进 dst 的那些——变的只是谁是 src。
    seed(&dst_dir, remote_names);
    seed(&src_dir, local_names);
    let remote = fx.remote(sub);
    let local = fwd(&src_dir);
    let (runner, guard) =
        setup_with_conf(fx.root(), Some(fx.conf()), vec![job_at("m", JobKind::Sync, &remote, &local)]).await;
    (fx, runner, guard)
}

#[tokio::test]
async fn the_download_direction_names_its_doomed_relative_to_the_local_side() {
    // 前缀取错那一族在这个方向最要命：错的话 doomed 会等于本地全部文件（§4.1 记的那次事故）。
    // 云端 = alias 指向的 dst/sub，本地 = src/sub。
    let (fx, runner, _g) = mirror_back("imgprompt", &["a.webp", "b.webp"],
        &["a.webp", "b.webp", "only-local.webp"]).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert_eq!(p.doomed, vec!["only-local.webp"], "不许带 imgprompt/ 前缀，也不许把云端那侧列进来：{:?}", p.doomed);
    // 预览是只读的：两侧一个文件都不该动。
    assert_eq!(files_in(&fx.src().join("imgprompt")), vec!["a.webp", "b.webp", "only-local.webp"]);
    assert_eq!(files_in(&fx.dst().join("imgprompt")), vec!["a.webp", "b.webp"]);
}

#[tokio::test]
async fn the_download_direction_refuses_an_emptied_remote_side() {
    // 这一档的形状是"云端一个都没有、本地一堆"——真跑就会清空本地目录，正是必须拦住的那一种。
    let (_fx, runner, _g) = mirror_back("", &[], &["a.webp", "b.webp"]).await;
    let err = runner.preview_mirror("m").await.unwrap_err().to_string();
    assert!(err.contains("一个对象都没有"), "{err}");
    assert!(!err.contains("Sync") && !err.contains("Invariant"), "内部名词不是人话：{err}");
}

#[tokio::test]
async fn the_download_direction_refuses_a_majority_of_the_local_side() {
    // 本地 3 个、云端只保住 1 个 → doomed 2/3，达到一半。与上传方向那条互为反向证人。
    let (_fx, runner, _g) = mirror_back("", &["a.webp"], &["a.webp", "b.webp", "c.webp"]).await;
    let err = runner.preview_mirror("m").await.unwrap_err().to_string();
    assert!(err.contains("一半"), "{err}");
    assert!(err.contains(&RATIO_DEN.to_string()), "{err}");
}

#[tokio::test]
async fn a_second_preview_on_the_same_pair_voids_the_first_token() {
    let (_fx, runner, _g) = mirror("", &["a.webp", "b.webp"], &["a.webp", "b.webp", "stale.webp"]).await;
    let p1 = runner.preview_mirror("m").await.unwrap();
    let p2 = runner.preview_mirror("m").await.unwrap();
    assert_eq!(p1.doomed, p2.doomed);
    assert_ne!(p1.token, p2.token, "同一对目录同时只该有一份待确认清单");
    // 作废判据不在这里验（要等"执行前重列、逐条比对"那一步有消费者），这里只验"新 token 换旧的"
    assert!(!matches!(runner.state("m").await, TaskState::Running { .. }));
}

/// 裁定 R64：远端那个子路径在桶里**还不存在**时，`operations/list` 对它回 404（spec §3.2 实测），
/// 而"第一次把一个新子目录推上云"是用户的真实用法，不是故障。这一条钉住那一档被读成
/// "这一侧什么都没有"（于是 doomed 为空、什么都不删），而不是把整次预览弄成一次错误。
#[tokio::test]
async fn a_remote_destination_subpath_that_does_not_exist_yet_previews_as_an_empty_doomed_list() {
    let fx = Fx::new();
    seed(&fx.src(), SHARED);
    // 故意**不**建 `fx.dst()/fresh`：alias 指向 fx.dst()，所以"桶里"那个子目录就是不存在。
    let local = fwd(&fx.src());
    let remote = fx.remote("fresh");
    let (runner, _g) = setup_with_conf(fx.root(), Some(fx.conf()),
        vec![job_at("m", JobKind::Sync, &local, &remote)]).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert!(p.doomed.is_empty(), "全新的远端目录本该是空集，实际交出的删除清单是 {:?}", p.doomed);
    assert_eq!((p.src_count, p.dst_count), (3, 0), "两侧看见了多少要能自查：{:?}", (p.src_count, p.dst_count));
    assert_eq!(files_in(&fx.src()), vec!["s1.webp", "s2.webp", "s3.webp"], "预览不许动源端");
    assert!(files_in(&fx.dst()).is_empty(), "预览不许在对端造出目录或文件");
}

/// 同一档在另一头的形状：404 映射成空集之后，"src 一个都没有"仍然要落进既有的空基准拒绝，
/// 不许因为"这一侧只是没看见"就交出一份等于整个对端的清单。话术还得说得是人话，
/// 不是 `ListError::NotFound` 那句"该前缀在远端不存在"。
#[tokio::test]
async fn an_absent_remote_source_side_still_hits_the_empty_source_refusal() {
    let fx = Fx::new();
    seed(&fx.dst(), &["a.webp", "b.webp"]);
    seed(&fx.src(), &["a.webp", "b.webp", "c.webp"]);
    let remote = fx.remote("never-uploaded");
    let local = fwd(&fx.src());
    let (runner, _g) = setup_with_conf(fx.root(), Some(fx.conf()),
        vec![job_at("m", JobKind::Sync, &remote, &local)]).await;
    let err = runner.preview_mirror("m").await.unwrap_err().to_string();
    assert!(err.contains("一个对象都没有"), "{err}");
    assert!(!err.contains("不存在"), "404 原样抛出会把「这一侧是空的」说成一次故障：{err}");
    assert_eq!(files_in(&fx.src()), vec!["a.webp", "b.webp", "c.webp"], "拒绝就一个都不该少");
}

/// 拒绝也要说清它当时看见的是什么：逐条清单排在两道拒绝**之前**（spec §4.1），
/// 所以被比例守卫拦掉的那一次，日志里必须已经躺着那两条"会删掉"。顺序写反就红。
#[tokio::test]
async fn a_refused_preview_has_already_logged_the_item_by_item_list() {
    let (_fx, runner, _g) = mirror("", &["a.webp"], &["a.webp", "b.webp", "c.webp"]).await;
    let err = runner.preview_mirror("m").await.unwrap_err();
    assert!(err.to_string().contains("一半"), "{err}");
    let log = rclone_deck_lib::runner::tasklog::TaskLog::tail(
        &runner.last_log("m").await.expect("预览也该有日志"), 8192).unwrap();
    assert!(log.contains("会删掉：b.webp") && log.contains("会删掉：c.webp"),
        "被拒绝的预览没逐条写出它看见的东西：{log}");
    assert!(log.find("会删掉：c.webp") < log.find("预览被拒绝"),
        "逐条清单排在了拒绝之后：{log}");
}

// --------------------------------------------------------------- 执行前重列、逐条比对（spec §4.1 第 3 步）

#[tokio::test]
async fn a_mirror_run_deletes_exactly_the_previewed_orphans_and_reports_the_count() {
    let (fx, runner, _g) = mirror("", SHARED, &dst_side(SHARED, ORPHANS)).await;
    let p = runner.preview_mirror("m").await.unwrap();
    let st = runner.run_mirror("m", &p.token).await.unwrap();
    let TaskState::Done { stats, previewed, dry_run, .. } = st else { panic!("该跑完：{st:?}") };
    assert!(!dry_run);
    assert_eq!(stats.deletes, 2, "真删的必须就是清单上那批");
    assert_eq!(previewed, Some(2), "预览数要作为数据走出去，界面不许自己重算");
    assert_eq!(files_in(&fx.dst()), vec!["s1.webp", "s2.webp", "s3.webp"], "孤儿没了、共有的还在");
    // 界面那一份与日志那一份必须是同一个数，而且是**同一行**上并排的两个字段：那一行同时写着
    // rclone 自己报的 `deletes=` 与面板确认过的 `previewed=`。只要求"日志里某处出现过"是假证人——
    // 开始实删那行本来就写着预览数，删掉终态那行的字段它照样绿。
    let log = rclone_deck_lib::runner::tasklog::TaskLog::tail(
        &runner.last_log("m").await.expect("跑完该有日志"), 8192).unwrap();
    let n = previewed.expect("终态里的预览数");
    let line = log.lines().find(|l| l.contains("任务结束")).expect("终态那一行没落进日志尾");
    assert!(line.contains(&format!("previewed={n}")) && line.contains(&format!("deletes={n}")),
        "终态那行没把两个数说成同一个 {n}：{line}");
}

#[tokio::test]
async fn a_change_between_preview_and_execute_refuses_the_run_instead_of_guessing() {
    // spec §6 那一行的端到端证人：预览之后对端多了一个，逐条比对必须拒掉这次实删。
    // 形状按文件顶部那条规矩挑的：改动前后都 `doomed × 2 < dst_count`（1/6 → 2/7），
    // 比例守卫全程安静，所以这道拒绝只能是逐条比对那一道。
    let (fx, runner, _g) = mirror("", K5, &dst_side(K5, &["stale.webp"])).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert_eq!(p.doomed, vec!["stale.webp"]);
    seed(&fx.dst(), &["late.webp"]);   // 人读清单那几十秒里，对端多了一个
    let err = runner.run_mirror("m", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("清单变了"), "{err}");
    assert!(err.contains("重新预览"), "{err}");
    assert_eq!(files_in(&fx.dst()).len(), 7, "拒绝执行就一个都不该少：{:?}", files_in(&fx.dst()));
    // token 已经被吃掉：同一个 token 再试只能撞见"这份确认不作数"。
    let again = runner.run_mirror("m", &p.token).await.unwrap_err().to_string();
    assert!(again.contains("不作数"), "{again}");
}

#[tokio::test]
async fn an_equal_size_swap_of_the_doomed_set_refuses_too_and_deletes_nothing() {
    // 逐条比对与"只比个数"唯一的分野在这一档：确认过的那个文件消失了、同时冒出一个没确认过的，
    // 删除候选**条数不变**。光比个数会放行这次实删，删掉一个从头到尾没人看过的东西，
    // 而预览数与实删数还正好相等，事后那道不变量比对也是哑的。
    let (fx, runner, _g) = mirror("", K5, &dst_side(K5, &["stale.webp"])).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert_eq!(p.doomed, vec!["stale.webp"]);
    std::fs::remove_file(fx.dst().join("stale.webp")).unwrap();
    seed(&fx.dst(), &["late.webp"]);   // 一进一出：dst 总数与 doomed 条数都跟预览时一模一样
    let err = runner.run_mirror("m", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("清单变了"), "{err}");
    assert_eq!(
        files_in(&fx.dst()),
        vec!["k1.webp", "k2.webp", "k3.webp", "k4.webp", "k5.webp", "late.webp"],
        "没确认过的东西一个都不该少：{:?}", files_in(&fx.dst()));
}

#[tokio::test]
async fn an_emptied_source_side_between_preview_and_confirm_names_this_side_not_the_peer() {
    // 钉的是**顺序**：两道复核排在逐条比对之前（spec §4.1"两道拒绝排在比对之前"）。
    // 源端在确认那几十秒里空掉（盘掉挂载是最真实的一种），此时删除候选也变了；若先报"清单变了"，
    // 操作员被指去查对端，而该看的是自己这一侧。
    let (fx, runner, _g) = mirror("", K5, &dst_side(K5, &["stale.webp"])).await;
    let p = runner.preview_mirror("m").await.unwrap();
    for n in K5 {
        std::fs::remove_file(fx.src().join(n)).unwrap();   // 这一侧没了
    }
    let err = runner.run_mirror("m", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("复核"), "说的是别的判据：{err}");
    assert!(err.contains("这一侧空=true"), "没点明是这一侧空了：{err}");
    assert!(!err.contains("清单变了"), "源端掉了却说成对端被动过，会把人支去检查错的那一侧：{err}");
    assert_eq!(
        files_in(&fx.dst()),
        vec!["k1.webp", "k2.webp", "k3.webp", "k4.webp", "k5.webp", "stale.webp"],
        "拒绝执行就一个都不该少：{:?}", files_in(&fx.dst()));
}

#[tokio::test]
async fn a_subpath_change_also_refuses_at_the_deeper_endpoint() {
    // 与上面那条同一形状，只是两端同时下潜一层：改动前后都是 1/6 → 2/7，比例守卫安静。
    let (fx, runner, _g) = mirror("imgprompt", K5, &dst_side(K5, &["stale.webp"])).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert_eq!(p.doomed, vec!["stale.webp"]);
    std::fs::write(fx.dst().join("imgprompt").join("late.webp"), b"x").unwrap();
    let err = runner.run_mirror("m", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("清单变了"), "{err}");
    assert_eq!(files_in(&fx.dst().join("imgprompt")).len(), 7, "一个都不该少");
}

#[tokio::test]
async fn an_unchanged_pair_still_runs_when_there_is_nothing_to_delete() {
    // 空 doomed 是"确实不用删"，不是"基准缺失"（R29 的第三态）。
    let (fx, runner, _g) = mirror("", &["a.webp"], &["a.webp"]).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert!(p.doomed.is_empty());
    let st = runner.run_mirror("m", &p.token).await.unwrap();
    assert!(matches!(st, TaskState::Done { previewed: Some(0), .. }), "{st:?}");
    assert_eq!(files_in(&fx.dst()), vec!["a.webp"]);
}

#[tokio::test]
async fn an_unknown_or_reused_token_never_reaches_the_daemon() {
    let (fx, runner, _g) = mirror("", &["a.webp"], &["a.webp", "stale.webp"]).await;
    let err = runner.run_mirror("m", "prev-00000000000000000000000000000000").await.unwrap_err().to_string();
    assert!(err.contains("重新预览"), "{err}");
    assert!(!err.contains("token"), "内部名词不许上界面：{err}");
    assert_eq!(files_in(&fx.dst()).len(), 2, "没授权就什么都没动：{:?}", files_in(&fx.dst()));
}

#[tokio::test]
async fn a_valid_token_for_another_job_does_not_authorize_this_one() {
    // 同一份 token 是**按任务**登记的：拿 A 的确认去授权 B，即便 B 此刻的清单和当初预览的一模一样，
    // 也不许走——否则预览与确认可以分属两个任务，"你看到的就是被删的"从中间断开。
    let fx = Fx::new();
    let ra = fx.remote("");
    let rb = fx.remote("b");
    seed(&fx.src(), &["a.webp", "b.webp"]);
    seed(&fx.dst(), &["a.webp", "b.webp", "stale.webp"]);
    let (runner, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![
        job_at("a", JobKind::Sync, &fwd(&fx.src()), &ra),
        job_at("b", JobKind::Sync, &fwd(&fx.src()), &rb),
    ]).await;
    let p = runner.preview_mirror("a").await.unwrap();
    let err = runner.run_mirror("b", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("不作数"), "{err}");
    assert_eq!(files_in(&fx.dst()), vec!["a.webp", "b.webp", "stale.webp"], "拿别人的确认什么都不能动");
    // 那次被拒的调用**吃掉**了 A 的授权：同一个 token 回到自己名下也不该再用第二次，
    // 否则"先拿它去试一圈别的任务"就是一条免费的探测路径。
    let back = runner.run_mirror("a", &p.token).await.unwrap_err().to_string();
    assert!(back.contains("不作数"), "被拒的那一次把授权留下了：{back}");
    assert_eq!(files_in(&fx.dst()), vec!["a.webp", "b.webp", "stale.webp"], "回来也别偷偷跑：{:?}", files_in(&fx.dst()));
}

#[tokio::test]
async fn a_run_that_cannot_get_the_lock_has_already_consumed_the_confirmation() {
    // 钉的是"授权排在 `acquire` 之前"这一句：先手动占住这一对目录的键，再拿一份有效确认去跑。
    // 拿到 Busy 之后放开键、用**同一个** token 再试一次——若授权是在拿到锁之后才吃的，
    // 这一次就会真跑起来并把孤儿删掉，界面上看到的是"刚才那个没授权的清单居然作数了"。
    let (fx, runner, _g) = mirror("", SHARED, &dst_side(SHARED, ORPHANS)).await;
    let p = runner.preview_mirror("m").await.unwrap();
    // 占的是这条任务**整组**的树键（两侧各一把，来源与实现同一个：`Job::scope_keys()`）。
    // 本文件占键那一行从来不带 `tokio::time::timeout`（那是 `tests/job_mutex.rs` 的纪律，
    // 它防的是"申领退化成了排队"），照旧不加。
    let keys = runner.job("m").expect("夹具里有 m").scope_keys();
    let held = runner.mutex().acquire_all(&keys, "手动占用").await.unwrap();
    match runner.run_mirror("m", &p.token).await {
        Err(RunError::Busy { owner }) => assert!(owner.contains("手动占用"), "{owner}"),
        other => panic!("这一对目录被人占着，实删却跑成了 {other:?}"),
    }
    drop(held);
    let err = runner.run_mirror("m", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("不作数"), "拿不到锁的那一次把授权留回了桌上：{err}");
    assert_eq!(
        files_in(&fx.dst()),
        vec!["gone.webp", "s1.webp", "s2.webp", "s3.webp", "stale.webp"],
        "被拒之后对端不该有任何变化：{:?}", files_in(&fx.dst()));
}

/// 同一对目录上**两条任务**：对齐 `m`（本地→云端）与复制 `up`（同方向）。
/// `mirror()` 只注册一条，这一条是为了端到端打到 `invalidate_previews`——
/// "人看完清单、中间先去跑了一次上传"是真实序列，而它的后果是让云端多出文件、旧清单失效。
async fn mirror_pair_with_a_copy_row(sub: &str)
    -> (Fx, rclone_deck_lib::runner::Runner, DaemonGuard)
{
    let fx = Fx::new();
    let src_dir = if sub.is_empty() { fx.src() } else { fx.src().join(sub) };
    let dst_dir = if sub.is_empty() { fx.dst() } else { fx.dst().join(sub) };
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::create_dir_all(&dst_dir).unwrap();
    seed(&src_dir, &["a.webp", "b.webp", "new-comer.webp"]);   // 前两个云端也有，第三个是待上传的
    seed(&dst_dir, &["a.webp", "b.webp", "stale.webp"]);        // stale 就是那份旧清单上唯一的删除候选
    let local = fwd(&src_dir);
    let remote = fx.remote(sub);
    let jobs = vec![
        job_at("m", rclone_deck_lib::config::jobs::JobKind::Sync, &local, &remote),
        job_at("up", rclone_deck_lib::config::jobs::JobKind::Copy, &local, &remote),
    ];
    let (runner, guard) = setup_with_conf(fx.root(), Some(fx.conf()), jobs).await;
    (fx, runner, guard)
}

#[tokio::test]
async fn a_copy_on_the_same_pair_between_preview_and_confirm_voids_the_confirmation() {
    // `invalidate_previews` 的端到端证人：只验"新 token 覆盖旧 token"那一条看不到这里。
    // 场景是真的：人点了「让两边对齐」，然后先去跑了一次上传（或反向那一个），回来直接确认。
    // 那次 copy 会改变对端，旧清单上承诺的删除范围已经不成立了。
    let (_fx, runner, _g) = mirror_pair_with_a_copy_row("").await;
    let p = runner.preview_mirror("m").await.unwrap();
    // 同配对的**真**复制跑一次（`up` 与 `m` 交的是**同一组** `Job::write_scopes()`，
    // 所以这一跑动的就是对齐那两棵树，确认随之作废）。
    runner.run_copy("up", false).await.unwrap();
    let err = runner.run_mirror("m", &p.token).await.unwrap_err().to_string();
    // 说得出"不作数"而不是"清单变了"：这条证的就是作废那条路，不是重列比对那条路。
    assert!(err.contains("不作数"), "{err}");
    assert!(!err.contains("不一样"), "作废与复核是两条判据，别混成一句：{err}");
}

#[tokio::test]
async fn a_trial_run_between_preview_and_confirm_keeps_the_confirmation_valid() {
    // 裁定"试跑不作废确认"这一选择的证人：预览 → 同一个对齐任务先试跑（只读）→ 再确认，必须还能跑。
    // 若把 `invalidate_previews` 也作用在 dry run 上，这里会拿到"不作数"而红。
    let (fx, runner, _g) = mirror("", SHARED, &dst_side(SHARED, ORPHANS)).await;
    let p = runner.preview_mirror("m").await.unwrap();
    runner.run_copy("m", true).await.unwrap();   // 只读试跑：一个东西都不删
    let st = runner.run_mirror("m", &p.token).await.unwrap();
    let TaskState::Done { previewed, dry_run, .. } = st else { panic!("该跑完：{st:?}") };
    assert!(!dry_run, "确认那一步才是实删：{st:?}");
    assert_eq!(previewed, Some(2));
    assert_eq!(files_in(&fx.dst()), vec!["s1.webp", "s2.webp", "s3.webp"]);
}

#[tokio::test]
async fn a_refusal_reevaluated_at_execution_names_the_refusal_not_the_list_change() {
    // 复核时的拒绝（空源 / 占比）这一档的形状是"删除候选一个字没变、但两侧计数变成事故形状"，
    // 所以它排在比对之前也不会被比对抢走话术：这里说的必须是复核拒绝，不是"清单变了"。
    // 这也是执行侧占比拒绝唯一的活证人——去掉那段复核就什么都删了。
    // （源端整个空掉那一档另有证人，它同时也在钉"复核排在比对之前"这个顺序本身。）
    let (fx, runner, _g) = mirror("", &["a.webp", "b.webp"], &["a.webp", "b.webp", "stale.webp"]).await;
    let p = runner.preview_mirror("m").await.unwrap();
    assert_eq!(p.doomed, vec!["stale.webp"]);
    std::fs::remove_file(fx.dst().join("b.webp")).unwrap();   // b 本地还有、只是不在对端了，删除候选一个没变
    let err = runner.run_mirror("m", &p.token).await.unwrap_err().to_string();
    assert!(err.contains("复核"), "{err}");
    assert!(!err.contains("清单变了"), "占比拒绝被说成了清单变化：{err}");
    assert!(fx.dst().join("stale.webp").exists(), "复核拒绝就一个都不该少：{:?}", files_in(&fx.dst()));
}

/// spec §4.1 留下的那条**交叉核对**，加上"冷点那条路被拒之后什么都没动"。
///
/// 为什么要单独一条：面板的 `doomed` 走的是 `operations/list` + 自己的集合差，
/// 「先试跑」的 `deletes` 走的是 rclone 自己的匹配器 —— 两条算路互相独立，相等才算两边看的
/// 是同一批东西。只钉其中一边，两边各自漂移就没人喊（`runner_local.rs` 那两条各钉一边）。
/// 同一夹具里还要把「开始」那条路钉住：被拒的实删一个文件都不许少 —— 它现在被拒是因为
/// 那道闸装在 `run_copy` 第一行，去掉闸门这里就红（去掉调用不等于去掉函数：纯函数那两条
/// 证人在 `commands_surface.rs` 里照样绿，这正是"只有函数、没有门"这种实现会被抓住的地方）。
#[tokio::test]
async fn the_transport_path_refuses_a_real_mirror_but_lets_the_trial_run_through() {
    let (fx, runner, _g) = mirror("", SHARED, &dst_side(SHARED, ORPHANS)).await;
    let err = runner.run_copy("m", false).await.unwrap_err().to_string();
    assert!(err.contains("对齐"), "得说出这是对齐任务：{err}");
    assert_eq!(files_in(&fx.dst()).len(), 5, "被拒的实删一个都不该动：{:?}", files_in(&fx.dst()));

    let p = runner.preview_mirror("m").await.unwrap();
    let st = runner.run_copy("m", true).await.unwrap();
    // 借来匹配：这条 `Done` 之后还要拿 `{st:?}` 当失败信息，把 stats 移走就借用不了。
    let TaskState::Done { stats, dry_run, .. } = &st else { panic!("试跑该跑完：{st:?}") };
    assert!(*dry_run, "布尔丢了的话，这一跑就是真删：{st:?}");
    assert_eq!(stats.deletes, p.doomed.len() as u64,
        "rclone 自己算的 deletes 必须等于面板算的 doomed（spec §4.1 的交叉核对）");
    assert_eq!(files_in(&fx.dst()).len(), 5, "试跑不许动任何东西");
}

// ------------------------------ 邻居 Scope 那一格：读不作废、写实删作废（Task 5 修复轮 1）

/// 两条任务的**位置集合相交而不相等**：对齐 `a` 是 `src/` → `decklocal:img`，
/// 对齐 `b` 是 `src/other/` → `decklocal:img/2026`。alias 后端就是 `fx.dst()` 那个目录，
/// `fx.remote(sub)` 交出 `decklocal:<sub>`，所以那两个 remote 就是 `dst/img` 与 `dst/img/2026`，
/// 播种只要往这两个目录里写文件。
///
/// 这一对形状是挑出来的，两半各要一格：
/// - `img` 是 `img/2026` 的祖先 ⇒ **写**进 `img/2026` 必须作废那份"关于 `img`"的预览；
/// - 两条任务的 `mutex_key()` 一个也不等（`…/src=>decklocal:img` 与 `…/src/other=>decklocal:img/2026`），
///   抽取前的 pair-key 判据在这一对上**什么都作不掉** ⇒ 这一格只有放宽之后的判据分得出来。
///   现有那批证人把两条任务放在**同一对**目录上（见 `mirror_pair_with_a_copy_row`），
///   那种夹具下两种判据给出同一个答案，所以放宽的那一格从来没有端到端证人看过。
async fn mirror_two_jobs_on_neighbouring_scopes()
    -> (Fx, rclone_deck_lib::runner::Runner, DaemonGuard)
{
    let fx = Fx::new();
    // `a` 的源端是整棵 `src/`（`side_set` 递归列举，所以 `other/` 也在 a 那一侧的集合里），
    // `b` 的源端只有那一层。
    seed(&fx.src(), &["s1.webp", "s2.webp", "s3.webp", "s4.webp"]);
    seed(&fx.src().join("other"), &["n1.webp"]);
    // a 的目标端 = 4 个共享 + 1 个孤儿 → doomed 1 / dst 5，`1×2 < 5`，比例守卫全程安静
    //（形状规矩见文件顶部那一段）。
    seed(&fx.dst().join("img"), &["s1.webp", "s2.webp", "s3.webp", "s4.webp", "gone.webp"]);
    // 故意**不**建 `dst/img/2026`：b 那一侧的远端走裁定 R64 那一档（列举根 404 读成空集），
    // 于是 b 的 doomed 是空集——它那一跑只往上写东西、一个都不删。
    let (runner, guard) = setup_with_conf(fx.root(), Some(fx.conf()), vec![
        job_at("a", JobKind::Sync, &fwd(&fx.src()), &fx.remote("img")),
        job_at("b", JobKind::Sync, &fwd(&fx.src().join("other")), &fx.remote("img/2026")),
    ]).await;
    (fx, runner, guard)
}

#[tokio::test]
async fn a_neighbouring_write_voids_the_outer_preview_while_a_neighbouring_read_does_not() {
    let (fx, runner, _g) = mirror_two_jobs_on_neighbouring_scopes().await;

    // ── 第一半：**读**不作废。预览 a、再预览 b（一次只读），然后确认 a —— 必须还能跑完。
    // 发凭证处用的是 `PreviewRegistry::supersede`（只顶掉"scope 集合完全相同"的旧清单），
    // 这一半钉的就是那个选择：换成 `invalidate` 的话这里拿到"不作数"——
    // b 那份预览的源端 `src/other` 落在 a 那份预览的源端 `src` 里面，两者交集非空。
    let pa = runner.preview_mirror("a").await.unwrap();
    assert_eq!(pa.doomed, vec!["gone.webp"], "a 那份清单只关于它自己两侧：{:?}", pa.doomed);
    let _pb = runner.preview_mirror("b").await.unwrap();
    let st = runner.run_mirror("a", &pa.token).await
        .unwrap_or_else(|e| panic!("预览 b 是一次读，它不该作废 a 正在核对的那份清单，实际拒绝：{e}"));
    assert!(matches!(st, TaskState::Done { previewed: Some(1), dry_run: false, .. }),
        "确认 a 那一步该是实删跑完：{st:?}");
    assert_eq!(
        files_in(&fx.dst().join("img")),
        vec!["other/n1.webp", "s1.webp", "s2.webp", "s3.webp", "s4.webp"],
        "孤儿没了、a 源端那一层跟着传上去：{:?}", files_in(&fx.dst().join("img")));

    // ── 第二半：**写**作废。上面那一跑把 a 的两侧对齐了，重新预览 a 拿到的是空 doomed；
    // 然后让 b 往它自己那棵树（`img/2026`）里真写一个东西，再拿 a 那份确认去跑。
    let pa2 = runner.preview_mirror("a").await.unwrap();
    assert!(pa2.doomed.is_empty(), "第二半验的是作废，不是逐条比对：{:?}", pa2.doomed);
    // b 的真写只能走它自己的对齐入口：`run_copy("b", false)` 会被 `transport_kind_gate` 拒掉
    //（b 是对齐任务，那条入口只受理复制与试跑），所以这里先预览 b、再 `run_mirror(b)`：
    // 它删 0 个、把 `other/n1.webp` 同步进 `img/2026/`，那一次 `sync/sync` 就是"被写的一侧"，
    // 走的是 `run_mirror` 复核通过之后那句 `invalidate_writes`。
    let pb2 = runner.preview_mirror("b").await.unwrap();
    runner.run_mirror("b", &pb2.token).await
        .unwrap_or_else(|e| panic!("b 自己那份预览该跑得完，写不成就没有第二半的作废条件：{e}"));
    assert_eq!(files_in(&fx.dst().join("img/2026")), vec!["n1.webp"],
        "b 那一跑必须真的写进去，否则第二半什么都没动");

    let err = runner.run_mirror("a", &pa2.token).await.unwrap_err().to_string();
    // 三条判据不许混成一句：这一格要的必须是**作废**那一句。b 那一跑同时也改了 a 重新列出来的集合
    //（对端多出一个 `2026/n1.webp`），所以"清单变了"在那一步同样成立——只断言"被拒"分不出是哪一条
    // 判据拒的；而把 `invalidate_writes` 的判据退回 pair-key 语义（比 `job.mutex_key()`）时，
    // 这里恰好改判成"清单变了"，因为这两条任务不是同一对目录。
    assert!(err.contains("不作数"), "b 往 a 那棵树的子目录里写了东西，a 那份预览就该作废：{err}");
    assert!(!err.contains("清单变了"), "作废与逐条比对是两条判据，别混成一句：{err}");
    assert!(!err.contains("复核"), "两道复核也没在这里替作废说话：{err}");
    assert_eq!(
        files_in(&fx.dst().join("img")),
        vec!["2026/n1.webp", "other/n1.webp", "s1.webp", "s2.webp", "s3.webp", "s4.webp"],
        "被拒的确认一个东西都不该动：{:?}", files_in(&fx.dst().join("img")));
}

// ============================================================ 不预览直接批
//
// 批准的对象从「这一份名单」换成「这一趟」：按下「让两边对齐」不该先付一趟 LIST 才按得动红色那颗。
// 这里换掉的不是安全判据，是**参照物**——逐条比对要有"人看过的那一份"才比得出来；没人看过就没有
// 参照物，此时守这一趟的是那两道拒绝，它们照旧排在动手之前。
// 所以这一组只问三件事：不预览跑得通、两道拒绝照旧咬、凭证照旧一次一用。
// 「看过名单」那一档一分不丢，它的证人还是原来那两条：
// `a_change_between_preview_and_execute_refuses_the_run_instead_of_guessing` 与
// `an_equal_size_swap_of_the_doomed_set_refuses_too_and_deletes_nothing`。

#[tokio::test]
async fn a_confirmation_without_a_preview_deletes_what_the_run_itself_lists() {
    let (fx, runner, _g) = mirror("", SHARED, &dst_side(SHARED, ORPHANS)).await;
    let token = runner.confirm_mirror("m").await.expect("确认这一步不该去列清单");
    let st = runner.run_mirror("m", &token).await.unwrap();
    let TaskState::Done { stats, previewed, .. } = st else { panic!("该跑完：{st:?}") };
    assert_eq!(stats.deletes, 2, "删的必须是动手那一刻列出来的那批孤儿：{stats:?}");
    // 没有预览也有一条"数要作为数据走出去"的口径：终态报的那个就是这一跑列出来的数。
    assert_eq!(previewed, Some(2), "界面不许自己再数一遍：{previewed:?}");
    assert_eq!(files_in(&fx.dst()), vec!["s1.webp", "s2.webp", "s3.webp"], "孤儿没了、共有的还在");
}

#[tokio::test]
async fn a_confirmation_without_a_preview_still_refuses_an_emptied_source_side() {
    // 这一档是那条不变量真正被测的地方：没人看过名单，所以**只有**这两道拒绝在守着整棵对端。
    // 源端空 = 删除候选等于整个目标端，放过去就是照着一次挂载事故删库。
    let (fx, runner, _g) = mirror("", &[], &["a.webp", "b.webp"]).await;
    let token = runner.confirm_mirror("m").await.expect("确认时还没列，判不了比例");
    let err = runner.run_mirror("m", &token).await.unwrap_err().to_string();
    assert!(err.contains("这一侧空=true"), "要说得出是哪一种形状：{err}");
    assert_eq!(files_in(&fx.dst()), vec!["a.webp", "b.webp"], "拒绝就一个都不该少");
}

#[tokio::test]
async fn a_confirmation_without_a_preview_still_refuses_a_majority_of_the_destination() {
    let (fx, runner, _g) = mirror("", &["a.webp"], &["a.webp", "b.webp", "c.webp"]).await;
    let token = runner.confirm_mirror("m").await.expect("确认时还没列，判不了比例");
    let err = runner.run_mirror("m", &token).await.unwrap_err().to_string();
    assert!(err.contains("占比超限=true"), "{err}");
    assert!(err.contains(&RATIO_DEN.to_string()), "要带阈值：{err}");
    assert_eq!(files_in(&fx.dst()).len(), 3, "拒绝就一个都不该少：{:?}", files_in(&fx.dst()));
}

#[tokio::test]
async fn a_confirmation_without_a_preview_is_spent_by_the_first_run() {
    let (fx, runner, _g) = mirror("", SHARED, &dst_side(SHARED, ORPHANS)).await;
    let token = runner.confirm_mirror("m").await.unwrap();
    runner.run_mirror("m", &token).await.unwrap();
    let again = runner.run_mirror("m", &token).await.unwrap_err().to_string();
    assert!(again.contains("不作数"), "同一份确认不能授权第二次实删：{again}");
    // 第二次被拒之后，盘上还是第一次跑完的那个形状（没有"又删了一遍"）。
    assert_eq!(files_in(&fx.dst()), vec!["s1.webp", "s2.webp", "s3.webp"]);
}

#[tokio::test]
async fn a_mirror_confirmation_never_drives_a_browser_action() {
    // 凭证只认"刚才看的是哪件事"（R78）。对齐那份"没带名单的确认"尤其不许被浏览器那五个
    // 动手入口拿去用——那五个的删除范围**就是**那份名单，没有名单就等于整棵都算。
    let (fx, runner, _g) = mirror("", SHARED, &dst_side(SHARED, ORPHANS)).await;
    let token = runner.confirm_mirror("m").await.unwrap();
    let err = runner.browser_run(&token).await.unwrap_err().to_string();
    assert!(err.contains("没带清单"), "要指出的是这份凭证根本没有名单可比：{err}");
    assert_eq!(files_in(&fx.dst()), vec!["gone.webp", "s1.webp", "s2.webp", "s3.webp", "stale.webp"],
        "浏览器那一步一个东西都不该动（`files_in` 交的是排序后的名字）");
    // 而且它已经被吃掉：拿同一份去走正路也只能撞见"不作数"。
    let later = runner.run_mirror("m", &token).await.unwrap_err().to_string();
    assert!(later.contains("不作数"), "{later}");
}
