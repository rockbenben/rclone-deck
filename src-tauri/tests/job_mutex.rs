use std::sync::Arc;

use rclone_deck_lib::mutex::{Busy, JobMutex, LockGuard, MultiGuard};
use rclone_deck_lib::scope::Scope;

/// **这个文件里每一个跨实现的调用都要包 timeout**（执行期补的规矩，见台账 Step 5b）。
/// 理由很实际：本模块的回归形态是"卡住"而不是"失败"——R31 那种设计会让第二次 `try_acquire`
/// 静悄悄地去等表锁，测试于是永远不返回。卡住的套件在本项目里会被误读成"子代理挂了"，
/// 而不是"设计坏了"；45s 墙钟下去就是 exit 124，没人看得到断言。包了 timeout，回归=失败。
const WAIT: std::time::Duration = std::time::Duration::from_secs(1);

async fn acquire(m: &JobMutex, key: &str, owner: &str) -> Result<LockGuard, Busy> {
    tokio::time::timeout(WAIT, m.try_acquire(key, owner))
        .await
        .unwrap_or_else(|_| panic!("try_acquire({key:?}) 在 {WAIT:?} 内没返回：互斥退化成了排队"))
}

async fn holder(m: &JobMutex, key: &str) -> Option<String> {
    tokio::time::timeout(WAIT, m.holder(key))
        .await
        .unwrap_or_else(|_| panic!("holder({key:?}) 在 {WAIT:?} 内没返回：读侧被持锁方挡住了"))
}

async fn acquire_all(m: &JobMutex, keys: &[String], owner: &str) -> Result<MultiGuard, Busy> {
    tokio::time::timeout(WAIT, m.acquire_all(keys, owner))
        .await
        .unwrap_or_else(|_| panic!("acquire_all({keys:?}) 在 {WAIT:?} 内没返回：多键申领退化成了排队"))
}

/// 裁定 R90 的那条入口同样要包 timeout，理由与上面两个 helper 一字不差：
/// 它的回归形状可以是"卡住"（改成 await 别的锁）而不只是"判错"，不包就看不见断言。
async fn acquire_scopes(m: &JobMutex, scopes: &[Scope], owner: &str) -> Result<MultiGuard, Busy> {
    let keys: Vec<String> = scopes.iter().map(|s| s.key()).collect();
    tokio::time::timeout(WAIT, m.acquire_scopes(scopes, owner))
        .await
        .unwrap_or_else(|_| panic!("acquire_scopes({keys:?}) 在 {WAIT:?} 内没返回：申领退化成了排队"))
}

#[tokio::test]
async fn the_second_claim_on_a_key_gets_the_first_owner_name() {
    let m = JobMutex::new();
    let _first = acquire(&m, "a=>b", "R2 图床上传").await.unwrap();

    let err = acquire(&m, "a=>b", "R2 图床下载").await.unwrap_err();
    assert_eq!(err.owner, "R2 图床上传");
}

#[tokio::test]
async fn dropping_the_guard_releases_the_key() {
    let m = JobMutex::new();
    {
        let _g = acquire(&m, "a=>b", "job").await.unwrap();
        assert!(acquire(&m, "a=>b", "other").await.is_err());
    }
    assert!(acquire(&m, "a=>b", "other").await.is_ok());
    assert!(holder(&m, "a=>b").await.is_none());
}

/// R39（按诚实性改过词）：`LockGuard::owner()` 的当下读者是**本文件的证人测试**，runner 的日志
/// 是**计划中**的读者（Task 9 未写）。release 构建不需要 `allow(dead_code)` 是因为 `Drop` 的
/// 绊线分类无条件读 `owner` 字段（`cargo check --all-targets --release` 必须无警告）。
/// 顺带钉住两件事：**名字是标签不是所有权**——guard 报的名字、`holder()` 的名字和 `Busy.owner`
/// 是同一个串，而 R34 之后真正的所有权是那个不外泄的 token；以及 `owner()` 与 `key()` 一样，
/// 在 guard 被 move 之前后都读得到（Task 9 把它塞进 spawn 的任务里之后还要写日志）。
#[tokio::test]
async fn the_guard_reports_the_owner_name_that_holder_and_busy_also_report() {
    let m = JobMutex::new();
    let g = acquire(&m, "a=>b", "R2 图床上传").await.unwrap();
    assert_eq!(g.key(), "a=>b");
    assert_eq!(g.owner(), "R2 图床上传", "guard 自己得说清代表谁持锁");
    assert_eq!(holder(&m, "a=>b").await.as_deref(), Some(g.owner()),
        "读侧与 guard 必须报同一个名字");

    let err = acquire(&m, "a=>b", "R2 图床下载").await.unwrap_err();
    assert_eq!(err.owner, g.owner(), "Busy 要报的是**持锁方**的名字，不是申领失败那个");
    drop(g);
    assert!(holder(&m, "a=>b").await.is_none(), "drop 之后键应已放开");
}

#[tokio::test]
async fn the_reverse_direction_is_a_separate_key() {
    let m = JobMutex::new();
    let _up = acquire(&m, "a=>b", "上传").await.unwrap();
    let _down = acquire(&m, "b=>a", "下载").await.unwrap();

    let snap = tokio::time::timeout(WAIT, m.snapshot())
        .await
        .unwrap_or_else(|_| panic!("snapshot() 在 {WAIT:?} 内没返回：界面会在传输期间停止更新"));
    assert_eq!(snap.len(), 2);
    assert_eq!(snap["a=>b"], "上传");
    assert_eq!(snap["b=>a"], "下载");
}

