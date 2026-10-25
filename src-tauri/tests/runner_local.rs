#![cfg(windows)]

// 起 daemon 的夹具（守卫、`Fx`、`setup*`、`job`、`PORT`）在 `tests/localdaemon/mod.rs`：
// 四条起真 daemon 的目标（本文件、`mirror_runner`、`browser_runner`、`draft_pairs`）共用这一份夹具
// 与同一份端口账；规则（凡能留下活 daemon 的用例都必须持有守卫）写在那份文件顶部。
// 留在本文件的是只有它用得着的两段式夹具（冻结日志、接管端口）与它们各自的证人。

pub mod localdaemon;
use localdaemon::*;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rclone_deck_lib::config::jobs::{Job, JobKind, Manifest};
use rclone_deck_lib::runner::{transfer_invariant_violation, RunError, Runner, TaskState};
use rclone_deck_lib::scope::Scope;

/// 把这次任务自己那份日志变成写不下去。`TaskLog::write` 每行都
/// `OpenOptions::append(true).open(path)`，Windows 上对只读文件开写就是 ACCESS_DENIED——
/// 这是 R48 I-1 说的"盘满 / ACL / 路径过长"那一类在本机唯一既不需要管理员权限、也不需要
/// 填满磁盘的写法（属性挂在文件上，随 TempDir 一起消失，不改目录 ACL、不碰任何共享路径）。
///
/// 先等"已提交"那一行**真的进了日志**再冻结：不然写不下去的可能是它**前面**某一条，
/// 那落在"job 还没提交成功"的区间里，放开锁本来就是对的——这条证人要钉的是
/// Unknown 那一臂自己的顺序，所以必须让第一次失败落在那一臂里。
async fn freeze_task_log(r: &Runner, id: &str) -> PathBuf {
    let path = r.last_log(id).await.expect("open_log 在提交之前就登记了日志路径");
    for _ in 0..2000 {
        if rclone_deck_lib::runner::tasklog::TaskLog::tail(&path, 4096)
            .map(|t| t.contains("已提交")).unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut perm = std::fs::metadata(&path)
        .unwrap_or_else(|e| panic!("读 {} 的属性失败：{e}", path.display()))
        .permissions();
    // `set_readonly` 是 `std::fs::Permissions` 的固有方法（Windows 上没有 PermissionsExt 那个
    // Unix trait），它翻的是文件属性 FILE_ATTRIBUTE_READONLY，不改目录 ACL、不碰共享路径。
    perm.set_readonly(true);
    std::fs::set_permissions(&path, perm)
        .unwrap_or_else(|e| panic!("把 {} 设成只读失败：{e}", path.display()));
    path
}

/// 裁定 R49 证人 (a) 的夹具：把"第一笔写不下去"**挪到"已提交"那一行**，而不是落在 R48 那条
/// 已经 state-first 的那条 Unknown 臂上。做法是抢一个更早的时间窗：状态一推进 `Queued` 就冻结——
/// 那一刻「任务开始」已经落盘（它排在「已提交」之前），提交请求还在飞，「已提交」那一行还没写。
///
/// 为什么这里是 `yield_now` 忙轮询而不是像 `freeze_task_log` 那样睡固定毫秒：本测试跑在
/// `#[tokio::test]` 的单线程 runtime 上，`transfer` 只在 submit 的那次 `await` 让出，而 `Queued`
/// 正是在那次让出**之前**写进去的——所以并发的本 future 一定在那次让出里读到 `Queued`，抢在
/// "已提交"之前把文件设成只读。睡固定时长反而会赌 submit 比睡眠久，本地回环的 submit 可能更快，
/// 那就成了一条会**偶尔绿**的测试（比没有测试更坏，见 R48 那段的同款教训）。
async fn freeze_log_at_queued(r: &Runner, id: &str) -> PathBuf {
    // open_log 把路径登记在 `transfer` 之前，先等它出现，再抢"已提交"前的窗口。
    let path = loop {
        if let Some(p) = r.last_log(id).await { break p; }
        tokio::task::yield_now().await;
    };
    // 有界只是防止夹具本身死循环（真任务必然经过 Queued），不是用来抢时序的。
    for _ in 0..2_000_000u64 {
        if matches!(r.state(id).await, TaskState::Queued) { break; }
        tokio::task::yield_now().await;
    }
    assert!(matches!(r.state(id).await, TaskState::Queued),
        "没抢到 Queued 窗口，夹具不成立：实际 {:?}", r.state(id).await);
    let mut perm = std::fs::metadata(&path)
        .unwrap_or_else(|e| panic!("读 {} 的属性失败：{e}", path.display()))
        .permissions();
    // 只读属性随 TempDir 一起消失，不改目录 ACL、不碰共享路径（同上 `freeze_task_log`）。
    perm.set_readonly(true);
    std::fs::set_permissions(&path, perm)
        .unwrap_or_else(|e| panic!("把 {} 设成只读失败：{e}", path.display()));
    path
}

/// 本用例的 daemon 死了之后，把**同一个端口**接管过来。必须先 bind 再让 runner 发请求：
/// 反过来写的话超时的 SYN 落在"还没有监听者"的窗口里被拒，reqwest 拿到 Transport，
/// 而 `accept()` 永远等不到第二个连接——那是夹具死锁，不是被测行为。
/// `kill_daemon` 等到的是"进程从进程表里消失"，监听 socket 的关闭还差半步，所以这里重试。
async fn take_over_the_dead_port() -> tokio::net::TcpListener {
    let mut last: Option<std::io::Error> = None;
    for _ in 0..50 {
        match tokio::net::TcpListener::bind(("127.0.0.1", *PORT)).await {
            Ok(l) => return l,
            Err(e) => last = Some(e),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("端口 {} 一直没空出来：{}", *PORT, last.expect("循环里必然记过一次失败"));
}

/// 在已经占住的端口上答**一次** 400。回的是固定字面量，**不回显请求**（读到的字节直接丢掉），
/// 所以凭据不可能经由错误话术回流。监听者随本函数落地，端口随即空出来。
async fn answer_once_with_a_400(l: tokio::net::TcpListener) {
    let (mut sock, _) = l.accept().await.expect("runner 的提交会连上来");
    let mut buf = [0u8; 4096];
    let mut seen = Vec::new();
    while let Ok(n) = tokio::io::AsyncReadExt::read(&mut sock, &mut buf).await {
        if n == 0 { break; }
        seen.extend_from_slice(&buf[..n]);
        // 等头部结束再答：半程断开会让 reqwest 拿到 Transport，那就变成另一档了。
        if seen.windows(4).any(|w| w == b"\r\n\r\n") { break; }
    }
    let body = "{\"error\":\"一个不是 rclone 的监听者拒了这个请求\"}";
    let resp = format!(
        "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\
         \r\nconnection: close\r\n\r\n{}", body.len(), body);
    tokio::io::AsyncWriteExt::write_all(&mut sock, resp.as_bytes()).await
        .expect("把 400 写回去");
    drop(sock);
}

// --------------------------------------------------------------- 纯函数

#[test]
fn the_invariant_flags_any_deletion_reported_under_a_copy_kind() {
    let mut s = empty_stats();
    s.deletes = 1;
    let v = transfer_invariant_violation(&JobKind::Copy, &s, None).expect("必须报出来");
    // 读数留在排障那一份里（界面上点「为什么？」才看到）
    assert!(v.detail.contains("deletes=1"), "{v:?}");
    // 而**明面那半句不许带读数**：`deletes=` 是内部词，操作员读不出它的方向
    assert!(!v.short.contains("deletes="), "违约那句的摘要里还有内部读数：{}", v.short);
    assert!(!v.short.is_empty(), "{v:?}");
}

#[test]
fn the_invariant_flags_deleted_dirs_too() {
    let mut s = empty_stats();
    s.deleted_dirs = 2;
    assert!(transfer_invariant_violation(&JobKind::Copy, &s, None).is_some());
}

#[test]
fn the_invariant_accepts_a_clean_copy() {
    assert!(transfer_invariant_violation(&JobKind::Copy, &empty_stats(), None).is_none());
}

#[test]
fn the_invariant_lets_sync_delete_when_nothing_was_previewed() {
    let mut s = empty_stats();
    s.deletes = 5;
    // 不带预览数（copy 那一路、或老状态）时，sync 删东西是它的语义，不判违约。
    assert!(transfer_invariant_violation(&JobKind::Sync, &s, None).is_none());
}

#[test]
fn the_invariant_also_catches_a_delete_count_that_disagrees_with_the_preview() {
    // 对齐的删除范围是 rclone 自己算的；面板能担保的只有"两次列表一致"。
    // 这一条把那句担保从注释变成有人听的结果。
    let mut s = empty_stats();
    s.deletes = 5;
    // copy 那一档的老判据不许漂
    assert!(transfer_invariant_violation(&JobKind::Copy, &s, None).is_some());
    assert!(transfer_invariant_violation(&JobKind::Copy, &empty_stats(), None).is_none());
    // sync：预览说 3 个、实删 5 个 = 违反"你看到的就是被删的"
    let v = transfer_invariant_violation(&JobKind::Sync, &s, Some(3)).expect("必须报出来");
    // 两个数都要在**排障那一份**里出现（少了任何一个，这一句就没法核对）
    assert!(v.detail.contains("3") && v.detail.contains("5"), "{v:?}");
    assert!(!v.short.contains("deletes="), "违约那句的摘要里还有内部读数：{}", v.short);
    // 对得上就安静
    assert!(transfer_invariant_violation(&JobKind::Sync, &s, Some(5)).is_none());
    // **预览数缺失不等于 0**：老状态、或不是走对齐进来的，都不能判成"预览说 0 却删了 5"
    assert!(transfer_invariant_violation(&JobKind::Sync, &s, None).is_none());
}

#[test]
fn the_two_directions_are_the_same_set_of_trees() {
    // 取代 `reverse_of_swaps_the_two_sides`：那个函数连同"反向是这把锁的一种状态"一起删了，
    // 配对不再靠把方向串倒过来读，而靠**两侧树键的集合相等**（裁定 R71 / I3）。
    // 这一条必须单独钉住，不许并进下面那条端到端用例：集合一旦不等（`scope_keys` 少了一侧），
    // "反向被拒"就再也拿不到冲突，那条端到端会安静地换一种别的原因红或绿，说不清是哪一侧没了。
    let fx = Fx::new();
    let up = job("up", JobKind::Copy, &fx.src(), &fx.dst());
    let down = job("down", JobKind::Copy, &fx.dst(), &fx.src());
    let mut a = up.scope_keys();
    let mut b = down.scope_keys();
    a.sort();
    b.sort();
    assert_eq!(a, b, "同一对目录的两个方向必须交出同一组键，否则反向任务挡不住");
    // 一组里是**两把**：两侧各一棵树，不许在归一里并成一把（src 与 dst 是两个位置）。
    assert_eq!(a.len(), 2, "两侧各一把键，实际 {a:?}");
    assert_ne!(a[0], a[1], "src 与 dst 归一成了同一把键：{a:?}");
}

// --------------------------------------------------------------- copy / sync 的删除不变量

#[tokio::test]
async fn copy_into_a_directory_holding_an_orphan_deletes_nothing() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"new").unwrap();
    std::fs::write(fx.dst().join("orphan.txt"), b"pre-existing").unwrap();

    let (r, _g) = setup(fx.root(), vec![job("c", JobKind::Copy, &fx.src(), &fx.dst())]).await;
    let stats = stats_of(r.run_copy("c", false).await.unwrap());

    assert_eq!(stats.deletes, 0, "copy 报告了删除：{stats:?}");
    assert_eq!(stats.deleted_dirs, 0);
    assert!(fx.dst().join("orphan.txt").exists(),
        "孤儿被删了——这就是原来那条 rclone sync 的行为，本项目就是为了不再这样");
    assert_eq!(std::fs::read(fx.dst().join("a.txt")).unwrap(), b"new");
    r.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn the_same_fixture_under_sync_kind_reports_the_orphan_as_a_delete_in_the_trial_run() {
    // 上一条的常驻"故意破坏"版：同一套夹具换成 sync，试跑必须说出它要删掉那个孤儿——
    // 否则上一条的 `deletes == 0` 就退化成恒真断言。
    //
    // 为什么从"真删一个孤儿"改成"试跑报出这个删除"（裁定 R60 的那道闸）：
    // `run_copy(..., false)` 对对齐任务在第一行就被拒，界面上能真删对齐的只有
    // 「让两边对齐」→确认那一条（`Runner::run_mirror`，它只认预览交回来的 token，
    // 那一路的端到端证人在 `tests/mirror_runner.rs`），这一档从 `run_copy` 还能跑的
    // 布尔只剩 `dry_run = true`。计数是 rclone 自己算的，
    // 所以"这对夹具里确实有一个 sync 会删的东西"照样被证住；而试跑一个东西都不删，
    // 那条不变量由下面那个断言现场看着。
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"new").unwrap();
    std::fs::write(fx.dst().join("orphan.txt"), b"pre-existing").unwrap();

    let (r, _g) = setup(fx.root(), vec![job("s", JobKind::Sync, &fx.src(), &fx.dst())]).await;
    let stats = stats_of(r.run_copy("s", true).await.unwrap());

    assert_eq!(stats.deletes, 1, "sync 的试跑没把那个孤儿算进删除清单：{stats:?}");
    assert!(fx.dst().join("orphan.txt").exists(), "试跑把东西真删了：dryRun 没落到 rclone");
    r.shutdown_rcd().await.unwrap();
}

/// 被拒的实删**不许留下任何副作用**（那道闸门装在 `run_copy` 的第一件事，R60）。
/// 这一条要钉的不是话术（`tests/commands_surface.rs` 钉那个），是顺序。
///
/// **闸门排在 `acquire` 之前**这件事是这么证真的：先手动占住这一对目录的整组键（两侧各一把），再让 `run_copy` 跑。
/// 若闸门排在 `acquire` 之后，`run_copy` 会先申领这组键、撞上"手动占用"而返回 `Busy`；它返回的却是
/// `Invariant`，才说明 `acquire` 根本没被执行到。单看结束后的 `locks()` 是钉不住这条的——
/// 键随 guard 落地即释放，"跑过一次再放开"和"从没跑过"在终态里一模一样，所以这里靠预先占键把它变成可观测。
/// 排在 `open_log` 之前则由"日志目录里没多出这个任务的文件"来钉；被拒的动作若建了日志文件，
/// 那目录里就会躺着一条"这个任务从没跑过"的文件。
///
/// "根本没碰 daemon" 是这么证的：把本用例自己的 daemon 杀掉、端口由测试接管成一个
/// 只会记账的监听者，`run_copy` 返回之后再问它收没收到过连接（同一个两段式夹具
/// 见 `a_submit_that_rcd_answered_releases_the_key_and_leaves_queued`）。
#[tokio::test]
async fn a_refused_real_mirror_touches_neither_the_lock_nor_the_log_nor_the_daemon() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"new").unwrap();
    std::fs::write(fx.dst().join("orphan.txt"), b"pre-existing").unwrap();
    let mirror = Job {
        id: "m".into(),
        name: "镜像 decklocal：以本地为主（删云端多余的）".into(),
        kind: JobKind::Sync,
        src: fwd(&fx.src()),
        dst: fwd(&fx.dst()),
        open_dir: None,
        manifest: None,
    };
    let (r, g) = setup(fx.root(), vec![mirror]).await;
    // 预先占住这对目录的**整组**键（两侧各一把）：闸门若在 `acquire` 之后就会在这里拿到 `Busy` 而不是 `Invariant`。
    let held = r.mutex().acquire_all(&keys_of(&r, "m"), "手动占用").await.unwrap();
    g.kill_daemon();
    // 僵尸连接那一段同 `a_submit_that_rcd_answered_releases_the_key_and_leaves_queued`：
    // 不等这一圈，提交可能落进健康检查留下的那条池化连接里，端口上就什么都看不见。
    tokio::time::sleep(Duration::from_millis(200)).await;
    let l = take_over_the_dead_port().await;

    let e = match r.run_copy("m", false).await {
        Err(e) => e,
        Ok(other) => panic!("镜像的实删本该被拒，实际跑成了 {other:?}"),
    };
    let msg = e.to_string();
    // 键被人占着却仍返回 `Invariant`（不是 `Busy`）：这就是"闸门排在 `acquire` 之前"的可观测证据。
    assert!(matches!(e, RunError::Invariant(_)), "拒绝被分到了另一档（键明明被占着）：{msg}");
    assert!(msg.contains("镜像"), "被拒的那句话没点名这是镜像任务：{msg}");
    assert!(matches!(r.state("m").await, TaskState::Idle),
        "被拒的动作把状态推进了 {:?}：界面会以为它真在跑", r.state("m").await);
    // 放开预先占的那把键，再验被拒的动作自己没留任何登记。
    drop(held);
    assert!(r.locks().await.is_empty(),
        "被拒的动作碰了那一队目录的锁：{:?}", r.locks().await.keys().collect::<Vec<_>>());
    let logs = fx.root().join("logs");
    let left: Vec<String> = if logs.exists() {
        std::fs::read_dir(&logs).unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect()
    } else {
        vec![]
    };
    assert!(left.is_empty(), "被拒的动作建了日志文件（它从没跑过）：{left:?}");
    assert!(fx.dst().join("orphan.txt").exists(), "被拒的实删真删了东西");
    assert!(tokio::time::timeout(Duration::from_millis(500), l.accept()).await.is_err(),
        "被拒的实删还是去敲了 daemon 的端口：提交根本没该发生");
}