/// 裁定 R31 的第一条证人：**持锁期间不许挡住别的事**。
/// 原来这版实现把 `OwnedMutexGuard` 存进 `LockGuard`，于是"任务锁"和"表锁"成了同一把：
/// 第一个任务一旦持锁，任何后续的 `try_acquire`（**换个键也一样**）都会 await 到任务结束，
/// 面板每秒轮询的 `holder()` / `snapshot()` 同样挂死——那两个测试在原设计里根本不是失败，
/// 是永久卡住。这里每条都包 `tokio::time::timeout`，所以回归的表现是**测试失败**而不是挂住。
#[tokio::test]
async fn a_held_lock_does_not_block_other_keys_or_the_read_side() {
    let m = JobMutex::new();
    let _held = acquire(&m, "a=>b", "R2 图床上传").await.unwrap();

    // 别的键照常能拿（互斥是按键的，不是全局排队）。
    let other = tokio::time::timeout(
        std::time::Duration::from_secs(1), m.try_acquire("c=>d", "另一个任务")).await;
    let _other = other.expect("持锁期间，别的键的 try_acquire 被挡住了（应当立刻返回，成功或 Busy）")
        .expect("不同的键应当拿得到锁");

    // 读侧照常能看（这正是界面每秒轮询的那两条）。
    let snap = tokio::time::timeout(std::time::Duration::from_secs(1), m.snapshot()).await
        .expect("持锁期间 snapshot() 挂住：界面会在传输过程中停止更新");
    assert_eq!(snap.get("a=>b").map(String::as_str), Some("R2 图床上传"));

    let holder = tokio::time::timeout(std::time::Duration::from_secs(1), m.holder("a=>b")).await
        .expect("持锁期间 holder() 挂住");
    assert_eq!(holder.as_deref(), Some("R2 图床上传"));

    // 同一个键必须是**立刻** Busy，而不是排队等待。
    let busy = tokio::time::timeout(
        std::time::Duration::from_secs(1), m.try_acquire("a=>b", "同键的第二个人")).await
        .expect("同键的第二次申领没有立刻被拒，而是在等锁");
    assert_eq!(busy.unwrap_err().owner, "R2 图床上传", "Busy 要说清是谁占着");
}

/// guard 被 `Arc` 带进别的任务里也要照常释放（Task 9 就是这么用的：guard 活在 spawn 出来的任务里）。
/// 这条钉的是 `LockGuard: Send + 'static`——字段从 `OwnedMutexGuard` 换成 `Arc + String` 之后
/// 这两条性质不再由原来的写法自动送上门。
#[tokio::test]
async fn a_guard_can_be_moved_into_another_task_and_still_releases() {
    let m = JobMutex::new();
    let guard = acquire(&m, "a=>b", "spawned").await.unwrap();
    let h = tokio::spawn(async move {
        // 出了作用域就释放。
        assert_eq!(guard.key(), "a=>b");
    });
    h.await.unwrap();
    assert!(holder(&m, "a=>b").await.is_none(), "guard 落地后锁应当已释放");
}

#[tokio::test(flavor = "multi_thread")]
async fn eight_contenders_on_one_key_produce_exactly_one_winner() {
    let m = Arc::new(JobMutex::new());
    let mut handles = vec![];
    for i in 0..8 {
        let m = Arc::clone(&m);
        handles.push(tokio::spawn(async move {
            match acquire(&m, "a=>b", &format!("job-{i}")).await {
                Ok(_guard) => {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    true
                }
                Err(Busy { .. }) => false,
            }
        }));
    }
    let winners: Vec<bool> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|h| h.unwrap())
        .collect();
    assert_eq!(winners.iter().filter(|w| **w).count(), 1, "{winners:?}");
    assert!(holder(&m, "a=>b").await.is_none(), "最后一个 guard 落地后锁应已释放");
}

/// R33 的证人：`Drop` 只许删**自己**那条登记。
/// 这条形状用公开 API 就能搭出来，不需要任何 mutation：
/// 拿 g1（owner=A）→ `force_release` 把 A 那条清掉 → 再拿 g2（owner=B，同一个键）→
/// 这时候丢下 g1：如果 `Drop` 还是无条件 `remove`，B 的登记会被**别人**的 guard 抹掉，
/// 于是第三个任务能在同一对目录上开跑——起点事故。所有权门控之后 B 必须还在。
/// R34 的证人：**同一个任务再跑一次，名字是一样的**（Task 9 传的 `owner` 就是 `job.name`）。
/// 所以所有权不能按名字认，否则这条路走得通：X 的一次运行进 `Unknown` → guard(K,"X") 存进
/// `abandoned` 并活到进程结束 → 人工 `force_release` → **同一个 X 再跑**，登记又是 (K,"X") →
/// 那条陈年 guard 落地，按名字比对**成立**，于是它删掉的是**当下这次运行**的登记：
/// 键被放开、rclone 还在删、反向任务此刻能进来——正是起点事故。
/// 比 token 之后这条彻底断掉：陈年 guard 的 token 与在跑那条不同，它什么都不能删。
#[tokio::test]
async fn an_abandoned_guard_cannot_free_a_same_named_reacquire() {
    let m = JobMutex::new();
    let abandoned = acquire(&m, "a=>b", "R2 图床上传").await.unwrap();
    assert_eq!(force_release(&m, "a=>b", "R2 图床上传").await.as_deref(), Some("R2 图床上传"));

    // 同一个任务再跑一次：名字**必然**相同，这是 Task 9 的真实形状，不是刁钻构造
    let live = acquire(&m, "a=>b", "R2 图床上传").await.unwrap();
    assert_eq!(holder(&m, "a=>b").await.as_deref(), Some("R2 图床上传"));

    drop(abandoned); // 按 token 认这不是它拿的，所以不许动
    assert_eq!(holder(&m, "a=>b").await.as_deref(), Some("R2 图床上传"),
        "陈年 guard 删掉了同名新运行的登记：键被放开而任务还在跑");
    assert!(acquire(&m, "a=>b", "插队者").await.is_err(), "在跑的那次必须还占着这把键");

    drop(live);
    assert!(holder(&m, "a=>b").await.is_none(), "真持有者落地之后才该空出来");
}

/// `force_release` 必须报出它要放的是谁；名字不匹配就不许清。
#[tokio::test]
async fn force_release_refuses_to_free_a_key_held_by_another_name() {
    let m = JobMutex::new();
    let _live = acquire(&m, "a=>b", "R2 图床上传").await.unwrap();
    assert_eq!(force_release(&m, "a=>b", "写错的名字").await, None,
        "名字不匹配却清掉了登记——那等于任何人一条 await 就能放开在跑的任务");
    assert_eq!(holder(&m, "a=>b").await.as_deref(), Some("R2 图床上传"), "登记必须原样还在");
}

/// R33 的证人：`Drop` 只许删**自己**那条登记。
/// 这条形状用公开 API 就能搭出来，不需要任何 mutation：
/// 拿 g1（owner=A）→ `force_release` 把 A 那条清掉 → 再拿 g2（owner=B，同一个键）→
/// 这时候丢下 g1：如果 `Drop` 还是无条件 `remove`，B 的登记会被**别人**的 guard 抹掉，
/// 于是第三个任务能在同一对目录上开跑——起点事故。所有权门控之后 B 必须还在。
#[tokio::test]
async fn a_stale_guard_never_frees_a_newer_owner_on_the_same_key() {
    let m = JobMutex::new();
    let g1 = acquire(&m, "a=>b", "job-1").await.unwrap();
    assert_eq!(force_release(&m, "a=>b", "job-1").await.as_deref(), Some("job-1"));
    let _g2 = acquire(&m, "a=>b", "job-2").await.unwrap();

    drop(g1); // 它拿的锁早被人工清掉了，这条 drop 不许动 job-2

    assert_eq!(holder(&m, "a=>b").await.as_deref(), Some("job-2"),
        "旧 guard 的 Drop 把新持有者的登记删掉了");
    assert!(acquire(&m, "a=>b", "job-3").await.is_err(), "键应当还被 job-2 占着");
}

/// R33：状态未知那条路的出口。**不许**用 `std::mem::forget(guard)` 来长期占键——
/// 那会留下一个进程内洗不掉的幽灵持有者（`snapshot()` 每秒报有人在跑，直到重启）。
/// `force_release` 才是那条唯一出路：它返回被清掉的名字，于是能被写进日志、能被审计。
#[tokio::test]
async fn force_release_is_the_only_way_to_clear_a_key_an_abandoned_guard_holds() {
    let m = JobMutex::new();
    let abandoned = acquire(&m, "a=>b", "跑了但读不到状态").await.unwrap();
    assert_eq!(holder(&m, "a=>b").await.as_deref(), Some("跑了但读不到状态"));

    assert_eq!(force_release(&m, "a=>b", "跑了但读不到状态").await.as_deref(),
        Some("跑了但读不到状态"));
    assert!(holder(&m, "a=>b").await.is_none(), "force_release 之后键必须空出来");

    // guard 还活着，但它现在**什么都不能改**（所有权已经不在这个键上了）。
    drop(abandoned);
    assert!(acquire(&m, "a=>b", "下一个").await.is_ok(), "被放弃的 guard 不许再锁住或释放任何东西");
}

/// 帮助函数：`force_release` 也要包 timeout，理由与 `acquire()` 一样。
/// R34：`force_release` 现在要求报出它要放的是谁（`expected_owner`），名字不匹配就不许清。
async fn force_release(m: &JobMutex, key: &str, expected_owner: &str) -> Option<String> {
    tokio::time::timeout(WAIT, m.force_release(key, expected_owner))
        .await
        .unwrap_or_else(|_| panic!("force_release({key:?}) 在 {WAIT:?} 内没返回"))
}

#[tokio::test]
async fn holder_is_none_for_a_key_nobody_claimed() {
    let m = JobMutex::new();
    assert!(holder(&m, "nope=>nope").await.is_none());
}

#[tokio::test]
async fn a_multi_key_claim_holds_every_key_or_none() {
    let m = JobMutex::new();
    // movefile 要同时拿源与目标。第一把拿到、第二把拿不到时，不许留下"目标已被持有"的残骸。
    let held = acquire_all(&m, &["b#".into()], "上传").await.unwrap();
    let err = acquire_all(&m, &["a#".into(), "b#".into()], "移动").await.unwrap_err();
    assert_eq!(err.owner, "上传", "该报出真正持有那把锁的人");
    assert!(holder(&m, "a#").await.is_none(), "失败的申领不许留下部分持有");
    assert_eq!(holder(&m, "b#").await.as_deref(), Some("上传"), "第二把失败不许把第一把放开");
    drop(held);
    assert!(holder(&m, "b#").await.is_none(), "guard 死了两把都要放掉");
    assert!(acquire_all(&m, &["a#".into(), "b#".into()], "移动").await.is_ok());
}