#[tokio::test]
async fn dry_run_leaves_the_destination_untouched() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"new").unwrap();

    let (r, _g) = setup(fx.root(), vec![job("c", JobKind::Copy, &fx.src(), &fx.dst())]).await;
    let st = r.run_copy("c", true).await.unwrap();

    assert!(!fx.dst().join("a.txt").exists(), "dry-run 居然真写了文件");
    match st {
        TaskState::Done { dry_run: true, .. } => {}
        other => panic!("期望 Done{{dry_run:true}}，实际 {other:?}"),
    }
    // dry-run 下 core/stats 的计数口径没实测过，所以这里只断言真正要守的东西：目标没被动过。
    r.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_failed_transfer_surfaces_rclone_own_error_text() {
    let fx = Fx::new();
    let missing = fx.root().join("does-not-exist");
    let (r, _g) = setup(fx.root(), vec![job("c", JobKind::Copy, &missing, &fx.dst())]).await;

    match r.run_copy("c", false).await {
        Ok(TaskState::Failed { error, .. }) => assert!(!error.is_empty()),
        // rclone 也可能在提交阶段就 400，两条路都算"把话说清楚了"
        Err(e) => assert!(!e.to_string().is_empty(), "{e}"),
        Ok(other) => panic!("源目录不存在却跑成功了：{other:?}"),
    }
    r.shutdown_rcd().await.unwrap();
}

// --------------------------------------------------------------- 互斥

#[tokio::test]
async fn the_reverse_direction_is_refused_while_the_other_one_holds_the_lock() {
    // 判据换了形状，行为一个字没改（spec I3 第 2 条）：原来占的是 `down` 的**方向串**、
    // 靠 `acquire` 里那步"再查一次反向键"把 `up` 挡下来；现在占的是 `up` 的**整组树键**
    // （两侧各一把），被挡的是同一棵树上的另一次写——也就是反方向那条任务。
    // 用例名留着说的是场景（另一条任务在同一批键被人占着期间跑不起来）；机制侧不再有"反向键"这个东西，
    // 拒绝全部来自键集合本身，所以 `owner` 里也没有 `（反方向正在跑）` 那半句了。
    let fx = Fx::new();
    let up = job("up", JobKind::Copy, &fx.src(), &fx.dst());
    let down = job("down", JobKind::Copy, &fx.dst(), &fx.src());
    let (r, _g) = setup(fx.root(), vec![up, down]).await;

    let held = r.mutex().acquire_all(&keys_of(&r, "up"), "手动占用").await.unwrap();
    match r.run_copy("down", false).await {
        Err(RunError::Busy { owner }) => assert!(owner.contains("手动占用"), "{owner}"),
        other => panic!("同一棵树被人占着，反向任务却拿到了执行权：{other:?}"),
    }
    drop(held);

    assert!(matches!(r.run_copy("down", false).await, Ok(TaskState::Done { .. })),
        "锁释放之后就该能跑");
    r.shutdown_rcd().await.unwrap();
}