#[tokio::test]
async fn duplicate_keys_in_one_claim_are_held_once_and_released_wholly() {
    let m = JobMutex::new();
    // 两侧在同一棵树上时（本地↔本地的对齐、或一个动作的两端撞进同一个目录），
    // 一次请求会带两遍同一把键。不去重就是自己撞自己。
    // 两遍 `x#` 故意**不相邻**写：`dedup` 只对有序序列有效，所以只有"先排序再去重"才去得掉，
    // 反过来第二个 `x#` 会活下来撞上自己 —— 这一句钉的就是那句"顺序不能反"。
    let g = acquire_all(&m, &["x#".into(), "y#".into(), "x#".into()], "同一端两侧").await.unwrap();
    assert_eq!(g.keys(), vec!["x#".to_string(), "y#".to_string()],
        "去重要看得见：同树那把只剩一个，而且 keys() 是排过序的");
    assert_eq!(holder(&m, "x#").await.as_deref(), Some("同一端两侧"));
    assert_eq!(holder(&m, "y#").await.as_deref(), Some("同一端两侧"), "不重复的那把也得真的拿着");
    drop(g);
    assert!(holder(&m, "x#").await.is_none(), "重复键释放时必须整个放掉，不能还剩一层");
    assert!(holder(&m, "y#").await.is_none(), "同一批里的每一把都要一起放掉");
}

#[tokio::test]
async fn the_claimed_keys_come_back_sorted_so_a_failure_reason_is_predictable() {
    let m = JobMutex::new();
    let g = acquire_all(&m, &["z#".into(), "a#".into()], "镜像").await.unwrap();
    // 申领内部按字符串排序，所以 keys() 的顺序是确定的。
    assert_eq!(g.keys(), vec!["a#".to_string(), "z#".to_string()]);
}

// --------------------------------------------------------------- 裁定 R90：锁按 Scope 的包含关系互斥
//
// 下面这一族钉的是"**同一棵树的上下两层必须互相排斥**"。为什么它必须由 `acquire_scopes` 来证、
// 而不是由上面那族键串证人顺带覆盖：`acquire_all` 收 `&[String]`，它手里只有等号可比较，
// 于是"占着 `r2:#img`"与"要写 `r2:#img/2026`"在它眼里是两把不相干的键。而浏览器与对齐
// **不共享任务**，它们唯一的关系就是同一棵树的两个层级（spec I3 整条讲的就是这一格），
// 所以等号互斥在这种形状里等于没有互斥——起点事故的形状换了一副面孔又回来了。

/// 祖先挡后代：`r2:img` 这棵树被占着时，它里面的 `r2:img/2026` 进不来。
/// 这条与下面那条**必须成对**（I3 明写"证人必须成对写"，父子那一格与 lookalike 那一格同族）：
/// 只写一个方向，实现就能只按"请求的键是不是持有键的前缀"来判，反方向照样漏。
/// 破坏一次（两条都实跑过）：
/// - `mutex.rs` 的 `conflicts_with` 里 `(Some(h), Some(r))` 那一档换成 `held_key == req_key`
///   （退回纯等号，也就是 R90 之前的形状）→ 本条红，红在那句
///   `.expect_err("父目录被占着时子目录该进不来")`：等号之下子层的申领**成功**了；
/// - 附带一句：`scope.rs` 的 `is_ancestor` 松成 `b.starts_with(a)`（少那个 `/`）时，
///   红的是**本条末尾**那句 `.expect("同远端的邻居前缀不该被挡")` —— 这条自带一个 lookalike 控制。
#[tokio::test]
async fn a_held_parent_tree_blocks_a_claim_on_a_prefix_inside_it() {
    let m = JobMutex::new();
    let _held = acquire_scopes(&m, &[Scope::remote("r2:", "img")], "R2 镜像").await.unwrap();

    let err = acquire_scopes(&m, &[Scope::remote("r2:", "img/2026")], "浏览器 purge")
        .await
        .expect_err("父目录被占着时子目录该进不来");
    assert_eq!(err.owner, "R2 镜像", "被拒要说的是占着那棵树的人，不是失败的申领方");
    // 挡人的那条登记必须原样还在（被拒的申领不许顺手改动持有表）。
    assert_eq!(holder(&m, "r2:#img").await.as_deref(), Some("R2 镜像"));
    // 另一棵树照旧进得来：互斥的单位是"这棵树"，不是"这把键附近"。
    let _other = acquire_scopes(&m, &[Scope::remote("r2:", "imgprompt")], "另一条任务")
        .await
        .expect("同远端的邻居前缀不该被挡");
}

/// 后代挡祖先（上一条的反方向）：占着 `r2:img/2026` 的人挡得住对 `r2:img` 整棵的申领。
/// 这一句在真实形状里更要紧——浏览器 purge 的是一个前缀（子层），要挡的是对齐对整棵树（父层）的写。
/// 破坏一次（两条都实跑过）：
/// - 同上把 `conflicts_with` 那一档退回纯等号 → 本条红（`.expect_err("子层被占着时对整棵树的申领该被拒")`）；
/// - 把 `Scope::conflicts` 改成**单向**判据（只留 `is_ancestor(&self.prefix, &other.prefix)`，
///   删掉 `|| is_ancestor(&other.prefix, &self.prefix)` 那半边）→ **只有本条红**，
///   上一条照旧绿：这一枪量到的正是"两条必须成对写"那句——单向的实现能骗过一半。
#[tokio::test]
async fn a_held_prefix_blocks_a_claim_on_the_whole_tree_above_it() {
    let m = JobMutex::new();
    let _held = acquire_scopes(&m, &[Scope::remote("r2:", "img/2026")], "浏览器 purge")
        .await
        .unwrap();

    let err = acquire_scopes(&m, &[Scope::remote("r2:", "img")], "R2 镜像")
        .await
        .expect_err("子层被占着时对整棵树的申领该被拒");
    assert_eq!(err.owner, "浏览器 purge");
    assert_eq!(holder(&m, "r2:#img/2026").await.as_deref(), Some("浏览器 purge"),
        "被拒的那次申领不许把占着子层那条登记抹掉");
}

/// I3 点名的 lookalike：**相似但不冲突**。`r2:img` 与 `r2:imgprompt` 只是字符串上像，
/// 是两棵互不相干的树；判成包含就是"两条不相干的任务互相假死"。
/// 破坏一次（实跑过）：把 `scope.rs` 的 `is_ancestor` 那句
/// `b.starts_with(&format!("{a}/"))` 松成 `b.starts_with(a)`（少一个 `/`）→ 本条红，
/// 红在那句 `.expect("`img` 与 `imgprompt` 是两棵树，互相假死也是缺陷")`。
/// 它与上面两条一起把"包含"这一格的边界钉住：只钉父子那两条会被一个"任何前缀都算祖先"
/// 的实现骗过去（实测同一枪还红了上面那条的末尾控制句与 `preview_registry` 的 lookalike 句）。
#[tokio::test]
async fn a_lookalike_neighbour_is_not_a_parent_so_it_is_not_blocked() {
    let m = JobMutex::new();
    let _held = acquire_scopes(&m, &[Scope::remote("r2:", "img")], "R2 镜像").await.unwrap();

    let _free = acquire_scopes(&m, &[Scope::remote("r2:", "imgprompt")], "另一条任务")
        .await
        .expect("`img` 与 `imgprompt` 是两棵树，互相假死也是缺陷");
    // 两棵都真的登记在表里（不是"挡错了但放过去了"）：锁过宽在这一格同样是 bug。
    assert_eq!(holder(&m, "r2:#img").await.as_deref(), Some("R2 镜像"));
    assert_eq!(holder(&m, "r2:#imgprompt").await.as_deref(), Some("另一条任务"));
}

/// 不同的远端 = 不同的树，前缀一模一样也不冲突。这一格与上面那条同族（都是"不许过度粘连"），
/// 但它是**另一个判据**：lookalike 比的是 prefix，这条比的是 root。
/// 破坏一次（实跑过）：把 `Scope::conflicts` 里 `self.root == other.root` 那一行换成 `true`
/// （只比前缀、不再问是不是同一朵云）→ 本条红，红在那句 `.expect("两朵云上的同名树是两棵，不该互相挡")`。
#[tokio::test]
async fn the_same_tree_name_on_another_remote_is_not_blocked() {
    let m = JobMutex::new();
    let _held = acquire_scopes(&m, &[Scope::remote("r2:", "a")], "R2 镜像").await.unwrap();

    let _free = acquire_scopes(&m, &[Scope::remote("qiniu:", "a")], "七牛镜像")
        .await
        .expect("两朵云上的同名树是两棵，不该互相挡");
    assert_eq!(holder(&m, "qiniu:#a").await.as_deref(), Some("七牛镜像"));
}

/// 一次申领里带两遍**同一棵树**：仍然只登记一条（Task 4 的去重不变量，R90 之后不许退化）。
/// 两侧落在同一棵树上不是刁钻形状——`Job::write_scopes()` 交的就是两棵树，本地↔本地的对齐、
/// 或一个动作的两端撞进同一个目录时那两把键是同一个串。
/// 两个重复项故意**不相邻**写（`img` 夹在中间）：折叠只对有序序列有效，
/// 顺序反过来的话第二个 `qiniu:#z` 会活下来并撞上第一条自己。
/// 裁定 R91 之后"去重"这一件事并进了那趟**折叠**（等号是包含关系的退化情形，一趟办完两件），
/// 所以本条钉的是折叠的退化那一档，下一条钉的是父子那一档——两条各有落点。
/// 破坏一次（两种写法都实跑过，坐标在 `docs/superpowers/evidence/build-notes.md` 那一轮）：
/// - **整趟折叠删掉**（连精确键去重一起没有，排序保留）→ 本条红，红在那句 `.unwrap()`：
///   `Busy { owner: "同一端两侧" }` —— 第二份 `qiniu:#z` 在表里看见了第一条（自己刚登记的那条）
///   并判成冲突；同一枪下 `a_batch_with_both_layers_of_one_tree_registers_the_ancestor_only`
///   一起红（实测本文件 22 passed / 2 failed，其余七个目标全绿）——一趟折叠确实在同时办两件事；
/// - 把折叠**换成只按精确键去重**（`ordered.dedup_by(...)`，父子那一半不要了）→ 本条**照旧绿**，
///   红的只有父子那条（实测本文件 23 passed / 1 failed）：两条判据各有落点，不是一条盖住另一条。
/// 同两枪下键串那一族的 `duplicate_keys_in_one_claim_are_held_once_and_released_wholly` **照旧绿**：
/// 它走的是 `acquire_all` 里的 `ordered.dedup()`，与锁层这趟折叠互不相干。
#[tokio::test]
async fn two_scopes_of_one_tree_in_a_single_claim_are_held_once_and_released_wholly() {
    let m = JobMutex::new();
    let scopes = [
        Scope::remote("qiniu:", "z"),
        Scope::remote("r2:", "img"),
        Scope::remote("qiniu:", "z"),
    ];
    let g = acquire_scopes(&m, &scopes, "同一端两侧").await.unwrap();
    assert_eq!(g.keys(), vec!["qiniu:#z".to_string(), "r2:#img".to_string()],
        "去重要看得见：同树那把只剩一个，而且 keys() 是按键排过序的");
    assert_eq!(holder(&m, "qiniu:#z").await.as_deref(), Some("同一端两侧"));
    assert_eq!(holder(&m, "r2:#img").await.as_deref(), Some("同一端两侧"), "不重复的那把也得真的拿着");
    drop(g);
    assert!(holder(&m, "qiniu:#z").await.is_none(), "重复键释放时必须整个放掉，不能还剩一层");
    assert!(holder(&m, "r2:#img").await.is_none(), "同一批里的每一把都要一起放掉");
}