/// 键里**两侧都在**：只占住源端那一把，正向任务就该拿不到执行权。
///
/// 为什么单独一条、还故意只占一把：占整组的话，实现里把源端漏掉也照样撞在目标端上，
/// 那几条只说"被占了"，说不出漏的是哪一侧。这一条说的是**哪一侧**——
/// Task 7 的破坏 B1（`acquire` 的键里去掉源端）里，"源端"这一格只有这一条看得见
///（同一趟还有四条未知结局的证人红，但它们红的是"少了一把在表里"，不是"反向挡不住"）。
/// 两侧都进键的理由写在 `Runner::acquire` 头上：对齐动手前要重列两侧，
/// 源被并发改写的话它核对的就不是它将要写的那份东西。
///
/// `keys.first()` 取的是源端：`Job::scope_keys()` 的顺序是 `endpoint_src()` 再 `endpoint_dst()`
/// （`config/jobs.rs`），改那个顺序要连带改这里。
#[tokio::test]
async fn the_source_side_is_one_of_the_keys_a_run_has_to_take() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let (r, _g) = setup(fx.root(), vec![job("up", JobKind::Copy, &fx.src(), &fx.dst())]).await;

    let keys = keys_of(&r, "up");
    assert_eq!(keys.len(), 2, "两侧各一把键，实际 {keys:?}");
    let held = r.mutex().try_acquire(keys.first().expect("上面刚断过是两把"), "手动占用").await.unwrap();
    match r.run_copy("up", false).await {
        Err(RunError::Busy { owner }) => assert!(owner.contains("手动占用"), "{owner}"),
        other => panic!("只占住源端就挡不住了——`acquire` 的键里没有源端那一侧：{other:?}"),
    }
    drop(held);

    assert!(matches!(r.run_copy("up", false).await, Ok(TaskState::Done { .. })),
        "放开源端那把键之后就该能跑");
    r.shutdown_rcd().await.unwrap();
}

/// 裁定 R90 在这一层的说法：**一次写动作占住的是一棵树，不是登记表的某一行**。
/// 形状照 Task 7 落地当天抓到的缺陷写：对齐占着它的 `dst` 整棵（夹具那份 conf 里的
/// `decklocal:` → 键 `decklocal:#`），另一次写要动的是 `decklocal:sub`——同一棵树里再往里一层。
/// 纯等号的锁看不见这一格（两把键的串不等），于是对齐与浏览器同时落进同一棵树，
/// 起点事故换一层目录就回来了。这一条就是"锁必须 Scope 感知"在 daemon 侧的落点。
///
/// 浏览器代码要等 Task 8/9 才落地，所以两侧都直接用 `runner.mutex()`：
/// 持有方交的是**这条对齐自己那一对** `Job::write_scopes()`（生产形状），
/// 被挡的那一句既走 `run_copy`（走的就是 `Runner::acquire` 那条真路径），
/// 也照字面用 `Scope::remote("decklocal:", "sub")` 再申一次（说清挡它的是**树**、不是任务）。
/// 反面半边同样钉住：另一棵不相干的树必须照常能跑——锁过宽在这一格也是缺陷（I3 那句"互相假死"）。
///
/// 裁定 R92 把**读侧**也并进同一条判据：`Runner::blocking_owner(&job)` 就是 `commands.rs::get_jobs`
/// 那句 `locked_by` 的问法，这一条顺手把它的生产形状钉在这里（`nested` 的键一把都不在表里、
/// 树却在别人占着的父层里 ⇒ 读侧必须报出人；`unrelated` 必须报 `None`）。
/// 界面层（`JobView.lockedBy` → 「正在跑：谁」）没有夹具，见报告里的缺口那节。
///
/// 破坏（都实跑过，坐标在 `docs/superpowers/evidence/build-notes.md` 那张表里）。
/// **读侧那两句排在拒绝那两句之前**，所以每枪先红的都是读数；`run_copy` 那两句在同一次运行里
/// 看不见（libtest 一条用例只报第一个 panic），它们在 R90 那一轮各自量过（同一份判据、同一格）。
/// - `conflicts_with` 的 `(Some(h), Some(r))` 退回 `held_key == req_key`（R90 的 B1，Task 7 修复轮重跑）→
///   本条红在「nested 这一行被显示成没锁…」`left: None`、`right: Some("手动占用")`；那句
///   「下层拿到了执行权…Ok(Done { … transfers: 1 … })」排在它后面（等号之下 `run_copy("nested")`
///   真会跑成一次传输，那是 R90 的 B1 行量过的形状）；
/// - `Scope::conflicts` 松成"同根即冲突"（删掉 prefix 那半边，R90 的 B7，Task 7 修复轮重跑了全部八个目标）→
///   本条红在「另一棵不相干的树不许被读成'有人占着'」`left: Some("手动占用")`；
///   「锁过宽（I3 说的互相假死）」那句排在它后面，同为判据过宽在另一个层次上的落点。
/// - `blocking_owner` 的判据**单独**换成键等号（裁定 R92 要修的只有读侧那一半）→ 只有第一句红，
///   `run_copy` 那三句照旧绿（实测：只有这一句红，本文件其余全绿）：**拒绝侧是好的、坏的是读数**。
///
/// 要说清一条**这一条量不到**的：`Job::write_scopes()` 少交一侧（比如只交目标端）时本条**照旧绿**——
/// `held.keys()` 与 `want` 同源于那个被测函数，一起缩小就自洽（Task 7 的 B3 在 `mirror_runner` 上
/// 记过同一形状），所以那句断言只说"登记的条数与顺序"，不说"哪两侧进了集合"。那一格由
/// `the_two_directions_are_the_same_set_of_trees`（断"一组里正好两把"）与 `endpoint.rs` 的配对证人钉住。
/// 读侧那两句同样量不到它：`blocking_owner(&nested)` 要的是**父层那条登记**被看见，
/// 对齐少交一侧时它撞的还是另一侧那棵，读数不变。
#[tokio::test]
async fn a_held_tree_blocks_the_tree_inside_it_but_not_a_neighbour() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    // `other-*` 那两个目录与 `src`/`dst` 是兄弟不是子层：它们与对齐那两棵树都不相干。
    // 这个区分不是装饰：`Scope::local()` 的根只到盘符（`c:`），整台机器的本地目录都在同一根下，
    // 祖先判定必须自己在 prefix 那一层把兄弟分开。
    let other_src = fx.root().join("other-src");
    let other_dst = fx.root().join("other-dst");
    std::fs::create_dir_all(&other_src).unwrap();
    std::fs::write(other_src.join("b.txt"), b"x").unwrap();

    let mirror = job_at("m", JobKind::Sync, &fwd(&fx.src()), &fx.remote(""));
    let unrelated = job_at("unrelated", JobKind::Copy, &fwd(&other_src), &fwd(&other_dst));
    // 只有 dst 落进对齐那棵树里（`decklocal:sub` 是 `decklocal:` 的下层），src 在另一棵上。
    // 所以这一跑被挡**只能**是因为父子冲突：等号、源端、目标端字面串那三条都凑不出这个 Busy。
    let nested = job_at("nested", JobKind::Copy, &fwd(&other_src), &fx.remote("sub"));
    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![mirror.clone(), unrelated.clone(), nested.clone()],
    ).await;

    let held = r.mutex().acquire_scopes(&mirror.write_scopes(), "手动占用").await.unwrap();
    let mut want: Vec<String> = mirror.write_scopes().iter().map(|s| s.key()).collect();
    want.sort();
    assert_eq!(held.keys(), want,
        "交进去的这几棵**互不包住**的树，一棵登记一条、且 keys() 是按键排过序的（父子同批会被折到祖先，\
         那是 `job_mutex.rs` 的 `a_batch_with_both_layers_of_one_tree_registers_the_ancestor_only` 钉的格子；\
         这一句两侧同源于 `write_scopes()`，它钉的是\"登记的条数与顺序\"；**哪两侧进集合**由 \
         `the_two_directions_are_the_same_set_of_trees`与 `endpoint.rs` 的配对证人钉，见本条头上那句\"这一条量不到\"）");

    // 读侧那一格（裁定 R92）：`commands.rs::get_jobs` 问的就是这个查询，这里问的是同一个方法。
    // `nested` 的**两把键都不在表里**（表里是对齐那两棵），要写的那棵树却整个躺在对齐占着的父层里
    // ⇒ 原来那句按键等号查表报 `None`：界面上"没锁"、按钮亮着、点下去被 `Busy` 拒。
    // 读侧与拒绝侧必须由同一条判据答话，这一句钉的就是"同一条"。
    assert_eq!(r.blocking_owner(&nested).as_deref(), Some("手动占用"),
        "nested 这一行被显示成没锁，而它马上会被拒——读侧还在按键等号查表");
    assert_eq!(r.blocking_owner(&unrelated).as_deref(), None,
        "另一棵不相干的树不许被读成'有人占着'：读侧过宽会拦住一次本来能跑的动作");

    // 反面半边：不相干的那棵树照常跑完（这一次是真提交、真跑，挡它就该红）。
    assert!(matches!(r.run_copy("unrelated", false).await, Ok(TaskState::Done { .. })),
        "镜像占着自己那两棵树，却把另一棵树上的写也挡了：锁过宽（I3 说的互相假死）");

    // 正面半边 A：`Runner::acquire` 那条真路径。
    match r.run_copy("nested", false).await {
        Err(RunError::Busy { owner }) => assert!(owner.contains("手动占用"), "{owner}"),
        other => panic!("同一棵树里的下层拿到了执行权——锁比的还是键的等号，不是 Scope：{other:?}"),
    }
    // 正面半边 B：同一件事用 `Scope::remote` 直接说一遍，不经过任何任务定义。
    let err = r.mutex().acquire_scopes(&[Scope::remote("decklocal:", "sub")], "浏览器（Task 8 才有的那一方）")
        .await
        .expect_err("镜像占着整棵树时，下层前缀的申领该被拒");
    assert_eq!(err.owner, "手动占用", "被拒要说的是占着这棵树的人");

    drop(held);
    assert!(matches!(r.run_copy("nested", false).await, Ok(TaskState::Done { .. })),
        "放开那棵树之后下层就该能跑：Busy 不是从别处（路径写错、daemon 故障）来的");
    r.shutdown_rcd().await.unwrap();
}

/// R33b 的正面半边：guard 是**交进 `transfer` 里**的，所以任务正常跑完时锁必须已经放开。
/// 这条看着像上面的重复，其实钉的是另一件事：`transfer` 若在结束时把 guard 存起来或提前丢掉，
/// 前者会让键永久占用（面板从此跑不动这一对目录），后者会让作业还在跑就放开（起点事故）。
/// 两个方向都只有跑一次真任务才看得见，所以它必须在 runner 这一层，不在 Task 6 的单元测试里。
#[tokio::test]
async fn a_finished_transfer_leaves_no_registration_behind() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.webp"), b"x").unwrap();
    let (r, _g) = setup(fx.root(), vec![job("up", JobKind::Copy, &fx.src(), &fx.dst())]).await;

    assert!(matches!(r.run_copy("up", false).await, Ok(TaskState::Done { .. })));
    assert!(r.locks().await.is_empty(), "跑完了登记表还有一条：{:?}", r.locks().await);

    // 再跑一次同一对目录：拿得到锁才说明第一次真的放开了（不是靠 force_release 抹掉的）。
    assert!(matches!(r.run_copy("up", false).await, Ok(TaskState::Done { .. })),
        "同一把键在任务结束后必须可以再次申领");
    r.shutdown_rcd().await.unwrap();
}