/// **一批里同时出现同一棵树的上下两层**（裁定 R91）：`img` 与 `img/backup` 同批申领必须**成功**，
/// 并且只在**父层那把键**上留下一条登记。R91 之前这条拿到的是 `Busy { owner: 它自己 }`：
/// 父层先进表，子层随即在表里看见它自己 ⇒ 那条任务永远跑不了，而拒绝它的话说的是"已有任务在跑"、
/// 报的名字是它本身——锁拒绝一种形状的方式是说谎，这比多拦或少拦都坏。
///
/// 形状不是刁钻出来的：`kind = copy` 允许两侧同为 remote 或同为本地（那条"恰好一侧是 remote"的
/// 校验只管 `sync`），而 `jobs.toml` 明写是给用户编辑的；Task 9/10 的浏览器交的是用户在树上点
/// 出来的两个位置，父子必然出现。配置层不加拒绝的理由写在 `mutex::acquire_scopes` 头上
/// （`ALLOWED_FLAGS` 里没有删除类 flag ⇒ 父子复制只会增加文件）。
///
/// 折叠**不许减弱任何一格**，这三句钉的就是那三格：
/// ① 被折掉的子层、② 父层整棵、③ 更深一层——第三方来申领撞的都是同一条登记、报的都是同一个人
///（同根之下"互为祖先"是一条链，任何被子层挡住的申领也一定被父层挡住）。反面半边照旧钉住：
/// 邻居那棵树（`imgprompt`）必须还进得来，折叠不是顺手把整片前缀挡掉。
///
/// 破坏一次（实跑过，坐标在 `docs/superpowers/evidence/build-notes.md` 那一轮）：
/// **整趟折叠删掉** → 本条红在那句 `.expect("一批里父子同树不该把自己挡在门外（裁定 R91）")`，
/// 拿到的正是 `Busy { owner: "父子同批的那条复制" }`——自己挡自己，就是 R91 要修的形状；
/// 只按精确键去重那一档（把折叠退回 `dedup_by`）红的是同一句、同一个 `Busy` 值，
/// 而上面那条同树重复的证人照旧绿 ⇒ 这一格只有折叠拦得住。
#[tokio::test]
async fn a_batch_with_both_layers_of_one_tree_registers_the_ancestor_only() {
    let m = JobMutex::new();
    let g = acquire_scopes(
        &m,
        &[Scope::remote("r2:", "img"), Scope::remote("r2:", "img/backup")],
        "父子同批的那条复制",
    ).await.expect("一批里父子同树不该把自己挡在门外（裁定 R91）");

    assert_eq!(g.keys(), vec!["r2:#img".to_string()],
        "只登记极大那一层：子层折进父层，guard 里不该有第二条");
    assert_eq!(holder(&m, "r2:#img").await.as_deref(), Some("父子同批的那条复制"));
    assert!(holder(&m, "r2:#img/backup").await.is_none(), "子层另有一条登记 = 折叠没生效");

    // 第三方要哪一层都一样被挡、报的都是同一个人。
    for (req, which) in [
        (Scope::remote("r2:", "img"), "父层整棵"),
        (Scope::remote("r2:", "img/backup"), "被折掉的那一层"),
        (Scope::remote("r2:", "img/backup/2026"), "更深一层"),
    ] {
        let err = acquire_scopes(&m, &[req], "浏览器（Task 9/10 那一方）").await
            .expect_err(&format!("「{which}」那一格该被父层那条登记挡住"));
        assert_eq!(err.owner, "父子同批的那条复制", "「{which}」报的必须是同一个人");
    }

    // 反面半边：邻居树照常进得来（折叠只折本批里的下层，不往外扩）。
    let neighbour = acquire_scopes(&m, &[Scope::remote("r2:", "imgprompt")], "另一条任务").await
        .expect("`img` 与 `imgprompt` 是两棵树，折叠不许把邻居一起挡掉");
    assert_eq!(holder(&m, "r2:#imgprompt").await.as_deref(), Some("另一条任务"));

    drop(g);
    assert!(holder(&m, "r2:#img").await.is_none(), "整批（折叠后只剩一条）必须一起放开");
    drop(neighbour);
}

/// **不对称本身**：只有两边都是 Scope 时才比包含关系，所以一个只交键串的持有者弱一级。
/// 这一条不是"顺手记下现状"，它是那条判据唯一的落点：`conflicts_with` 里
/// "**只有两边都是 Scope 时才比包含关系**"这句话要是漂成"总是比 Scope"或"总是比等号"，
/// 只有这一条会红（上面那族两边都是 Scope，看不出差别；键串那族两边都不是 Scope，也看不出差别）。
///
/// 三面各自钉一件事：
/// 1. 键串持有者 **挡得住** 同键的 Scope 申领（等号那一半还在，`try_acquire` 那 12 条证人的行为边界）；
/// 2. 键串持有者 **挡不住** 子层的 Scope 申领（这就是不对称，也是 `try_acquire` 不能用在生产写路径上的原因）；
/// 3. 反过来，Scope 持有者 **挡不住** 键串申领方要的子层（`try_acquire` 只查自己那一个键），
///    但同键那一条仍挡得住。
///
/// **第 2 面与第 3 面各自单独挨过一枪**（修复轮 1 补的，之前只有第 1 面有记录在案的变异——
/// 一条证人里三句断言，只红过第一句等于后两句从没被量过）：那一枪就是注释里否决过的写法
/// "让 `conflicts_with` 去**拆键串**再比包含关系"。两个方向分开打，因为一次运行只会死在第一句红上：
/// - **两侧都拆**（`held`/`req` 哪边是 `None` 就用 `key.split_once('#')` 还原成 `Scope` 再比）→
///   本条红在**第 2 面**那句 `.expect("登记里没有树可比时只认等号…")`，值
///   `Busy { owner: "老键串的持有者" }`（实测本文件 22 passed / 2 failed，另一条红的是
///   `the_read_side_reports_the_earliest_registration_when_two_clash`——它的构造要的正是这面不对称；
///   其余七个目标全绿，坐标在 `docs/superpowers/evidence/build-notes.md`）；
/// - **只拆申领方**（`req = None` 时把请求键拆成树去比持有方那条 `Scope`，持有方仍只按等号）→
///   本条红在**第 3 面前半**那句 `.expect("try_acquire 只查自己那一个键…")`，值
///   `Busy { owner: "Scope 的持有者" }`，而**第 2 面照旧绿**（实测同样是本文件 22 passed / 2 failed）
///   —— 两枪各自只打得到一面，合成一枪就会有一面永远没被量过。
/// 两枪都不碰第 1 面（等号那一半照旧命中），这正是"哪一档拆掉了都会多出一条静默"的另一半。
///
/// 另两枪（既有记录，都只打在**第 1 面**）：
/// - **去掉 `Option` 那一档**（`(Some(h), Some(r))` 改成 `(h, Some(r)) => h.expect(...)`，即"只要申领方
///   带树就比包含关系"）→ 本条**不是断言失败而是 panic**：`src\mutex.rs` 那句 `expect` 炸在
///   **第 1 面**（键串登记里没有 `Scope` 可取）。同一枪还红了 `runner_local` 的
///   `the_source_side_is_one_of_the_keys_a_run_has_to_take`、`the_reverse_direction_is_refused_…`
///   与 `mirror_runner` 的 `a_run_that_cannot_get_the_lock_has_already_consumed_the_confirmation`
///   —— 那三条都是"键串持有者 + Scope 申领方"的真实形状，所以 `Option` 这一档**不是**纸面对称性；
/// - **删掉等号那一档**（`_ => held_key == req_key` 改成 `_ => false`）→ 本条红在第 1 面的
///   `.expect_err("键相等那条判据不许因为换成 Scope 就漂掉")`；同一枪在键串那一族一起红
///   （B8 那一行有全部八个目标的读数）—— 那一档不是冗余，是那 12 条证人的底座。
#[tokio::test]
async fn a_string_keyed_holder_is_weaker_than_a_scope_holder() {
    let m = JobMutex::new();
    // 第 1 面：键串持有者 vs 同键的 Scope 申领 → 挡得住。
    let str_held = acquire(&m, "r2:#img", "老键串的持有者").await.unwrap();
    let err = acquire_scopes(&m, &[Scope::remote("r2:", "img")], "同键的 Scope 申领")
        .await
        .expect_err("键相等那条判据不许因为换成 Scope 就漂掉");
    assert_eq!(err.owner, "老键串的持有者");

    // 第 2 面：同一个键串持有者 vs **子层**的 Scope 申领 → 挡不住（不对称就在这）。
    let _child = acquire_scopes(&m, &[Scope::remote("r2:", "img/2026")], "子层的 Scope 申领")
        .await
        .expect("登记里没有树可比时只认等号：这条不对称是**有意的**，见 conflicts_with 的注释");
    drop(str_held);
    drop(_child);

    // 第 3 面：Scope 持有者 vs 键串申领方 → 同样只认等号。
    let _scope_held = acquire_scopes(&m, &[Scope::remote("r2:", "img")], "Scope 的持有者")
        .await
        .unwrap();
    let sub = acquire(&m, "r2:#img/2026", "键串申领方").await
        .expect("try_acquire 只查自己那一个键，父层被占着它看不见（同样的不对称，方向反过来）");
    drop(sub);
    let err = acquire(&m, "r2:#img", "键串申领方·同键").await.unwrap_err();
    assert_eq!(err.owner, "Scope 的持有者", "同键那一半两边都得挡得住");
}

// --------------------------------------------------------------- 裁定 R92：读侧与拒绝侧同一把尺子
//
// `JobMutex::blocking_owner` 是界面那句「正在跑：谁」的查询。它必须与拒绝那条用**同一个判据**
// （`conflicts_with`）与**同一个确定性选择**（token 最小那条），否则读侧与拒绝侧长出两套规矩：
// `commands.rs::get_jobs` 原来写的是 `j.scope_keys().iter().find_map(|key| locks.get(key))`，
// 那是按键**等号**查表，而锁从 R90 起比的是**包含关系** ⇒ "占着 `r2:#img` 的对齐"这一格里，
// `r2:#img/2026` 那一行显示成没锁、按钮亮着，点下去拿到 `Busy`。
// 生产形状那一格（一条任务的两棵树里只有一棵落在别人占着的父层里）在 `tests/runner_local.rs` 的
// `a_held_tree_blocks_the_tree_inside_it_but_not_a_neighbour` 里钉，这里钉的是登记表层。