/// **R33c 的那句"架构缺缝、无法执行"已经被 R48 改掉**：缝一直在，只是当时没去找——
/// `test-helpers` 交出 `kill_pid`，而本文件的守卫又存着**本用例自己那个 daemon 的 PID**，
/// 于是"提交成功之后把 daemon 杀掉"这个两段式夹具不需要任何新的生产口子。
/// 证人：[`a_job_whose_status_never_answers_after_submit_keeps_the_key_held`]。
/// 台账/验收里"未执行：架构缺缝"那一行，从这一条落地起才许改写成实执行（R48 末段）。
///
/// 当时试过而不合格的两条仍然留着，免得下一个人再试一遍：
/// - 先杀 rcd 再跑 → 连提交一起失败，拿到的是 `Err(Rc)`/`Transport`，走的是 R48 item 2 那一档
///   （那一档另有证人：[`a_submit_with_no_answer_holds_the_key_and_names_the_consequence`]）；
/// - 靠"任务跑得够慢"抢时序 → 本地对本地在第一次轮询（+500ms）之前就跑完，
///   结果取决于时序，是一条会**偶尔绿**的测试，比没有测试更坏。
///   现在这条不等任务：等的是**状态推到 `Running` 那一刻**（提交已返回），杀伐落在 500ms 窗口里。
///
/// 仍然**不许**用"手工往 `abandoned` 塞一个 guard"糊过去——那测的是字典不是行为（R48 原话）。
#[tokio::test]
async fn a_job_whose_status_never_answers_after_submit_keeps_the_key_held() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let up = job("up", JobKind::Copy, &fx.src(), &fx.dst());
    let down = job("down", JobKind::Copy, &fx.dst(), &fx.src());
    let (r, g) = setup(fx.root(), vec![up, down]).await;

    let (res, ()) = tokio::join!(
        r.run_copy("up", false),
        async {
            wait_until_running(&r, "up").await;
            g.kill_daemon();
        }
    );
    let reason = match res
        .expect("状态未知是一条 Ok 的路：job 没答上来不等于 runner 这次调用失败")
    {
        TaskState::Unknown { reason } => reason,
        other => panic!("连续三次读不到 job/status 应该走 Unknown，实际 {other:?}"),
    };
    assert!(reason.contains("job/status"), "话术没交代是哪一次读失败：{reason}");

    // R33 真正要防的那一半，从此有端到端证据而不是靠文档不变量撑着。
    // 打印的只有键（两侧树的键）和持有者名（就是任务名），凭据不在这里面。
    // 逐条点名：少锁一把就红，而且红的那一行说的是哪一侧漏了（整组键都要活到重启/force_release）。
    for key in keys_of(&r, "up") {
        assert_eq!(
            r.locks().await.get(&key).map(String::as_str),
            Some("up"),
            "状态未知的任务把 {key} 这把键放开了——同一棵树上的下一次写就能开跑"
        );
    }
    match r.run_copy("down", false).await {
        // 被拒时说出的是占着这棵树的那条任务名（`up`）：原来那句里"反方向"是 `acquire` 现加的
        // 后缀，现在这句话全部来自登记表本身——反向被拒不再是锁的一种状态，而是同一棵树被人占着。
        Err(RunError::Busy { owner }) => assert!(owner.contains("up"), "被拒却没说出占树的是谁：{owner}"),
        other => panic!("键没锁住，反向任务拿到了执行权：{other:?}"),
    }
    // 这里**不**断言 `fx.dst()` 里有没有那个文件：本地对本地的那一趟很可能在杀 daemon 之前
    // 就真跑完了（实测就是如此），而 runner 没能力确证它跑完——所以它按 R33 的话"没确证就不放开"。
    // 把目录状态钉进这条用例，钉的就不再是这把锁的语义了。
    // 这条用例也不调 `shutdown_rcd`：daemon 已经被它自己按 PID 收掉了。
}

/// R48 item 1 的证人：**日志写不下去不是放开这把锁的理由**。
/// 夹具 = 提交成功之后 ① 把这次任务的日志文件设成只读，② 杀掉 daemon（让状态读全失败，
/// 走进 Unknown 那条臂）。坏掉的形状正是"两条日志写排在 `abandoned.push` 之前、各带一个 `?`"：
/// 第一条 `log.kv` 一失败就 `return Err`，guard 随栈展开落地，键被放开，而 rclone 那边那个 job
/// 可能还在删。修好之后 guard 在 `Inflight` 手里，任何出口（`?`、panic、早退）都只存进 `abandoned`。
#[tokio::test]
async fn an_unwritable_task_log_after_submit_still_holds_the_key() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let up = job("up", JobKind::Copy, &fx.src(), &fx.dst());
    let down = job("down", JobKind::Copy, &fx.dst(), &fx.src());
    let (r, g) = setup(fx.root(), vec![up, down]).await;

    let (res, ()) = tokio::join!(
        r.run_copy("up", false),
        async {
            wait_until_running(&r, "up").await;
            freeze_task_log(&r, "up").await;
            g.kill_daemon();
        }
    );
    // 这一跑**返回什么不是这条用例要钉的**：日志写不下去当然要报错，job 状态也确实读不到。
    // 要钉的只有一件事——它有没有顺手把键放开。
    let _ = res;

    for key in keys_of(&r, "up") {
        assert!(r.locks().await.contains_key(&key),
            "日志目录写不下去把 {key} 放开了（这就是 R48 I-1 的形状）：{:?}",
            r.locks().await.keys().collect::<Vec<_>>());
    }
    match r.run_copy("down", false).await {
        Err(RunError::Busy { owner }) => assert!(owner.contains("up"), "被拒却没说出占树的是谁：{owner}"),
        other => panic!("键没了，反向任务拿到了执行权：{other:?}"),
    }
}

/// 裁定 R49 证人 (a)：**提交之后、"已提交"那一行日志写不下去，不许把界面冻在 `Running`**。
/// 与 [`an_unwritable_task_log_after_submit_still_holds_the_key`] 的区别就在**第一笔失败落在哪**：
/// 那条用 `freeze_task_log`（先等「已提交」进盘再冻结），首次失败于是落在「已提交」之后那条**已经
/// state-first** 的 Unknown 臂上——它把状态先写成 Unknown 再返回，界面看到的是 Unknown，缺口被它遮住。
/// 这一条改用 `freeze_log_at_queued`（在"已提交"之前就把文件设只读），首次失败就落在"已提交"这一行。
/// 坏掉的形状：那一行原本带一个 `?`，一失败就 `return Err`，`transfer` 离开循环、从此再没人轮询这个
/// job，而状态还停在 `Running { stats: None }`——一条幽灵在跑的任务，直到面板重启。
/// 修好之后（选 (a)：状态先行）：先 `set_state(Unknown{reason 点名日志})` 再返回 `Err`。
#[tokio::test]
async fn an_unwritable_submit_ack_log_goes_to_unknown_instead_of_freezing_running() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let up = job("up", JobKind::Copy, &fx.src(), &fx.dst());
    let (r, _g) = setup(fx.root(), vec![up]).await;

    let (res, ()) = tokio::join!(
        r.run_copy("up", false),
        async { freeze_log_at_queued(&r, "up").await; }
    );
    // 选 (a)：这条路**返回 Err**（日志断了要如实报错），但状态必须是 Unknown。
    assert!(res.is_err(), "'已提交'写不下去本该返回 Err，实际 {res:?}");
    match r.state("up").await {
        TaskState::Unknown { reason } => {
            assert!(reason.contains("日志"), "reason 没点名日志写不下去：{reason}");
        }
        other => panic!(
            "状态冻在 {other:?}：'已提交'那行日志写不下去却没人再轮询这个 job（R49 的靶子）"),
    }
    // 选 (a) 的代价，钉死在这里：日志目录坏 = 这一对 (src,dst) 锁到重启/force_release
    // （`Inflight` 仍握着 guard，那个 `return Err` 只走"存进 abandoned"）。
    // 整组键都在：少一把就说明存起来的不是这一次申领的那一批。
    for key in keys_of(&r, "up") {
        assert!(r.locks().await.contains_key(&key),
            "'已提交'失败却把 {key} 放开了：{:?}", r.locks().await.keys().collect::<Vec<_>>());
    }
}

/// 裁定 R49 证人 (b)：**知道结果的 terminal 臂**（这里走成功那条 → `Done`）在 `finished == true`
/// 之后确实把状态先落成真结果，日志那笔只是没落盘。断言两件事：① 状态是真结果（`Done`），
/// ② 返回的 `Err` 说的是"日志没落盘"，不是一句会被读成"任务失败"的话。
/// 夹具复用 `freeze_task_log`（先等"已提交"进盘再冻结）+ **不**杀 daemon：job 正常跑完，
/// 循环里 `retry_error` 为假、poll 也成功，于是第一笔写不下去的日志恰好是终结那行"任务结束"。
#[tokio::test]
async fn a_terminal_log_write_failure_keeps_the_real_result_and_speaks_of_the_log() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let up = job("up", JobKind::Copy, &fx.src(), &fx.dst());
    let (r, _g) = setup(fx.root(), vec![up]).await;

    let (res, ()) = tokio::join!(
        r.run_copy("up", false),
        async {
            wait_until_running(&r, "up").await;
            freeze_task_log(&r, "up").await;
        }
    );
    // 传完 + 结束日志写不下去：返回 Err（一次日志故障），但状态必须是真结果，绝不退回 Running。
    let e = match res {
        Err(e) => e,
        Ok(other) => panic!("结束日志写不下去本该返回 Err，实际 {other:?}"),
    };
    let msg = e.to_string();
    assert!(matches!(e, RunError::Io(_)), "结束日志写不下去被分到了另一档：{msg}");
    assert!(msg.contains("这一行日志没落盘"), "Err 没把话说成'只是日志断了'：{msg}");
    assert!(msg.contains("真结果已写入状态"), "Err 没交代结果已在状态里：{msg}");
    assert!(matches!(r.state("up").await, TaskState::Done { .. }),
        "结束日志写不下去把状态冻在 {:?}：R49 第 1 档要的是状态先行，界面该看到真结果",
        r.state("up").await);
    // 真结果 = Done：job 确证结束、`disarm` 已调，键应当放开（terminal 臂不锁键）。
    assert!(r.locks().await.is_empty(), "跑完了登记表还有一条：{:?}", r.locks().await);
    // daemon 这条没杀，收尾；那条只读属性挂在**文件**上，随 `TempDir` 一起消失（同 `freeze_task_log`）。
    r.shutdown_rcd().await.unwrap();
}

/// R48 item 2 的第一档：提交**没有得到答复**（`Timeout`/`Transport`）= 请求可能已经落地
/// = 那个 job 可能存在 = 结果未知。这一档既不放开键，也不许把状态冻在 `Queued`，
/// 而且话术必须把"这把键锁到哪里为止"说出来——出路只有 `force_release`，
/// 操作员看不见这句话就会以为"任务没跑成"= 反向可以跑。
#[tokio::test]
async fn a_submit_with_no_answer_holds_the_key_and_names_the_consequence() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let up = job("up", JobKind::Copy, &fx.src(), &fx.dst());
    let down = job("down", JobKind::Copy, &fx.dst(), &fx.src());
    let (r, g) = setup(fx.root(), vec![up, down]).await;
    g.kill_daemon();

    let reason = match r.run_copy("up", false).await
        .expect("没有答复的提交走的是 Unknown 这条 Ok 的路，不是 Err")
    {
        TaskState::Unknown { reason } => reason,
        other => panic!("提交没有答复应该走 Unknown，实际 {other:?}"),
    };
    assert!(reason.contains("没有得到答复"), "{reason}");
    assert!(reason.contains("force_release"), "话术没交代这把键锁到什么时候为止：{reason}");
    // 凭据只进 Basic 头、不进 URL（`rc/mod.rs` 的 `from_base_url`），所以这里的 `{e}`
    // 顶多带出 `http://127.0.0.1:<现挑的那个号>/sync/copy`；反向的断言留着当回归绊线。
    assert!(!reason.to_lowercase().contains("authorization"), "话术里出现了请求头：{reason}");
    assert!(matches!(r.state("up").await, TaskState::Unknown { .. }),
        "状态冻在 {:?}：界面会以为这个任务还在排队", r.state("up").await);

    for key in keys_of(&r, "up") {
        assert!(r.locks().await.contains_key(&key),
            "没有答复的提交把 {key} 放开了：{:?}", r.locks().await.keys().collect::<Vec<_>>());
    }
    match r.run_copy("down", false).await {
        Err(RunError::Busy { owner }) => assert!(owner.contains("up"), "被拒却没说出占树的是谁：{owner}"),
        other => panic!("键没锁住，反向任务拿到了执行权：{other:?}"),
    }
}

/// R48 item 2 的第二档：rcd **答了**一个错（HTTP 状态码 / 载荷里的 `error`）= 根本没有 job
/// 起来 = 正常放开锁，但状态**不许**冻在 `Queued`。上一条是"没有答复"，这一条是"有答复、
/// 答复是错的"——两档必须由这两个夹具分别钉住，否则实现里把两个 `Err` 写成一档也测不出来。
#[tokio::test]
async fn a_submit_that_rcd_answered_releases_the_key_and_leaves_queued() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let up = job("up", JobKind::Copy, &fx.src(), &fx.dst());
    let (r, g) = setup(fx.root(), vec![up]).await;
    g.kill_daemon();
    // 让 runtime 先转一圈再发请求：`RcdSupervisor::start` 那次健康检查在 reqwest 的连接池里
    // 留下一条 keep-alive 连接，daemon 死了它不会立刻被回收（hyper 的池驱动任务要靠 runtime
    // 轮询才发现连接关闭）。不睡这一下，那次提交会被写进那条僵尸连接、直接报 Transport，
    // 接管者永远收不到 SYN——那是夹具的坑，不是被测行为。
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 先接管端口，再让 runner 发那唯一的一次提交（顺序反了就是夹具死锁，见
    // `take_over_the_dead_port`）。
    let l = take_over_the_dead_port().await;
    let (res, ()) = tokio::time::timeout(
        Duration::from_secs(20),
        async { tokio::join!(r.run_copy("up", false), answer_once_with_a_400(l)) },
    ).await.unwrap_or_else(|_| panic!("20 秒内这次提交没和接管者完成握手：夹具不成立"));

    let e = match res {
        Err(e) => e,
        Ok(other) => panic!("一个 400 答复该走 Err（这一档没有 job 要保护），实际 {other:?}"),
    };
    assert!(matches!(e, RunError::Rc(..)), "答了的错被分到了另一档：{e}");
    // 认这句话术而不是只认"有个错"：它同时钉住 ① 这是 rcd 那一侧回的字面量（不是 reqwest
    // 的连接故障），② 错误串里没有回显、没有凭据。
    assert!(e.to_string().contains("一个不是 rclone 的监听者"), "{e}");
    assert!(r.locks().await.is_empty(),
        "rcd 答了的提交失败却把键留在表里：{:?}", r.locks().await.keys().collect::<Vec<_>>());
    assert!(matches!(r.state("up").await, TaskState::Idle),
        "状态冻在 {:?}：没有 job 起来，界面不该继续以为它会跑", r.state("up").await);
}

/// 锁的键现在是 Scope（裁定 R71）：一次动作拿两侧两把，方向不再是锁的一种状态。
/// 键**只**从 `Job::scope_keys()` 取——测试不自拼字符串：原来那对 `up_key_of`/`down_key_of`
/// 手抄了一份 `src=>dst`，与 `Job::mutex_key()` 是两份口径，实现一改就要靠"占错键那格变红"
/// 才发现，而那句红报的是夹具没占到实现的键、不是哪个语义坏了，读起来白绕一层。
/// 现在来源只有一个。
/// `up` 与 `down` 交出的是同一组（只是互换），配对由
/// [`the_two_directions_are_the_same_set_of_trees`] 钉住，不再靠 `reverse_of` 那种配对把戏。
fn keys_of(r: &Runner, id: &str) -> Vec<String> {
    r.job(id).unwrap_or_else(|| panic!("夹具里没有 {id} 这条任务")).scope_keys()
}

// --------------------------------------------------------------- gate

/// 裁定 R30：**gate / prune 的夹具必须用具名 remote 当 `dst`**，写法见 [`Fx::write_conf`]。
/// 原来这两个 helper 直接把本地临时目录拼进 `dst`，而 `actual_keys()` 走的是会报错的
/// `remote_for_list()`（Task 3 定的），于是每一条 gate / prune 测试都会在 `.unwrap()` 上炸——
/// 计划里这批测试从来没真的跑得起来。修法不是放宽 `remote_for_list()`（那是防假绿的），
/// 而是给夹具一份临时 conf + `decklocal` 这个 alias 远端。
///
/// 转换规则（对每个 gate / prune 测试都做一遍，别漏）：
/// - `prune_job(&fx, local, &fx.dst())` → `prune_job(&fx, local, &fx.remote(""))`
/// - `gate_job(...)` 里原来拼 `fwd(bucket)/ideahub/covers` 的 → `fx.remote("ideahub/covers")`
///   （远端的根就是 `fx.dst()`，所以 `ideahub/covers` 变成 remote 内的前缀而不是路径的一段）
/// - `setup(fx.root(), ...)` → `setup_with_conf(fx.root(), Some(fx.conf()), ...)`
/// - `copy` 那一组的 `dst` **不改**：它走 `sync/copy` 的 `dstFs`，本地路径是真要给 rclone 写的。
///
/// 收完必须验一遍没有漏网的：
/// Run: `grep -n 'dst: fwd(' src-tauri/tests/runner_local.rs`
/// Expected: 只剩 copy 组那几个，加上 `shared_defaults` 夹具里那一条对齐（它 dst 就是本地目录）；
/// 任何 gate 的 `dst: fwd(...)` 都是漏转换。
fn gate_job(fx: &Fx, expected: &Path) -> Job {
    Job {
        id: "g".into(), name: "闸门".into(), kind: JobKind::Gate,
        src: fwd(expected),
        dst: fx.remote("ideahub/covers"),
        open_dir: None, manifest: Some(Manifest::LocalDir),
    }
}

#[tokio::test]
async fn gate_names_exactly_the_keys_the_destination_is_missing() {
    let fx = Fx::new();
    let expected = fx.root().join("covers");
    std::fs::create_dir_all(&expected).unwrap();
    std::fs::create_dir_all(fx.dst().join("ideahub").join("covers")).unwrap();
    std::fs::write(expected.join("a.webp"), b"x").unwrap();
    std::fs::write(expected.join("b.webp"), b"x").unwrap();
    std::fs::write(fx.dst().join("ideahub").join("covers").join("a.webp"), b"x").unwrap();

    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![gate_job(&fx, &expected)],
    ).await;
    let report = r.run_gate("g").await.unwrap();

    // dst = `decklocal:ideahub/covers` ⇒ remote_for_list() = "covers"、prefix() = "covers/"，
    // 于是两侧 key 都带 `covers/`：这正是"期望侧拼的前缀"与"实际侧 rclone 自己拼的前缀"
    // 对上号的证人（对不上就会报成"两个都缺"）。
    assert!(!report.passed());
    assert_eq!(report.missing, vec!["covers/b.webp".to_string()]);
    assert_eq!(report.expected_count, 2);
    assert_eq!(report.actual_count, 1);
    assert!(!report.summary().is_empty());
    r.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn gate_passes_when_every_referenced_key_is_in_the_bucket() {
    let fx = Fx::new();
    let expected = fx.root().join("covers");
    let target = fx.dst().join("ideahub").join("covers");
    std::fs::create_dir_all(&expected).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    for name in ["a.webp", "b.webp"] {
        std::fs::write(expected.join(name), b"x").unwrap();
        std::fs::write(target.join(name), b"x").unwrap();
    }

    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![gate_job(&fx, &expected)],
    ).await;
    let report = r.run_gate("g").await.unwrap();
    assert!(report.passed(), "{report:?}");
    r.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn gate_recurses_into_subdirectories_of_the_bucket() {
    let fx = Fx::new();
    let expected = fx.root().join("covers");
    let target = fx.dst().join("ideahub").join("covers");
    std::fs::create_dir_all(expected.join("2026")).unwrap();
    std::fs::create_dir_all(target.join("2026")).unwrap();
    std::fs::write(expected.join("2026/deep.webp"), b"x").unwrap();
    // 桶里故意不放：operations/list 只列一层，不递归就会把这个 key 当成"已在桶里"

    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![gate_job(&fx, &expected)],
    ).await;
    let report = r.run_gate("g").await.unwrap();
    assert_eq!(report.missing, vec!["covers/2026/deep.webp".to_string()], "{report:?}");

    // 客户端数它实际发出的 LIST，gate 把差值写进任务日志。整棵树一次调用 → 这里必须是 1。
    // 钉住它是为了这条观测通路本身别断，同时也是一枚哨兵：数字涨回"目录数"就是有人把
    // 列举改回了逐层下探（那档成本已经用一次调用换掉了，见 `RcClient::operations_list`）。
    let log = std::fs::read_to_string(r.last_log("g").await.expect("应该有日志")).unwrap();
    assert!(log.contains("list_requests=1"), "日志里这次对账的 LIST 观测值不是 1：{log}");
    // 同一行的耗时是**下一次前缀变大时唯一会先动的数字**：界面上那两个计数只在列完之后才存在，
    // 而闸是等时间的。这条锚不求数值（数值要挂钟就会抖动），只求这一栏别静默消失。
    assert!(log.contains("list_secs="), "日志里没有列举耗时那一栏：{log}");
    r.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn gate_refuses_a_manifest_kind_that_belongs_to_subproject_two() {
    let fx = Fx::new();
    let mut j = gate_job(&fx, &fx.src());
    j.manifest = Some(Manifest::D1);
    let (r, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![j]).await;

    match r.run_gate("g").await {
        Err(RunError::Invariant(msg)) => assert!(msg.contains("子项目 2"), "{msg}"),
        other => panic!("期望 Invariant，实际 {other:?}"),
    }
    r.shutdown_rcd().await.unwrap();
}