/// 读侧的两个方向（成对写，与上面那族父子证人同一个理由）：占着父层要答得出人，占着子层也要。
/// 反面半边同一条里钉死：邻居树、另一朵云、空批次都必须是 `None`——读侧过宽会把没人碰的行
/// 标成"正在跑"，那与漏报一样是假读数（一个骗人去点，一个骗人不点）。
/// 破坏一次（实跑过，坐标在 `docs/superpowers/evidence/build-notes.md`）：把 `blocking_owner` 里的
/// 判据换成键**等号**（就是 `get_jobs` 原来那句的形状）→ 本条红在第一句
/// 「父层被占着时，子层那一行必须显示有人在跑」，`left: None`、`right: Some("R2 对齐")`；
/// 同一枪下 `the_read_side_reports_the_earliest_registration_when_two_clash` 与 `runner_local` 的
/// `a_held_tree_blocks_the_tree_inside_it_but_not_a_neighbour`（生产那一格）一起红
/// （实测 115 passed / 3 failed，其余五个目标全绿）。
#[tokio::test]
async fn the_read_side_names_the_holder_on_both_layers_of_a_tree() {
    let m = JobMutex::new();
    let parent = acquire_scopes(&m, &[Scope::remote("r2:", "img")], "R2 镜像").await.unwrap();
    // 正向：父层被占着，问子层。
    assert_eq!(
        m.blocking_owner(&[Scope::remote("r2:", "img/2026")]).as_deref(),
        Some("R2 镜像"),
        "父层被占着时，子层那一行必须显示有人在跑（等号查表在这里报 None）");
    // 同键那一半也还在：判据不能漂成"只比父子"。
    assert_eq!(m.blocking_owner(&[Scope::remote("r2:", "img")]).as_deref(), Some("R2 镜像"));
    // 一批里只要有一棵被挡着就算被挡，与 `acquire_scopes` 的全有或全无同形。
    assert_eq!(
        m.blocking_owner(&[Scope::remote("qiniu:", "img"), Scope::remote("r2:", "img/2026")]).as_deref(),
        Some("R2 镜像"),
        "另一朵云上那棵没被挡，不改变这一批会被拒这件事");

    // 反面半边：邻居树 / 另一朵云 / 空批次。
    assert_eq!(m.blocking_owner(&[Scope::remote("r2:", "imgprompt")]), None,
        "邻居树不许被读成'有人占着'：读侧过宽同样是缺陷");
    assert_eq!(m.blocking_owner(&[Scope::remote("qiniu:", "img")]), None, "另一朵云上的同名树");
    assert_eq!(m.blocking_owner(&[]), None, "空批次没有可挡的位置（申领它也不会被拒）");
    drop(parent);

    // 反方向：占着子层的人必须被"整棵树"那一格看见——真实形状就是浏览器 purge 一个前缀、
    // 对齐要写整棵树，读侧漏这一格就是让对齐那一行显示成没锁。
    let child = acquire_scopes(&m, &[Scope::remote("r2:", "img/2026")], "浏览器 purge").await.unwrap();
    assert_eq!(m.blocking_owner(&[Scope::remote("r2:", "img")]).as_deref(), Some("浏览器 purge"),
        "子层被占着时，整棵树那一行同样不许显示'没锁'");
    assert_eq!(m.blocking_owner(&[Scope::remote("r2:", "imgprompt")]), None);
    drop(child);
    assert_eq!(m.blocking_owner(&[Scope::remote("r2:", "img")]), None, "放开之后读侧必须跟着空");
}

/// 两条登记同时挡着时报**谁**：token 最小（最早登记）那条，与 `acquire_one` 同一个确定性选择。
/// 这里构造出"两条都命中、彼此却不冲突"的形状——键串登记与 Scope 登记之间只比等号（就是上上条
/// 那个不对称），所以它们能同时躺在表里：① `Scope(img)` 的登记（早）、② `try_acquire("r2:#img/2026")`
/// 的键串登记（晚）。对 `img/2026` 这个查询两条都命中（①比包含、②比等号），而 `HashMap`
/// 的迭代顺序是随机的 ⇒ 不取 token 最小，界面那一行的名字每次轮询换一个。
/// 破坏一次（实跑过，坐标在 `docs/superpowers/evidence/build-notes.md`）：`blocking_owner` 里
/// `min_by_key` 换成 `max_by_key` → 只有本条红，报出来的是 `Some("后登记的子层键")`
/// 而不是 `Some("先登记的父层")`（实测本文件 23 passed / 1 failed，上面那条读侧证人照旧绿——
/// 两条钉的是两件事，这正是分开写的作用）。
#[tokio::test]
async fn the_read_side_reports_the_earliest_registration_when_two_clash() {
    let m = JobMutex::new();
    let scope_held = acquire_scopes(&m, &[Scope::remote("r2:", "img")], "先登记的父层").await.unwrap();
    // 键串申领方进得来（不对称那一档），于是表里有两条都能挡 `r2:#img/2026` 的登记。
    let str_held = acquire(&m, "r2:#img/2026", "后登记的子层键").await.unwrap();

    assert_eq!(
        m.blocking_owner(&[Scope::remote("r2:", "img/2026")]).as_deref(),
        Some("先登记的父层"),
        "两条都挡着时报的必须是**最早登记**那条，与 `acquire_one` 拒绝时报的同一个");

    drop(str_held);
    drop(scope_held);
    assert_eq!(m.blocking_owner(&[Scope::remote("r2:", "img/2026")]), None);
}