// --------------------------------------------------------------- 盘点（R58 起只读）

// 裁定 R58：prune 的执行侧（勾选、三道确认闸、逐 key deletefile、终态 `Pruned`）已随功能
// 删除，它们的证人也一并删除——不留死证人。留下的这两条是盘点侧的：集合差的口径
// （交出去的只有"桶里有、本地没有"），以及空基准拒绝（R29 对"整个桶被当成盘点结论"的
// 硬保险；盘点只读之后它守的是"对账基准坏了却没人知道"）。`JobKind::Prune` 已不存在：
// 盘点夹具用 copy——`run_inventory` 不看 kind，形状由"src 是本地目录、dst 是 remote"
// 两端各自的报错保证。

/// `_fx` 仍然带在签名上（而不是删掉这个参数）：`src` 是夹具里的目录、`dst` 是那份夹具 conf 里
/// 的 `decklocal:`，两个实参都只在"这条用例的那个夹具"里才有意义，读调用点时要看得见这件事。
fn inventory_job(_fx: &Fx, local: &Path, remote: &str) -> Job {
    Job {
        id: "p".into(), name: "盘点夹具".into(), kind: JobKind::Copy,
        src: fwd(local), dst: remote.to_string(),
        open_dir: None, manifest: None,
    }
}

#[tokio::test]
async fn inventory_lists_only_the_orphans_on_the_far_side() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    std::fs::write(fx.src().join("pending.txt"), b"x").unwrap();
    std::fs::write(fx.dst().join("a.txt"), b"x").unwrap();
    std::fs::write(fx.dst().join("002.webp"), b"stale").unwrap();

    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![inventory_job(&fx, &fx.src(), &fx.remote(""))],
    ).await;
    let report = r.run_inventory("p").await.unwrap();

    assert_eq!(report.orphans, vec!["002.webp".to_string()],
        "pending.txt 只是还没上传，绝不能出现在盘点清单里");
    assert!(fx.dst().join("002.webp").exists(), "盘点不该动任何文件");
    r.shutdown_rcd().await.unwrap();
}

/// R29 的盘点侧证人（R58 之后它是 runner 里**唯一**的空基准拒绝；执行侧那条已随功能删除）：
/// 本地目录存在但为空、远端非空时，集合差交出的是**整个远端**。清单虽然只读，空基准下
/// "本地没有"这个结论本身不可信，盘点直接 `Err` 仍然是硬保险。
/// 认话术（R48 item 3 的口径）+ 日志里不许出现逐条孤儿——拒绝排在逐条打印之前，
/// 挪到之后这条就红。
#[tokio::test]
async fn inventory_refuses_an_empty_local_basis_instead_of_offering_the_whole_remote() {
    let fx = Fx::new();
    std::fs::write(fx.dst().join("002.webp"), b"stale").unwrap();

    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![inventory_job(&fx, &fx.src(), &fx.remote(""))],
    ).await;
    match r.run_inventory("p").await {
        // 认**话术**，不认"有个 Invariant"："这不是「整个位置都该删」"这半句在 `run_inventory`
        // 里只出现在空基准那一段。
        Err(RunError::Invariant(msg)) => assert!(
            msg.contains("这不是「整个位置都该删」"),
            "拒绝的理由不是空基准，而是另一道闸的话术：{msg}"),
        Ok(report) => panic!("空基准被当成盘点清单交出去了：{:?}", report.orphans),
        Err(_) => panic!("期望空基准的 Invariant 拒绝，实际是另一种失败"),
    }
    assert!(fx.dst().join("002.webp").exists(), "拒绝之后不该动任何文件");
    // 拒绝排在**逐条打孤儿**之前：挪到那个循环之后，日志就会把整个远端一条条列出来
    // ——那正是 R29 说的"盘点本身不给结论"要拦的东西。
    let log = std::fs::read_to_string(r.last_log("p").await.expect("应该有日志")).unwrap();
    assert!(log.contains("盘点被拒绝"), "{log}");
    assert!(!log.contains("孤儿 "), "空基准被逐条列了出来：{log}");
    r.shutdown_rcd().await.unwrap();
}

/// 名单落在日志旁边的 `lists/` 里，与那份日志**同名**（只换扩展名）：拿着日志就能找到它。
fn list_file(log: &Path) -> PathBuf {
    log.parent().unwrap().join("lists")
        .join(format!("{}.txt", log.file_stem().unwrap().to_string_lossy()))
}

/// 盘点顺手落一份**可 diff 的名单**（操作员原话："每次都要重列，能不能拉一次存成文件"）。
/// 三条各钉一处：① 内容只有两节名字——时刻与来源一律进文件名，内容里带时刻的话
/// 两周的文件天天整片整片地 diff 红；② 待传那一半的名字**不进日志**（`runner/mod.rs` 里
/// 那条"只记数不记名"的判据不推翻：六千行的东西不该灌进每次动作的证据里）；
/// ③ 与日志同名，住在 `logs/lists/`。
#[tokio::test]
async fn inventory_leaves_a_diffable_name_list_beside_the_log() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    std::fs::write(fx.src().join("pending.txt"), b"x").unwrap();
    std::fs::write(fx.dst().join("a.txt"), b"x").unwrap();
    std::fs::write(fx.dst().join("002.webp"), b"stale").unwrap();

    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![inventory_job(&fx, &fx.src(), &fx.remote(""))],
    ).await;
    r.run_inventory("p").await.unwrap();

    let log = r.last_log("p").await.expect("应该有日志");
    let list = list_file(&log);
    let text = std::fs::read_to_string(&list)
        .unwrap_or_else(|e| panic!("读不到 {}：{e}", list.display()));
    assert_eq!(text, "# 本地有、云端没有\npending.txt\n\n# 云端有、本地没有\n002.webp\n",
        "这份文件的形状是拿去 diff 的：多一行时刻、少一节标题，diff 就脏了");

    let log_text = std::fs::read_to_string(&log).unwrap();
    assert!(!log_text.contains("pending.txt"),
        "待传那一半的名字灌进日志了（那条「只记数不记名」的判据被推翻）：{log_text}");
    r.shutdown_rcd().await.unwrap();
}

/// 空基准那一档除了拒绝，**一个名单文件都不许多写**：那份清单本身不可信，
/// 落盘等于把"整桶都该删"存成一份可以拿去 diff 的证据。
#[tokio::test]
async fn the_refused_inventory_writes_no_name_list() {
    let fx = Fx::new();
    std::fs::write(fx.dst().join("002.webp"), b"stale").unwrap();

    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![inventory_job(&fx, &fx.src(), &fx.remote(""))],
    ).await;
    assert!(r.run_inventory("p").await.is_err(), "空基准不该被放行");
    let log = r.last_log("p").await.expect("应该有日志");
    assert!(!list_file(&log).exists(), "被拒绝的那一次也落了名单");
    r.shutdown_rcd().await.unwrap();
}

/// 这份名单写不成的时候（`lists` 那一档被一个同名文件占了 —— 与"不可写的 pid 文件"同一族夹具）：
/// **盘点本身照样得成**。它是只读面，交出来的两个数是刚列出来的真东西；为了一个副产品
/// 把整次盘点报成失败，等于让证据比附属品次要。但这一趟不许悄悄不吭声：
/// `listFile` 留空（界面那一行因此不露），日志里必须说出**没存成**与为什么。
#[tokio::test]
async fn a_list_that_cannot_be_written_still_returns_the_inventory() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    std::fs::write(fx.src().join("pending.txt"), b"x").unwrap();
    std::fs::write(fx.dst().join("a.txt"), b"x").unwrap();
    std::fs::write(fx.dst().join("002.webp"), b"stale").unwrap();
    // `logs/lists` 要当目录用，先拿一个同名文件占住它。
    std::fs::create_dir_all(fx.root().join("logs")).unwrap();
    std::fs::write(fx.root().join("logs").join("lists"), b"occupied").unwrap();

    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![inventory_job(&fx, &fx.src(), &fx.remote(""))],
    ).await;
    let report = r.run_inventory("p").await
        .expect("名单写不成不该把只读的盘点一起报成失败");
    assert_eq!(report.orphans, vec!["002.webp".to_string()], "清单本身还得交得出来");
    assert_eq!(report.list_file, "", "没存成却把路径交出去了：界面会指到一个不存在的地方");

    let log = std::fs::read_to_string(r.last_log("p").await.unwrap()).unwrap();
    assert!(log.contains("名单没存成"), "日志里没说实话（悄悄少了一份东西）：{log}");
    r.shutdown_rcd().await.unwrap();
}

/// 这份东西的唯一目的是隔两周 diff 一下：两次盘点之间只往本地加一个文件，
/// 两份名单的差就必须**恰好**是那个名字（时刻在文件名里，所以内容不许漂）。
#[tokio::test]
async fn two_inventory_lists_diff_to_exactly_the_change() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    std::fs::write(fx.src().join("pending.txt"), b"x").unwrap();
    std::fs::write(fx.dst().join("a.txt"), b"x").unwrap();

    let (r, _g) = setup_with_conf(
        fx.root(), Some(fx.conf()), vec![inventory_job(&fx, &fx.src(), &fx.remote(""))],
    ).await;
    r.run_inventory("p").await.unwrap();
    let first_log = r.last_log("p").await.unwrap();
    let first = std::fs::read_to_string(list_file(&first_log)).unwrap();

    std::fs::write(fx.src().join("new.txt"), b"x").unwrap();
    r.run_inventory("p").await.unwrap();
    let second_log = r.last_log("p").await.unwrap();
    // 两次各留一份（同一秒内撞名时 `TaskLog::create` 加 `-1`，所以两份必然不同名）。
    assert_ne!(first_log, second_log, "两次盘点共用了同一份日志/名单：历史被后一次盖掉了");
    let second = std::fs::read_to_string(list_file(&second_log)).unwrap();

    let added: Vec<&str> = second.lines()
        .filter(|l| !first.lines().any(|f| &f == l)).collect();
    assert_eq!(added, vec!["new.txt"],
        "两次名单的差不止那一个名字（内容里漂进别的东西了）：{added:?}");
    r.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn every_run_leaves_a_log_file_behind() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let (r, _g) = setup(fx.root(), vec![job("c", JobKind::Copy, &fx.src(), &fx.dst())]).await;

    r.run_copy("c", false).await.unwrap();
    let path = r.last_log("c").await.expect("应该有日志");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("任务开始"), "{text}");
    assert!(text.contains("任务结束"), "{text}");
    assert!(text.contains("deletes=0"), "日志里没有删除计数，事后就没法回溯：{text}");
    r.shutdown_rcd().await.unwrap();
}

/// 每一趟用的**比法**要落进那一趟自己的日志。顶栏那句话只在当下亮着，而"这一趟是按什么算
/// 变了"是事后回溯要用的事实（只比大小那一档会漏掉"内容改了但大小没变"，那件漏掉的事得能被查出来）。
/// 默认那一档要说 `size+modtime`：这一条和下面那条各钉一半，任何一边写死都能让另一边红。
#[tokio::test]
async fn the_log_records_the_default_comparison_basis() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let (r, _g) = setup(fx.root(), vec![job("c", JobKind::Copy, &fx.src(), &fx.dst())]).await;

    r.run_copy("c", false).await.unwrap();
    let text = std::fs::read_to_string(r.last_log("c").await.unwrap()).unwrap();
    assert!(text.contains("compare=size+modtime"),
        "「任务开始」那一行没写这一趟按什么比：{text}");
    r.shutdown_rcd().await.unwrap();
}

/// 带上 `--size-only` 的那一台面板，日志要说 `size-only`，而且**同一趟里不许两个值都在**
/// （两个都在就说明界面与日志各算了一份口径 —— 那是本仓反复红过的那族错）。
/// `setup_with_flags` 把这一串同时喂给 rcd 的 argv 与 runner 的那份 `Defaults`（同源，
/// 与 `app_state.rs` 里"exclude 必须同一份"那条同一个道理）。
#[tokio::test]
async fn the_log_records_a_size_only_basis_when_the_flag_is_configured() {
    let fx = Fx::new();
    std::fs::write(fx.src().join("a.txt"), b"x").unwrap();
    let (r, _g) = setup_with_flags(fx.root(),
        vec![job("c", JobKind::Copy, &fx.src(), &fx.dst())],
        vec!["--size-only".into()]).await;

    r.run_copy("c", false).await.unwrap();
    let text = std::fs::read_to_string(r.last_log("c").await.unwrap()).unwrap();
    assert!(text.contains("compare=size-only"), "开着那一枚时日志没换口径：{text}");
    assert!(!text.contains("compare=size+modtime"),
        "一趟里出现了两个比法，说明有两份口径在各自算：{text}");
    r.shutdown_rcd().await.unwrap();
}

// 让 BTreeSet 的 import 不至于变成死代码：gate 的 key 集合就是这个类型
#[test]
fn key_sets_are_ordered() {
    let s: BTreeSet<String> = ["b", "a"].iter().map(|s| s.to_string()).collect();
    assert_eq!(s.iter().next().unwrap(), "a");
}

// ------------------------------------------------- Task 14：上一次盘点作为数据留在行上，带时刻

#[tokio::test]
async fn an_inventory_leaves_a_summary_on_the_row_with_a_timestamp() {
    // spec §3：行上那两个数必须带着"它是几点算的"，而且**只有跑过才有**。
    let fx = Fx::new();
    seed(&fx.src(), &["a.webp", "b.webp", "new-comer.webp"]);
    seed(&fx.dst(), &["a.webp", "b.webp", "stale.webp"]);
    // 同名但内容不同：这是「会传」里最容易被漏掉的那一类（copy/sync 会覆盖它）。
    std::fs::write(fx.dst().join("a.webp"), b"a different cloud copy").unwrap();
    // `dst` 必须是具名 remote（R30）：两侧都是本地的 job 会被 parse 直接拒，
    // 而那种形状的"远端清单"本来就等于再扫一遍本地（§5.1 那个假绿的入口）。
    let p = job_at("p", JobKind::Copy, &fwd(&fx.src()), &fx.remote(""));
    // 第二条的 src 是一个空目录 ⇒ R29 的空基准拒绝。两条任务共用**一个** daemon：
    // 夹具的端口是固定的，一次测试里绝不并发起两个 rcd。
    let empty_src = fx.root().join("empty-src");
    std::fs::create_dir_all(&empty_src).unwrap();
    let q = job_at("q", JobKind::Copy, &fwd(&empty_src), &fx.remote(""));
    let (r, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![p, q]).await;

    let report = r.run_inventory("p").await.unwrap();
    assert!(!report.measured_at.is_empty(), "盘点结果要说得出它是几点算的");
    // b.webp 两边内容相同、但写入时刻不同，按默认的 size+modtime 也会进这一类 ——
    // 那正是这一趟会重写它、而旧摘要不会把它算进「会传」的原因。这里至少钉住我们改过的 a。
    assert!(report.diverged.iter().any(|d| d.name == "a.webp"),
        "同名不同内容必须进第三类：{:?}", report.diverged);
    let overwrites = report.diverged.len();
    let s = r.last_summary("p").expect("跑过一次就该有摘要");
    assert_eq!((s.pending_upload, s.orphans), (1, 1), "{s:?}");
    // 「会传」= 新增 1 + 会覆盖的那批；只报 pending_upload 会让这次覆盖显示成「会传 0 个」。
    assert_eq!((s.will_upload, s.replaces), (1 + overwrites, overwrites), "{s:?}");
    assert_eq!(s.measured_at, report.measured_at, "行上那个时间必须是这次盘点的时间");
    // 没跑过的任务没有读数 —— 这是"不自动刷"的另一半：不许凭空造一个 0/0 当现状。
    assert!(r.last_summary("nosuch").is_none(), "没跑过的任务不许有一行读数");

    // 被**拒绝**的那次盘点也不留摘要：R29 的空基准那次连结论都不可信，
    // 存下来就等于把一次拒绝演成"上次算过，是两个零"。
    assert!(r.run_inventory("q").await.is_err(), "本地空基准那次该硬拒绝");
    assert!(r.last_summary("q").is_none(), "被拒绝的那次不许留下摘要");
    assert_eq!(files_in(&fx.dst()), vec!["a.webp", "b.webp", "stale.webp"],
        "盘点是只读的：一个东西都不该动：{:?}", files_in(&fx.dst()));
    r.shutdown_rcd().await.unwrap();
}

/// 「会传 N 个」必须等于 rclone 这一趟真正要写的文件数。
///
/// 这是一个跨两层的承诺：卡片上的数来自面板自己的集合差，真正的写入数来自 rclone。
/// 只数 `pending_upload` 的那一版在这条上会红——rclone 会把同名不同内容的也写过去，
/// 而卡片写着「会传 0 个」。用 dry-run 取 rclone 的数：一个文件都不动，读数却是真的。
#[tokio::test]
async fn the_summary_count_matches_what_a_copy_will_actually_write() {
    let fx = Fx::new();
    seed(&fx.src(), &["same.webp", "new.webp"]);
    seed(&fx.dst(), &["same.webp"]);
    std::fs::write(fx.dst().join("same.webp"), b"cloud copy with a different size").unwrap();
    let p = job_at("p", JobKind::Copy, &fwd(&fx.src()), &fx.remote(""));
    let (r, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![p]).await;

    let report = r.run_inventory("p").await.unwrap();
    assert_eq!(report.pending_upload, vec!["new.webp".to_string()], "{:?}", report.pending_upload);
    assert_eq!(report.diverged.len(), 1, "{:?}", report.diverged);
    let s = r.last_summary("p").expect("跑过一次就该有摘要");

    let stats = stats_of(r.run_copy("p", true).await.unwrap());
    assert_eq!(stats.transfers as usize, s.will_upload,
        "卡片说会传 {} 个，rclone 实际要写 {} 个；这两个数不同就是在瞒一次覆盖",
        s.will_upload, stats.transfers);
    assert_eq!(stats.deletes, 0, "copy 不该删东西");
    r.shutdown_rcd().await.unwrap();
}

/// 下载方向的「会传 N 个」必须数**云端有、本地没有**的那一半。
///
/// 这是上一条的镜像：卡片上的数按位置命名（`orphans` 是云端多的），而这一趟写的是
/// dst（本地）。只数 `pending_upload` 会让「以云端为主」那一格显示成「会传 0 个」，
/// 实际却要下载整棵树；而对齐下载方向还会同时把这批说成「会删本机」。
#[tokio::test]
async fn the_summary_count_is_direction_aware_for_downloads() {
    let fx = Fx::new();
    // 本地多出两个，远端多出一个，另有一个同名但大小不同。
    seed(&fx.src(), &["same.webp", "local-a.webp", "local-b.webp"]);
    seed(&fx.dst(), &["same.webp", "remote-new.webp"]);
    std::fs::write(fx.src().join("same.webp"), b"local copy with a different size").unwrap();
    let p = job_at("p", JobKind::Copy, &fx.remote(""), &fwd(&fx.src()));
    let (r, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![p]).await;

    let report = r.run_inventory("p").await.unwrap();
    assert_eq!(report.pending_upload,
        vec!["local-a.webp".to_string(), "local-b.webp".to_string()], "{:?}", report.pending_upload);
    assert_eq!(report.orphans, vec!["remote-new.webp".to_string()], "{:?}", report.orphans);
    assert_eq!(report.diverged.len(), 1, "{:?}", report.diverged);
    let s = r.last_summary("p").expect("跑过一次就该有摘要");
    assert_eq!(s.will_upload, 2, "下载方向要写的是 1 个孤儿 + 1 个同名，不是本地多的那两个");

    let stats = stats_of(r.run_copy("p", true).await.unwrap());
    assert_eq!(stats.transfers as usize, s.will_upload,
        "卡片说会传 {} 个，rclone 实际要写 {} 个；下载方向把位置当成了方向",
        s.will_upload, stats.transfers);
    assert_eq!(stats.deletes, 0, "copy 不该删东西");
    r.shutdown_rcd().await.unwrap();
}

/// 盘点在**两个方向**上都摆得出来，而且两个方向给出的差集是各自那一帧（2026-10-09）。
///
/// 起因是安全上的不对称：原先 `run_inventory` 要求 src 是本地、dst 是远端，于是
/// 「以云端为主」那一格——**删本机硬盘、没有任何撤销**的那一档——恰恰是唯一没有盘点的一格。
///
/// 这条钉的是**方向反了的那一格也能盘**，以及报告仍然是「本地 vs 远端」这一帧：
/// 以云端为主时，要删的是 `pending_upload`（本地有、云端没有的那一份），
/// 不是 `orphans` —— 字段名按位置走，不按方向走。
#[tokio::test]
async fn an_inventory_also_works_with_the_cloud_as_the_winning_side() {
    let fx = Fx::new();
    seed(&fx.src(), &["a.webp", "b.webp", "new-comer.webp"]);   // 本地 3 个
    seed(&fx.dst(), &["a.webp", "b.webp", "stale.webp"]);       // 远端 3 个
    // 方向反了：src 是 remote、dst 是本地目录。这一条在 2026-10-09 之前直接报"src 不是目录"。
    let p = job_at("p", JobKind::Sync, &fx.remote(""), &fwd(&fx.src()));
    let (r, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![p]).await;

    let report = r.run_inventory("p").await.expect("以云端为主那一格也该盘得出来");
    // 报告仍按**位置**命名：orphans = 云端有本地没有的（= 这次会传上去的），pendingUpload = 本地有云端没有的（= 这次会删掉的）。
    assert_eq!(report.orphans, vec!["stale.webp".to_string()],
        "云端多出来的那一个（以云端为主时它会被传下来，不是删除候选）：{:?}", report.orphans);
    assert_eq!(report.pending_upload, vec!["new-comer.webp".to_string()],
        "本地多出来的那一个才是这一格要删的：{:?}", report.pending_upload);
    assert_eq!((report.local_count, report.remote_count), (3, 3), "{report:?}");
    assert_eq!(files_in(&fx.src()), vec!["a.webp", "b.webp", "new-comer.webp"],
        "盘点是只读的：反方向也不许动东西：{:?}", files_in(&fx.src()));
    r.shutdown_rcd().await.unwrap();
}

/// 两侧**同型**时报错并说清是哪一种形状，而不是静默给一份没意义的差集。
/// 两侧都是本地：差集退化成"两棵本机树互减"；两侧都是远端：没有可读的那一列。
#[tokio::test]
async fn an_inventory_refuses_a_pair_whose_two_sides_are_the_same_kind() {
    let fx = Fx::new();
    seed(&fx.src(), &["a.webp"]);
    let other = fx.root().join("other");
    seed(&other, &["b.webp"]);
    let p = job_at("p", JobKind::Copy, &fwd(&fx.src()), &fwd(&other));
    let (r, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![p]).await;
    let err = r.run_inventory("p").await.unwrap_err().to_string();
    assert!(err.contains("都是本地目录"), "两侧同型没被拦下，或没说清是哪一种：{err}");
    r.shutdown_rcd().await.unwrap();
}

/// 「看差异」记下那份比对基准时，记的必须是**本条任务那一档要删的那些**（2026-10-10 修的方向账）。
///
/// 病因：`run_inventory` 那份报告的 `orphans` 与 `pending_upload` 是同一个集合差内核在两个方向上的
/// 两半（互为补集）。原先无论哪一方向都记 `orphans`，于是「云端覆盖本地」那一档 —— 删本机硬盘、
/// 没有任何撤销的那一档 —— 只要先点过「看差异」，动手前那次重列就永远比对不上，每次都拒「清单变了」。
/// 那条出口因此成了死路，而它存在的理由恰恰是「看过差异就别再列一遍」（省的是计费的那次 LIST）。
///
/// 这条证人钉三件事：① 下载方向「看差异 → 直接批 → 跑」这条链**通**（改之前它必然红在「清单变了」）；
/// ② 用的确实是从「看差异」来的那份基准，而不是退回「没看名单」那一档 —— 后者照样能跑通，所以只断言
///    「跑通了」是空证，必须去日志里认那一句；③ 反方向那一条拿的是**自己的键**，不被这一份污染。
#[tokio::test]
async fn a_download_direction_confirmation_reuses_the_seen_inventory_list() {
    let fx = Fx::new();
    // 本机 4 个、云端 3 个：下载方向要删的是本机多出来的那一个（`pending_upload`），而 `orphans`
    // 在这一对位置上是空的 —— 两份互为补集，拿错就永远对不上。
    seed(&fx.src(), &["a.webp", "b.webp", "c.webp", "only-here.webp"]);
    seed(&fx.dst(), &["a.webp", "b.webp", "c.webp"]);
    // 下载方向：src 是那个具名 remote（alias → fx.dst），dst 是本地目录 fx.src。
    let p = job_at("p", JobKind::Sync, &fx.remote(""), &fwd(&fx.src()));
    // 同一对位置的反方向那一条，用来证明键是分方向的、两边各拿自己那份。
    let q = job_at("q", JobKind::Sync, &fwd(&fx.src()), &fx.remote(""));
    let (r, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![p, q]).await;

    r.run_inventory("p").await.expect("下载方向那一格也该盘得出来");
    let token = r.confirm_mirror("p").await.expect("「看差异」之后直接批得下去");
    // ② 基准确实取自「看差异」那一份。那一行是 `confirm_mirror` 写的，而**每一趟动作新开一份日志**，
    // 所以必须在实删之前去读确认那一份 —— 之后再 `last_log` 拿到的是实删那份，那句已经在别的文件里
    // （第一版就是这么错过：读到的尾巴根本没有那一行，而行为是对的）。
    let confirm_log = r.last_log("p").await.expect("确认那一步的日志路径");
    let confirm_text = std::fs::read_to_string(&confirm_log).expect("读确认那一份日志");
    assert!(confirm_text.contains("基准取自「看差异」那一份"),
        "这次确认没用上「看差异」那份基准（那条出口在界面上就是死的）：{confirm_text}");
    let state = r.run_mirror("p", &token).await.expect("对齐被挡下了：改之前这一句红在「清单变了」");
    match state {
        TaskState::Done { stats, .. } => assert_eq!(stats.deletes, 1,
            "删的应该就是本机多出来那一个，不多不少：{stats:?}"),
        other => panic!("期望 Done，实际 {other:?}"),
    }
    // 动手那一份日志要说出这次确实带着逐条名单来比过（`confirmed_a_list`），而不是演出来的。
    let run_log = r.last_log("p").await.expect("实删那一步的日志路径");
    let run_text = std::fs::read_to_string(&run_log).expect("读实删那一份日志");
    assert!(run_text.contains("confirmed_a_list=yes"),
        "动手那一步的记录说这份确认没带逐条名单：{run_text}");
    assert_eq!(files_in(&fx.src()), vec!["a.webp", "b.webp", "c.webp"],
        "本机多出来那一个没了、其余三个还在：{:?}", files_in(&fx.src()));

    // ③ 反方向那一条自己点一次「看差异」、自己拿自己那份（这一趟两端已经齐了，doomed 为空）。
    r.run_inventory("q").await.expect("上传方向那一格");
    let token_q = r.confirm_mirror("q").await.expect("上传方向也批得下去");
    let st_q = r.run_mirror("q", &token_q).await.expect("上传方向也该跑得通");
    assert!(matches!(st_q, TaskState::Done { .. }), "期望 Done，实际 {st_q:?}");
    r.shutdown_rcd().await.unwrap();
}
/// 真写过之后，卡上那份「上次盘点」必须作废；试跑与空跑不许把它拿走。
///
/// 卡面继续挂着跑前的「会传 2 个」，而这一趟已经把 2 个传完了——那是一句假现状。
/// 差异面与日志仍留着这一趟的现场；卡面回到「还没算过差异」，要新数字就再点一次。
#[tokio::test]
async fn a_real_write_retires_the_pre_run_summary_but_a_trial_does_not() {
    let fx = Fx::new();
    seed(&fx.src(), &["new.webp"]);
    let p = job_at("p", JobKind::Copy, &fwd(&fx.src()), &fx.remote(""));
    let (r, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![p]).await;

    r.run_inventory("p").await.unwrap();
    assert!(r.last_summary("p").is_some(), "跑过盘点就该有摘要");

    let dry = r.run_copy("p", true).await.unwrap();
    assert!(matches!(dry, TaskState::Done { dry_run: true, .. }), "试跑终态不对：{dry:?}");
    assert!(r.last_summary("p").is_some(), "试跑一个字节都没写，跑前摘要仍然成立");

    let real = r.run_copy("p", false).await.unwrap();
    assert!(matches!(real, TaskState::Done { dry_run: false, .. }), "真跑终态不对：{real:?}");
    assert!(r.last_summary("p").is_none(), "真写过之后卡面还挂着跑前的会传/会删，就是假现状");

    // 空跑（没有任何文件要写）不碰摘要：它没有让任何一份盘点失真。
    r.run_inventory("p").await.unwrap();
    let again = r.run_copy("p", false).await.unwrap();
    assert!(matches!(again, TaskState::Done { .. }), "{again:?}");
    assert!(r.last_summary("p").is_some(), "零传输的空跑不该把摘要也拿走");
    r.shutdown_rcd().await.unwrap();
}

/// 对账任务的「从哪里来」指向一个不存在的目录时，说人话并指出改哪一格。
///
/// 旧句是 `job g 的 src 不是目录：…` —— 两个内部词（job / src），没有下一步；
/// 而表单只拦空值，不拦"路径写错但格式合法"，所以这一档在新加对账任务时真会碰到。
#[tokio::test]
async fn a_gate_with_a_missing_source_directory_says_where_to_fix_it() {
    let fx = Fx::new();
    let missing = fx.root().join("no-such-src");
    let j = gate_job(&fx, &missing);
    let (r, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![j]).await;

    let err = r.run_gate("g").await.unwrap_err().to_string();
    assert!(err.contains("从哪里来"), "还在用内部的 src 说法：{err}");
    assert!(err.contains("不是一个可读的目录"), "没说清坏在哪：{err}");
    assert!(err.contains("改这条"), "只说坏了，没说去哪儿改：{err}");
    assert!(err.contains("文件清单；到「设置」"), "错误里漂进了源码缩进空格，界面会出现一长串空白：{err}");
    assert!(!err.contains("job "), "内部词 job 又上屏了：{err}");
    r.shutdown_rcd().await.unwrap();
}
