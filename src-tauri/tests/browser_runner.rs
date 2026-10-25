//! 浏览器动作面的护栏：四句拒绝的话术，加上读侧两条生产通路在**真 daemon** 上的读数。
//!
//! 这里每条都在问同一件事：界面交回来的那个位置，面板说的是不是一棵算得出的树；
//! 而拒绝的时候，是不是真的一个字节都没动。
//!
//! 为什么起真 daemon：`Runner::browser_remotes` / `browser_list` 要 `Arc<RcdHandle>`，
//! 而 `RcdHandle` 只能由 `RcdSupervisor::start` 交出来，wiremock 桩不出那一层
//! （Task 8 报告缺口 1，裁定 R96 把它排在这里）。
//! 全部本地对本地：`decklocal:` 是 alias 指向临时目录（`localdaemon` 那套夹具），
//! `r2:` / `qiniu:` 一个请求都不发。

#![cfg(windows)]

pub mod localdaemon;
use localdaemon::*;

use rclone_deck_lib::config::jobs::JobKind;
use rclone_deck_lib::runner::browser::{
    browser_clear_refused_by_rules_refusal, browser_list_changed_refusal, browser_prefix_root_refusal,
    browser_scope, browser_target_exists_refusal, browser_target_label, local_fs_id,
    purge_deleted_dirs, purge_done_count, BrowserListing, ShortNote,
};

// ---------------------------------------------------------------- 四句话术（纯函数）

#[test]
fn the_overwrite_refusal_says_the_panel_blocked_it_and_when_the_target_changed() {
    // I2c/R73 的归因必须说对：rclone 对一个撞名的写入回的是 **200**（F10），
    // 所以"它拒绝了"是假话；而"没动它"只在**我们拦下**的那一次才是真的。
    let s = browser_target_exists_refusal("decklocal:2026/a.webp", 4242, "2026-10-01T18:10:48+08:00");
    assert!(s.contains("已经有一个同名文件"), "{s}");
    assert!(s.contains("换个名字"), "得给出下一步：{s}");
    assert!(s.contains("4.14 KiB") && s.contains("10-01 18:10"),
        "大小与时间是操作员判断'是不是我要动的那个东西'的依据，而它们走人类读法（秒与纳秒留在日志里）：{s}");
    assert!(!s.contains("rclone"), "不许冒认成 rc 的拒绝：{s}");
    assert!(!s.contains("object not found"), "rc 那句反话不许出现在任何话术里：{s}");
}

#[test]
fn the_target_label_inserts_the_separator_only_where_the_root_shape_needs_it() {
    // 回显只走 `Scope::describe()` 那一处坐标系（裁定 R97）。三种根形各钉一句：
    // 本地盘符的 `fs` 结尾**没有**分隔符（`local_fs_id` 交的是 `D:/x/y`），不插就粘成 `D:/x/ya.webp`；
    // 远端名自带结尾冒号，插了反而变成 `r2:/img`；桶名写在 `fs` 里那种，插的是桶内那一段。
    // 其余用例的目标都是 alias 远端（那种拼法恰好与粘法等价），所以这一格只有这里能钉住。
    assert_eq!(browser_target_label("D:/vault/img", "a.webp"), "d:/vault/img/a.webp");
    assert_eq!(browser_target_label("r2:", "img/a.webp"), "r2:img/a.webp");
    assert_eq!(browser_target_label("r2:img", "2026/a.webp"), "r2:img/2026/a.webp");
    assert_eq!(browser_target_label("D:/vault/img", ""), "d:/vault/img", "只到位置本身时不留尾斜杠");
}

#[test]
fn the_clear_prefix_refusal_names_the_rule_count_and_the_alternative() {
    // R74/I8：操作员唯一的判断依据是"这里显示的不是全部"。
    let s = browser_clear_refused_by_rules_refusal(2);
    assert!(s.contains("2 条排除规则") && s.contains("没显示出来"), "{s}");
    assert!(s.contains("删掉这一层看到的"), "要指出能用的那条通路（按钮上那个名字逐字对上）：{s}");
    assert!(!s.contains("purge"), "内部方法名不上界面：{s}");
}

#[test]
fn a_prefix_root_is_named_as_the_thing_that_was_refused() {
    // F12：alias→本地时 purge 连目录本身一起删，所以两侧的根一律拒绝。
    let s = browser_prefix_root_refusal("远端");
    assert!(s.contains("不能清空整个远端") && s.contains("根上"), "{s}");
}

#[test]
fn a_changed_listing_refusal_says_both_counts_and_denies_a_partial_action() {
    let s = browser_list_changed_refusal("这个前缀", 3, 5);
    assert!(s.contains("3") && s.contains("5"), "{s}");
    assert!(!s.contains("已删除"), "复核不过就一个都没动，别说成做了一半：{s}");
}

// ---------------------------------------------------------------- 读侧在真 daemon 上的形状

/// 三条形状同源的底座：`decklocal:` = alias 指向 fx.dst()，里面两个深度共 3 个对象。
async fn browse_setup() -> (Fx, rclone_deck_lib::runner::Runner, DaemonGuard) {
    let fx = Fx::new();
    seed(&fx.dst(), &["a.webp", "sub/b.webp", "sub/deep/c.webp"]);
    seed(&fx.src(), &["a.webp"]);
    // 任务里出现过的那一个本地目录（dst 是 alias 指的临时目录，不算本地行），
    // 就是左列该收进来的那一条本地行（spec §3/§4：不新增配置）。
    let job = job_at("j", JobKind::Copy, &fwd(&fx.src()), &fx.remote(""));
    let (runner, guard) = setup_with_conf(fx.root(), Some(fx.conf()), vec![job]).await;
    (fx, runner, guard)
}

#[tokio::test]
async fn browsing_a_prefix_sends_the_names_the_write_side_will_have_to_use() {
    // 这一条钉的是读侧交出去的那份**坐标**：`Path`（fs 根内的完整相对名），不是会撞车的最后一段。
    // F16 实测：同一棵树下 `pick.webp` 与 `x/pick.webp` 的 `Name` 都是 `pick.webp`，而写动作按名字
    // 执行（I2c）——交错了名字，Task 11 的"删这一个"就会删到没被点中的那一个。
    //
    // 列举是**单层**的（见 `rc::ListShape` 的 `Level` 那一档）：
    // 每进来一层只看到这一层的条目，深处的东西要再进一层才看到 —— 它没丢，只是不再摊平在同一张表里。
    // 坐标那条性质照旧：条目名仍然是 fs 根内的完整相对名，写动作直接拿它当 `remote`。
    let (_fx, runner, _g) = browse_setup().await;
    let l = runner.browser_list("decklocal:", "").await.unwrap();
    assert_eq!(l.path, "decklocal:", "回显那句要读得出是哪个位置（I1）：{}", l.path);
    assert_eq!(
        l.entries.iter().map(|e| (e.name.as_str(), e.size, e.is_dir)).collect::<Vec<_>>(),
        [("sub", 0, true), ("a.webp", 1, false)],
        "根上只有这一层：目录在前、按名字排；深处的条目不摊平到这里：{:?}", l.entries);
    // 前缀里那一层：条目名继续带满前缀，写动作直接拿它当 `remote`。
    let sub = runner.browser_list("decklocal:", "sub").await.unwrap();
    assert_eq!(sub.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["sub/deep", "sub/b.webp"], "前缀要跟着名字走：{:?}", sub.entries);
    assert_eq!(sub.path, "decklocal:sub");
    // 再进一层：`sub/deep/c.webp` 在这里，而且名字仍然是完整相对名（不是 `c.webp`）。
    let deep = runner.browser_list("decklocal:", "sub/deep").await.unwrap();
    assert_eq!(deep.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["sub/deep/c.webp"], "深处那一个进一层就在，而且坐标没退成最后一段：{:?}", deep.entries);
}

#[tokio::test]
async fn the_usage_is_the_set_the_operator_just_looked_at() {
    // F16：`operations/size` 在本地这一族上**不看前缀**（问 `sub` 与问一个不存在的前缀都回
    // 整棵树 13 B / 4 个，而且都是 200），所以用量按已经列到的那一份算。
    // 列举改成单层之后，"已经列到的那一份"就是**这一层**：根上 `1 B / 1 个`（`a.webp`），
    // 进一层 `1 B / 1 个`（`sub/b.webp`），再进一层 `1 B / 1 个`（`sub/deep/c.webp`）。
    // 目录条目不进数。界面那句读数因此必须说"这一层"，不能说"这个前缀往下一共"。
    let (_fx, runner, _g) = browse_setup().await;
    let root = runner.browser_list("decklocal:", "").await.unwrap();
    assert_eq!((root.total_bytes, root.total_count), (1, 1), "{:?}", root.entries);
    let sub = runner.browser_list("decklocal:", "sub").await.unwrap();
    assert_eq!((sub.total_bytes, sub.total_count), (1, 1), "{:?}", sub.entries);
    let deep = runner.browser_list("decklocal:", "sub/deep").await.unwrap();
    assert_eq!((deep.total_bytes, deep.total_count), (1, 1), "{:?}", deep.entries);
    // 共享夹具的 exclude 是空的 ⇒ 规则那道闸是开的；但**根上那一档仍然不给清空**（F12：
    // alias→本地时 purge 连目录本身一起删）。两道闸与那句原因都在后台一次算完，
    // 作为两个字段走出去 —— 界面不许从"能不能"反推"为什么"。
    assert_eq!(root.filtered_note, ShortNote::none());
    assert!(!root.can_clear_prefix, "站在根上不给清空（F12）：{:?}", root.clear_note);
    assert!(root.clear_note.contains("根上"), "点不动要说得出是哪一道闸： {:?}", root.clear_note);
    assert!(sub.can_clear_prefix, "进到子层就该给清空：{:?}", sub.clear_note);
    assert_eq!(sub.clear_note, ShortNote::none(), "能清空时那两段都是空串，界面上不挂 Tooltip：{:?}", sub.clear_note);
}

#[tokio::test]
async fn a_capability_note_explains_and_never_vetoes_the_listing_on_a_real_daemon() {
    // 简报 Step 6 第 3 枪要的那一条，按**量到的形状**写：
    // `decklocal:`（alias→本地）的 fsinfo 实测答 200 + `Copy=false / Purge=false / FilterAware=true`，
    // 所以 capabilities 是**非空**的三段话——简报原文那句"断言它是空串"与实测相反，没照抄。
    // "解释性字段不许否决列表"那一臂（fsinfo 单独失败）在本地形状上**不可达**：
    // 目录不存在时 fsinfo 回 200（它不做存在性检查），fs 是文件时它和 list 一起 500（都是实测）。
    // 那一臂钉在客户端那层（tests/browser_list.rs 的
    // `a_capability_lookup_failure_still_returns_the_listing`）+ `feature_note(空表)==""`。
    let (_fx, runner, _g) = browse_setup().await;
    let l = runner.browser_list("decklocal:", "").await.unwrap();
    assert!(!l.capabilities.is_empty(), "alias→本地实测会报那三位，这里必须看得见那句解释");
    assert!(l.capabilities.contains("没报"), "要说'没报这个能力'，不是'不能做'（F6）：{:?}", l.capabilities);
    assert!(l.capabilities.contains("排除规则"), "FilterAware=true 正是那句过滤提示的来源：{:?}", l.capabilities);
    assert!(!l.capabilities.contains("Purge=false"), "内部位名与 `键=值` 形状都不许上界面：{:?}", l.capabilities);
    assert!(!l.entries.is_empty(), "解释性字段在场的时候，列表也在场");
}

#[tokio::test]
async fn a_typoed_remote_and_an_empty_fs_fail_loudly_instead_of_reading_as_empty() {
    // 读侧最贵的两种误读：打错的远端名 / 空 fs 被当成"这一侧什么都没有"。
    // 两条都是实测：`didn't find section in config file ("nodecknosuchremote")` 与
    // `can't use empty string as a path`——原样抛出，没有走进 `ListError::NotFound` 那一档。
    let (_fx, runner, _g) = browse_setup().await;
    let err = runner.browser_list("nodecknosuchremote:", "").await.unwrap_err().to_string();
    assert!(err.contains("nodecknosuchremote"), "打错的远端名必须点名，不许读成空桶：{err}");
    let err = runner.browser_list("", "").await.unwrap_err().to_string();
    assert!(!err.is_empty(), "空 fs 不是'一个空目录'：{err}");
    // 而**这个**才是"这一侧没有东西"那一档（F9/R64）：前缀没了 ⇒ Ok + 空。
    let gone: BrowserListing = runner.browser_list("decklocal:", "no-such-dir-9f3c").await.unwrap();
    assert!(gone.entries.is_empty() && gone.total_count == 0, "{:?}", gone.entries);
    // 前缀是一个文件名：响亮失败（rclone 答的是 `readdir …: The system cannot find the path
    // specified.`，原文带操作员自己的路径，按 F14 那条口径可以给他看），不许读成"这个目录是空的"。
    assert!(runner.browser_list("decklocal:", "a.webp").await.is_err());
}

/// 本地根被移走/改名时，那一栏必须是"列不出来"，不能是一个漂亮的空目录。
///
/// rclone 对本地缺失路径的回法会把两者混成零条；界面于是把「D:\我的图片 不见了」
/// 显示成「这一层 0 个文件」，还带着一句「暂无数据」。这是出厂示范那条路径最可能的形状。
#[tokio::test]
async fn a_missing_local_directory_fails_instead_of_looking_empty() {
    let fx = Fx::new();
    seed(&fx.src(), &["a.webp"]);
    let job = job_at("j", JobKind::Copy, &fwd(&fx.src()), &fx.remote(""));
    let id = local_fs_id(&fx.src()).expect("源目录是盘符形状的本地 fs");
    let (runner, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![job]).await;
    std::fs::remove_dir_all(fx.src()).unwrap();

    let err = runner.browser_list(&id, "").await.unwrap_err().to_string();
    assert!(err.contains("不存在"), "移走的目录必须说它不在，而不是交出 0 条：{err}");
    assert!(err.contains("改这条") && err.contains("换一对位置"),
        "只说坏了不够，还要说去哪儿改：{err}");
}

#[tokio::test]
async fn the_left_column_offers_named_remotes_plus_the_measured_local_shapes_only() {
    // F3：远端那几行问 rclone 自己，id 带冒号（`config/listremotes` 回来的名字**不带**）。
    // F16：本地那几行的 id 是**转换**后的盘符正斜杠形状，不是 `to_string_lossy()` 的原样；
    // 没量过的形状（UNC、相对路径、盘符相对、整块盘）根本不发——这一条把那两件事一起钉住。
    let (fx, runner, _g) = browse_setup().await;
    let rs = runner.browser_remotes().await.unwrap();
    assert!(rs.iter().any(|r| r.id == "decklocal:" && !r.is_local), "{:?}", rs);
    let local = rs.iter().filter(|r| r.is_local).collect::<Vec<_>>();
    // 实测就是**一条**：这条任务的 src 是本地目录、dst 是 `decklocal:`（远端），所以本地只有 src 那一行。
    assert_eq!(local.len(), 1, "左列不该凭空多出一行：{:?}", local);
    for r in &local {
        assert_eq!(r.id, local_fs_id(std::path::Path::new(&r.id)).unwrap(),
            "本地那一行的 id 必须是转换过的形状：{}", r.id);
        assert!(!r.id.contains('\\'), "不许把反斜杠发进 rc 的 `fs`：{}", r.id);
    }
    // 与 F16 那一档对照：开头带斜杠的写法会答成 `Ok + 空列表`（实测），所以它绝不该出现在左列。
    let lead = format!("/{}", fwd(&fx.src()));
    assert!(local.iter().all(|r| r.id != lead), "{:?}", local);
}

/// 远端那半边也要交出**任务里在用的位置**，不能只交远端名。
///
/// 只交远端名的话，站在 `r2:` 上那个动作在 rclone 里是"列桶"（ListBuckets），而按桶发的钥匙
/// 没有这个权限：在操作员真配置上量过（只读，各一次）——`fs="r2:"` 回 500
/// `AccessDenied … ListBuckets`，同一把钥匙 `fs="r2:img"` 回 200。而那一档当时是左列里
/// **唯一**能选的远端行，所以那一栏永远列不出来（操作员报的就是这个）。
///
/// 钉两件事：那一行在名单里，而且它**真的列得动** —— 只多一行死条目等于没修。
#[tokio::test]
async fn the_remote_side_offers_the_positions_the_jobs_already_use() {
    let fx = Fx::new();
    seed(&fx.dst(), &["a.webp", "sub/b.webp", "sub/deep/c.webp"]);
    seed(&fx.src(), &["a.webp"]);
    // dst 带桶内路径：`endpoint_of` 把它切成 fs=`decklocal:sub`、remote=`deep`，
    // 拼回去就是配置里那个写法，也正是 `fs` 参数该有的形状。
    let deep = fx.remote("sub/deep");
    let job = job_at("j", JobKind::Copy, &fwd(&fx.src()), &deep);
    let (runner, _g) = setup_with_conf(fx.root(), Some(fx.conf()), vec![job]).await;

    let rs = runner.browser_remotes().await.unwrap();
    let row = rs.iter().find(|r| r.id == deep)
        .unwrap_or_else(|| panic!("左列里没有任务在用的那个远端位置：{rs:?}"));
    assert!(!row.is_local, "它是远端那半边的行：{row:?}");
    assert_eq!(row.label, deep, "远端行的标签就是位置本身（只有本地行用目录名）：{row:?}");
    // 根那一行还在（钥匙够的人要用它），而且不会因为派生行多出一份重复。
    assert_eq!(rs.iter().filter(|r| r.id == "decklocal:").count(), 1, "{rs:?}");

    // 关键的一半：站上去列得动。fs 带了路径段时，条目名是**相对那个 fs** 的（实测：
    // `fs="fx:d001"` 回来的 Path 不再重复带 `d001/`），写动作拿它当 `remote` 正好对上。
    let l = runner.browser_list(&deep, "").await.unwrap();
    assert_eq!(l.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["c.webp"],
        "那一行列不出东西的话，这一行就是死的：{:?}", l.entries);
}

// ---------------------------------------------------------------- 点段在边界拒，且一个请求都不发

#[tokio::test]
async fn a_dot_segment_is_refused_before_any_request_rather_than_by_the_daemon_complaining() {
    // 这一条要钉的不是"会报错"（纯函数那条证人已经钉了判据），而是**拒在发请求之前**：
    // `prefix` 是界面在一次动作里交回来的自由串，配置侧的 parse 管不到它（裁定 R98/R99）。
    // 打法是把两句话术分开：同一个不存在的远端，带点段的那一次答的是面板那句（含"点"、
    // 且没有 rc 的 `didn't find section`），正常名字那一次答的是 rc 那句。
    // 反面半边不是装饰：少了它，"没有 didn't find section"这一句可能永远为真（证人空转）。
    // 不用"读不到远端名"当判据——面板那句拒绝本来就会把位置整个念出来。
    // 这样就不必给 `Runner` 开一条读请求计数的口子（那才是为了证人扩可见性）。
    let (_fx, runner, _g) = browse_setup().await;
    // 分界的是**回答来自哪一层**，不是名字：面板那句拒绝会把整个位置念出来（含远端名），
    // 所以"读不到远端名"根本不是可用的判据 —— 用的是 rc 那句话的措辞。
    let err = runner.browser_list("nodecknosuchremote:", "a/../b").await.unwrap_err().to_string();
    assert!(err.contains("点"), "这句必须是面板自己拦的：{err}");
    assert!(!err.contains("didn't find section"),
        "既然没发请求，rc 那句'没这个远端'不可能出现；出现了就是先发了再拒：{err}");

    // 反面半边：同一个远端、一个正常名字 —— 这一次真的发出去了，读到的就是 rc 的话。
    let say = runner.browser_list("nodecknosuchremote:", "a/b").await.unwrap_err().to_string();
    assert!(say.contains("didn't find section"), "对照组要证明上面那句不是永远为真：{say}");
    assert!(!say.contains("点"), "rc 那句里不该混进面板的拒绝话术：{say}");
}

// ---------------------------------------------------------------- Task 10：复制类动作（copyfile）

/// 一棵浏览器要动的树 + 一条对齐任务 `m`（`src = fx.src/`，`dst = decklocal:` 根）。
/// `m` 存在只为了一个理由：锁要把浏览器与对齐算成**同一批树**（spec I3）。
async fn browser_tree(names: &[&str]) -> (Fx, rclone_deck_lib::runner::Runner, DaemonGuard) {
    let fx = Fx::new();
    seed(&fx.dst(), names);
    let jobs = vec![job_at("m", JobKind::Sync, &fwd(&fx.src()), &fx.remote(""))];
    let (runner, guard) = setup_with_conf(fx.root(), Some(fx.conf()), jobs).await;
    (fx, runner, guard)
}

/// 对齐 `m` 要写的那两棵树。交 `Vec<Scope>` 而不是键串（裁定 R100）：
/// 键串那一侧的登记 `scope = None` 只按等号命中，"对齐占着 `decklocal:` 整棵"
/// 与"浏览器往 `decklocal:sub/` 里写"就判不出冲突——而那正是这一族用例要挡的形状。
/// 计划原文这里写的是 `acquire_all(&mirror_keys(...))`，实跑会挡不住，已按裁定改。
fn mirror_scopes(runner: &rclone_deck_lib::runner::Runner) -> Vec<rclone_deck_lib::scope::Scope> {
    runner.job("m").expect("夹具里有 m").write_scopes()
}

/// 夹具根下面另一个本地目录（与对齐的 `src/` 是兄弟，不在它的树里）。
fn outside(fx: &Fx, name: &str) -> std::path::PathBuf {
    let p = fx.root().join(name);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[tokio::test]
async fn a_download_copies_the_named_object_and_touches_nothing_else() {
    // F1 的那条规矩在浏览器侧继续钉：只动被点中的那一个。
    let (fx, runner, _g) = browser_tree(&["pick.webp", "neighbour.webp"]).await;
    let out = outside(&fx, "local-out");
    let r = runner.browser_copy("decklocal:", "pick.webp", &fwd(&out), "down.webp").await.unwrap();
    // 复制不发凭证、没有清单：`has_list` 为假，那两个数在界面上不出现（后台的口径）。
    assert_eq!((r.has_list, r.confirmed, r.done, r.agrees), (false, 0, 1, true), "{r:?}");
    assert_eq!(r.note, "源保留，写了 1 个对象", "没有清单那一档要说的话作为数据走出去：{r:?}");
    assert_eq!(files_in(&out), vec!["down.webp"], "只该动被点中的那一个：{:?}", files_in(&out));
    assert_eq!(files_in(&fx.dst()), vec!["neighbour.webp", "pick.webp"],
        "源侧一个都不许动（copyfile 是非破坏动作）：{:?}", files_in(&fx.dst()));
    // 报出去的那一句要点名目标位置（I1），且不许是"当前目录"那种要靠记忆补的说法。
    assert!(r.target.contains("down.webp"), "{:?}", r.target);
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_colliding_upload_is_refused_by_the_panel_before_any_write() {
    // I2c/R73 的行为证人。**为什么必须存在**：F10 那一档（同大小同 modtime）rc 回 200 而什么都不写，
    // 面板若照"rc 会报错"来设计，操作员会看到一次成功的提示和一份没变的旧文件。
    let fx = Fx::new();
    seed(&fx.src(), &["same.webp"]);
    seed(&fx.dst(), &["same.webp"]);
    let before = std::fs::read(fx.dst().join("same.webp")).unwrap();
    let (runner, _g) = setup_with_conf(fx.root(), Some(fx.conf()),
        vec![job_at("u", JobKind::Copy, &fwd(&fx.src()), &fx.remote(""))]).await;
    // 这一次是往**云端**写一个已经在那里的名字。
    let err = runner.browser_copy(&fwd(&fx.src()), "same.webp", "decklocal:", "same.webp")
        .await.unwrap_err().to_string();
    assert!(err.contains("已经有一个同名文件"), "{err}");
    assert!(err.contains("一个字节都没写"), "拦下来就必须真的没动：{err}");
    assert!(!err.contains("object not found"), "不许把 rc 那句反话端上来：{err}");
    assert_eq!(std::fs::read(fx.dst().join("same.webp")).unwrap(), before,
        "被拒的那一次不许写过任何字节");
    assert!(fx.src().join("same.webp").exists(), "复制是非破坏动作，源要在");
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_running_mirror_blocks_a_write_into_a_directory_inside_its_tree() {
    // I3 的第一句话：一把锁管住两个入口。这一格要的形状是**包含**而不是等号——
    // 浏览器写的是 `decklocal:sub/`（对齐占着的是 `decklocal:` 整棵），
    // 键串的登记（`scope = None`）在这一格挡不住（裁定 R90/R100），所以这条同时是
    // "锁认识树"的活证人。
    let (fx, runner, _g) = browser_tree(&["x.webp", "sub/old.webp"]).await;
    let from = outside(&fx, "outside");
    std::fs::write(from.join("pick.webp"), b"x").unwrap();

    let held = runner.mutex().acquire_scopes(&mirror_scopes(&runner), "镜像任务").await.unwrap();
    let err = runner.browser_copy(&fwd(&from), "pick.webp", "decklocal:", "sub/copy.webp")
        .await.unwrap_err().to_string();
    assert!(err.contains("镜像任务"), "Busy 要报出持锁的人：{err}");
    assert!(!fx.dst().join("sub/copy.webp").exists(), "被挡的那一次一个字节都不该落：{:?}", files_in(&fx.dst()));
    drop(held);

    // 放开之后同一件事就该做成——这一句防止"永远挡住"那种实现蒙过去。
    let r = runner.browser_copy(&fwd(&from), "pick.webp", "decklocal:", "sub/copy.webp").await.unwrap();
    assert_eq!(r.done, 1, "{r:?}");
    assert!(fx.dst().join("sub/copy.webp").exists(), "{:?}", files_in(&fx.dst()));
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_browser_read_out_of_the_mirrors_tree_is_blocked_too() {
    // 一次复制动的是**两棵树**，所以源端也要进集合：浏览器从对齐正占着的那棵树里**读**一个对象出来，
    // 也会改掉那份对齐预览的源（源变了删除候选就变了，spec I3 第 3 条）。
    // 只把目标端交出去的实现会在这一条红（计划 Step 6 的第 2 枪）。
    let (fx, runner, _g) = browser_tree(&["x.webp", "sub/old.webp"]).await;
    let into = outside(&fx, "elsewhere");
    let held = runner.mutex().acquire_scopes(&mirror_scopes(&runner), "镜像任务").await.unwrap();
    let err = runner.browser_copy("decklocal:", "sub/old.webp", &fwd(&into), "taken.webp")
        .await.unwrap_err().to_string();
    assert!(err.contains("镜像任务"), "源端落在镜像的树里，同样该被挡：{err}");
    assert!(files_in(&into).is_empty(), "被挡的那一次不许在目标目录留下东西：{:?}", files_in(&into));
    drop(held);
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_browser_copy_between_two_unrelated_directories_still_runs_while_the_mirror_holds() {
    // 上一面的**反面半边**（少了它，"越挡越宽"那种实现也能全绿——I3 说的互相假死同样是缺陷）。
    // 两端都在夹具根的兄弟目录里，与对齐那两棵树（`fx.src/` 与 `decklocal:` 整棵）不相包。
    let (fx, runner, _g) = browser_tree(&["x.webp"]).await;
    let from = outside(&fx, "outside-a");
    let into = outside(&fx, "outside-b");
    std::fs::write(from.join("pick.webp"), b"x").unwrap();
    let held = runner.mutex().acquire_scopes(&mirror_scopes(&runner), "镜像任务").await.unwrap();
    let r = runner.browser_copy(&fwd(&from), "pick.webp", &fwd(&into), "moved.webp").await.unwrap();
    assert_eq!(r.done, 1, "{r:?}");
    assert_eq!(files_in(&into), vec!["moved.webp"], "{:?}", files_in(&into));
    drop(held);
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn every_browser_action_leaves_its_own_log() {
    // I7：`{动作}-{时间}.log` 落进面板的日志目录，事后能认出这是哪一次。
    let (fx, runner, _g) = browser_tree(&["pick.webp"]).await;
    let into = outside(&fx, "logs-target");
    runner.browser_copy("decklocal:", "pick.webp", &fwd(&into), "d.webp").await.unwrap();
    let logs = files_in(&fx.root().join("logs"));
    assert!(logs.iter().any(|f| f.starts_with("复制-")), "动作日志没落地：{logs:?}");
    runner.shutdown_rcd().await.unwrap();
}

// ---------------------------------------------------------------- Task 11：按名字的删除与唯一的动手入口

#[tokio::test]
async fn deleting_the_visible_set_removes_exactly_those_names_and_reports_both_counts() {
    // I7 要的"三样"：预览清单 + 实际计数 + 是否一致。这里断言的是数据形状，不是日志文案。
    let (fx, runner, _g) = browser_tree(&["a.webp", "b.webp", "c.webp"]).await;
    let p = runner.browser_preview_delete("decklocal:", "", vec![]).await.unwrap();
    assert_eq!(p.doomed, vec!["a.webp".to_string(), "b.webp".to_string(), "c.webp".to_string()]);
    assert_eq!(p.doomed_ratio, "3/3", "100% 的形状要照实报，但**不拿它拒绝**（I2b）");
    assert_eq!(p.total_in_prefix, 3);
    let r = runner.browser_run(&p.token).await.unwrap();
    assert_eq!((r.has_list, r.confirmed, r.done, r.agrees), (true, 3, 3, true), "{r:?}");
    assert_eq!(files_in(&fx.dst()), Vec::<String>::new(), "该删的删完：{:?}", files_in(&fx.dst()));
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_full_prefix_delete_by_name_is_still_executable_even_though_it_is_one_for_one() {
    // I2b 那道口子的**存在证明**：删掉某处 100% 的对象必须能执行。
    // 没有这条，实现者照 I2 字面复用对齐的比例守卫（`doomed × RATIO_DEN < total`）
    // 就做出一个永远点不动的按钮，而拒绝文案还会说"这更像事故不像清理"。
    let (_fx, runner, _g) = browser_tree(&["only.webp"]).await;
    let p = runner.browser_preview_delete("decklocal:", "", vec![]).await.unwrap();
    assert_eq!(p.doomed.len(), 1);
    assert_eq!(p.doomed_ratio, "1/1", "整棵都在这里了，比例那句要照实说");
    let r = runner.browser_run(&p.token).await.unwrap();
    assert_eq!(r.done, 1, "100% 的形状被守卫挡住了，就是 I2b 说的那个永远点不动的按钮");
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_change_between_preview_and_run_refuses_and_deletes_nothing() {
    let (fx, runner, _g) = browser_tree(&["a.webp", "b.webp"]).await;
    let p = runner.browser_preview_delete("decklocal:", "", vec![]).await.unwrap();
    seed(&fx.dst(), &["c.webp"]);   // 预览之后、确认之前多了一个
    let err = runner.browser_run(&p.token).await.unwrap_err().to_string();
    assert!(err.contains("2") && err.contains("3"), "两个数都要说出来：{err}");
    assert_eq!(files_in(&fx.dst()), vec!["a.webp", "b.webp", "c.webp"],
        "对不上就一个都不动：{:?}", files_in(&fx.dst()));
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn one_named_object_needs_its_own_confirmation_and_spares_its_neighbour() {
    let (fx, runner, _g) = browser_tree(&["pick.webp", "neighbour.webp"]).await;
    let p = runner.browser_preview_delete("decklocal:", "", vec!["pick.webp".into()]).await.unwrap();
    assert_eq!(p.doomed, vec!["pick.webp".to_string()], "名单里的就是名单里的，不多不少");
    assert_eq!(p.action, "删掉这 1 个".to_string(),
        "确认层的标题要写清这次是几个（I1：范围是显式数据，不靠操作员记忆）：{}", p.action);
    assert_eq!(p.doomed_ratio, "1/2", "只点中一个的时候，比例要说清分母：{}", p.doomed_ratio);
    let r = runner.browser_run(&p.token).await.unwrap();
    assert_eq!((r.confirmed, r.done), (1, 1));
    assert_eq!(files_in(&fx.dst()), vec!["neighbour.webp"],
        "只该删被点中的那一个：{:?}", files_in(&fx.dst()));
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_name_that_is_not_in_what_we_just_listed_is_refused_at_preview() {
    // `browser_preview_delete` 里那道"名单必须全在这次看到的集合里"的分支。
    // 少了这一格，"点三个、界面上只剩两个"就变成"按名字自己造一份清单"——
    // 那正是这份凭证不该授权的事。计划没排这一条，本任务补上（不然这个分支没人跑过）。
    let (_fx, runner, _g) = browser_tree(&["pick.webp"]).await;
    let err = runner.browser_preview_delete("decklocal:", "",
        vec!["pick.webp".into(), "ghost.webp".into()]).await.unwrap_err().to_string();
    assert!(err.contains("ghost.webp"), "要点名是哪一个不在清单里：{err}");
    assert!(err.contains("不作数"), "说的是这次授权不发，不是'找不到'：{err}");
    // 反面半边：全部都在的时候照样发得出来凭证（少了这句，拒一切的实现也能绿）。
    let p = runner.browser_preview_delete("decklocal:", "", vec!["pick.webp".into()]).await.unwrap();
    assert_eq!(p.doomed, vec!["pick.webp".to_string()], "{:?}", p);
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_second_run_with_the_same_token_finds_no_authorization() {
    // ⚠ 这条的形状是被**破坏实验**教出来的（S3 那一枪原本打不红，读数记在 build-notes）：
    // 一次**成功**的删除之后，凭证在登记表里本来就已经被 `invalidate_writes` 抹掉了
    // ——那时候"第二次拿不到授权"为真，但为真的是作废那条路，不是 `consume` 那一条。
    // 要单独钉"凭证花一次就没了"，必须走一次**没做成**的确认：动手前比对先拒（不写、不作废），
    // 此时登记表里若还留着这份清单，第二次就会拿到同一句"对不上"而不是"不作数"。
    let (fx, runner, _g) = browser_tree(&["a.webp", "b.webp"]).await;
    let p = runner.browser_preview_delete("decklocal:", "", vec![]).await.unwrap();
    seed(&fx.dst(), &["c.webp"]);           // 让动手前那一步先拒绝：不写任何东西
    let first = runner.browser_run(&p.token).await.unwrap_err().to_string();
    assert!(first.contains("变了") && first.contains("一个都没动"), "第一次拿到的是复核不过那句：{first}");
    let err = runner.browser_run(&p.token).await.unwrap_err().to_string();
    assert!(err.contains("不作数"), "凭证花第二次要说'不作数'：{err}");
    assert!(!err.contains("对不上"),
        "第二次不能再报一遍复核结果——那份授权已经不在这张表里了：{err}");
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_browser_delete_voids_a_mirror_preview_on_the_same_tree() {
    // I3 第 3 条的**浏览器侧**证人：操作员手里那份"要删这 2 个"的清单，
    // 在有人清空同一棵树之后还能"确认执行"，就是"一把锁"退化成口号的地方。
    // 顺序是这条的要害：先 preview_mirror、再浏览器删、最后确认对齐。
    // 换了序就变成在测"重列比对"而不是"作废"。
    let (fx, runner, _g) = browser_tree(&["a.webp", "b.webp", "stale.webp"]).await;
    // 对齐那份清单必须**真有待删的东西**：src 给 a、b，dst 多一个 stale ⇒ doomed = {stale}。
    seed(&fx.src(), &["a.webp", "b.webp"]);
    let mirror = runner.preview_mirror("m").await.unwrap();
    assert_eq!(mirror.doomed, vec!["stale.webp".to_string()],
        "先确认那份清单立起来了：{:?}", mirror.doomed);
    let drop_it = runner.browser_preview_delete("decklocal:", "", vec![]).await.unwrap();
    assert_eq!(drop_it.doomed.len(), 3, "浏览器看到的是三个（含 stale）");
    runner.browser_run(&drop_it.token).await.unwrap();
    let err = runner.run_mirror("m", &mirror.token).await.unwrap_err().to_string();
    assert!(err.contains("不作数"), "被浏览器清空之后，旧清单必须不作数：{err}");
    assert!(!err.contains("不一样"), "作废与复核是两条判据，别混成一句：{err}");
    runner.shutdown_rcd().await.unwrap();
}

// ---------------------------------------------------------------- 三条护栏各自的证人

/// 每个动作一份日志（I7）。这一条钉的是"一份"与"落在夹具的日志目录里"，
/// 而**不**是脱敏（那是 `TaskLog` 自己的证人，`tests/tasklog.rs`）。
#[tokio::test]
async fn each_action_opens_its_own_log_under_the_directory_the_panel_is_already_running_in() {
    let (fx, runner, _g) = browse_setup().await;
    let want_dir = runner.log_dir();
    let log_dir = std::path::Path::new(&want_dir);
    assert_eq!(want_dir, fwd(&fx.root().join("logs")),
        "日志目录必须是夹具那一份（R68：跑在夹具里就要看得见它）");

    let first = runner.browser_open_log("下载").await.unwrap();
    let second = runner.browser_open_log("下载").await.unwrap();
    assert_ne!(first.path(), second.path(),
        "同一个动作的两次执行不许写进同一份文件，那正是「一个动作一份」的反面");
    for log in [first.path(), second.path()] {
        assert_eq!(log.parent(), Some(log_dir), "日志要落在 log_dir 里：{}", log.display());
        let name = log.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("下载-"), "文件名前缀就是动作名（I7 要说得出这是哪一次）：{name}");
        assert!(log.exists(), "开出来就要在磁盘上，不是等第一行才建：{}", log.display());
    }
    // 不同动作各开一份：前缀不同就是不同文件，不需要靠时间戳隔开。
    let other = runner.browser_open_log("清空").await.unwrap();
    assert!(other.path().file_name().unwrap().to_string_lossy().starts_with("清空-"),
        "{}", other.path().display());
}

/// 存在性预检（I2c）。三格都要钉：撞名拒、没有就放行、**取不到不许当成"没有"**。
/// 第三格是这里唯一会骗人的一格：`rclone` 对一个撞名的写入回 200（F10），
/// 所以"把它当成错误"不是保守，是唯一诚实的读法；而 `Err → Ok(None)` 那种"顺手兜住"
/// 会把"打错的远端名"读成"这一侧没有同名对象"，然后放行一次覆盖。
#[tokio::test]
async fn the_existence_precheck_refuses_a_name_already_there_and_never_reads_a_failure_as_absent() {
    let (_fx, runner, _g) = browse_setup().await;

    // 撞名那一只：夹具里 `a.webp` 真实存在（seed 写的是 1 个字节）。
    let err = runner.refuse_if_target_exists("decklocal:", "a.webp").await.unwrap_err().to_string();
    assert!(err.contains("已经有一个同名文件"), "要说是面板拦的：{err}");
    assert!(err.contains("一个字节都没写"), "拦下来就必须真的没动：{err}");
    assert!(err.contains("1 B"), "大小要带出来，那是操作员认出'是不是我要的那个'的依据：{err}");
    assert!(err.contains("decklocal:a.webp"), "位置要点名（I1）：{err}");
    assert!(!err.contains("rclone"), "归因不许冒认成 rc 的拒绝：{err}");

    // 没有那一格：放行。
    runner.refuse_if_target_exists("decklocal:", "gone-9f3c.webp").await
        .expect("没有同名对象就该放行");

    // 取不到那一格：远端名不存在 ⇒ 抛出错误，不许变成"放行"。
    let err = runner.refuse_if_target_exists("nodecknosuchremote:", "a.webp").await
        .expect_err("stat 拿不到答案时必须抛出，不能当成'这里没有同名对象'");
    let say = err.to_string();
    // 绊线钉的是**屏上那句撞名话术的本名**（`refuse_if_target_exists` 说的是「同名文件」）。
    // 以前这里否定的是一个仓里根本不存在的串，于是一条空证：无论怎么改话术它都不会红。
    assert!(!say.contains("已经有一个同名文件"), "这一格不是撞名，不许复用撞名那句：{say}");
    assert!(say.contains("nodecknosuchremote"), "打错的远端名要原样点名：{say}");
}

/// 锁申领的单位是**所在目录**，且算出来的就是对齐任务那同一套坐标系（裁定 R90/R97/R99）。
/// 这一条只钉"算得对不对"；"挡住没挡住"由同文件那几条活证人钉（每条浏览器写动作都申领这些 Scope）。
#[tokio::test]
async fn the_lock_units_are_the_directories_actually_touched_and_share_the_jobs_coordinate_system() {
    let (_fx, runner, _g) = browse_setup().await;
    let scopes = runner.browser_lock_scopes(&[("decklocal:", "sub/deep/x.webp")]);
    assert_eq!(scopes.len(), 1, "一次动作的一端交一棵树：{:?}", scopes);
    // 所在目录，不是对象本身：把 parent_of 摘掉，这条会变成对象级的那一棵。
    assert_eq!(scopes[0].key(), "decklocal:#sub/deep", "键要点得到那个目录：{:?}", scopes[0].key());
    assert_ne!(scopes[0].key(), "decklocal:#sub/deep/x.webp", "对象级的键挡不住一个改名撞进来");

    // 同一目录里的两个名字 → 同一棵树（这才叫"互相踩得着"）。
    let pair = runner.browser_lock_scopes(&[("decklocal:", "sub/a.webp"), ("decklocal:", "sub/b.webp")]);
    assert_eq!(pair[0].key(), pair[1].key(), "同目录的两个动作必须抢同一把锁：{:?}", pair);

    // 与任务侧同一个坐标系：这条夹具任务的远端端点是 `decklocal:` 的根，
    // 而根是所有前缀的祖先 ⇒ 往它下面任何一层写都该被判成冲突（R90 那一格）。
    let job_scopes = runner.job("j").expect("夹具里有 j").write_scopes();
    assert!(scopes[0].conflicts(&job_scopes[0]) || scopes[0].conflicts(&job_scopes[1]),
        "浏览器往任务正在写的那棵树里写，必须判成冲突：{scopes:?} vs {job_scopes:?}");
    // 反面半边：同根下的邻居目录不是同一棵树，不许越挡越宽（那是 I3 说的互相假死）。
    assert!(!browser_scope("decklocal:", "other").conflicts(&browser_scope("decklocal:", "sub/deep")),
        "邻居目录互相不该挡");
}
// ---------------------------------------------------------------- Task 12：目录动作与清空整个前缀

/// 一棵**带排除规则**的树：rcd 启动带这些规则，`Defaults.exclude` 是同一份（夹具里
/// `setup_with_conf_and_exclude` 存在的理由就是让这两条同源，分开给会造出假场景）。
async fn browser_tree_with_rules(rules: &[&str]) -> (Fx, rclone_deck_lib::runner::Runner, DaemonGuard) {
    let fx = Fx::new();
    seed(&fx.dst(), &["visible.txt", "hidden.log"]);
    let jobs = vec![job_at("m", JobKind::Sync, &fwd(&fx.src()), &fx.remote(""))];
    let (runner, guard) = setup_with_conf_and_exclude(
        fx.root(), Some(fx.conf()), jobs, rules.iter().map(|s| s.to_string()).collect()).await;
    (fx, runner, guard)
}

/// 点中的那个位置对应的锁键（`holder` 要的是字符串）。坐标系只有 `browser_scope` 一处定义。
fn scope_of(fs: &str, prefix: &str) -> String { browser_scope(fs, prefix).key() }

#[tokio::test]
async fn clearing_a_prefix_is_refused_while_exclude_rules_are_active() {
    // R74/I8 的**行为**证人，同时是 F11 那个现象在产品里的落点：
    // 有规则时"清空整个前缀"会删没显示出来的东西，所以这一档必须点不动。
    let (fx, runner, _g) = browser_tree_with_rules(&["*.log"]).await;
    let err = runner.browser_preview_purge("decklocal:", "sub").await.unwrap_err().to_string();
    assert!(err.contains("1 条排除规则"), "{err}");
    assert!(err.contains("没显示出来"), "{err}");
    assert!(err.contains("删掉这一层看到的"), "要指出能用的那条通路（按钮上那个名字逐字对上）：{err}");
    // 被拒的那一次一个东西都没动（F11 的后果是真实的，所以这条不能省）。
    assert_eq!(files_in(&fx.dst()), vec!["hidden.log", "visible.txt"], "{:?}", files_in(&fx.dst()));
    // 而**同一棵树**上按名字的删除照样可用 —— 闸门关的是 purge，不是整条删除通路。
    let p = runner.browser_preview_delete("decklocal:", "", vec!["visible.txt".into()]).await.unwrap();
    assert_eq!(p.doomed, vec!["visible.txt".to_string()], "{:?}", p);
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn clearing_a_prefix_works_when_nothing_is_hidden_and_removes_the_whole_layer() {
    // 反面半边：规则数为 0 时这道闸不拦，purge 的核对靠"动手前重列一遍"。
    let (fx, runner, _g) = browser_tree(&["gone.webp", "also-gone.webp"]).await;
    runner.browser_mkdir("decklocal:", "sub").await.unwrap();
    seed(&fx.dst().join("sub"), &["one.webp", "two.webp"]);
    let p = runner.browser_preview_purge("decklocal:", "sub").await.unwrap();
    assert_eq!(p.doomed, vec!["sub/one.webp".to_string(), "sub/two.webp".to_string()], "{:?}", p.doomed);
    assert_eq!(p.doomed_ratio, "2/2", "100% 照实报，但它不是拒绝的理由（I2b）：{}", p.doomed_ratio);
    let r = runner.browser_run(&p.token).await.unwrap();
    assert_eq!((r.has_list, r.confirmed, r.done, r.agrees), (true, 2, 2, true), "{r:?}");
    assert!(!fx.dst().join("sub").exists(), "purge 连目录本身一起删是实测形状（F12）");
    assert!(fx.dst().join("gone.webp").exists(), "只该动被选中的那一层：{:?}", files_in(&fx.dst()));
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn the_root_of_either_side_is_never_a_clear_target() {
    // F12 + I2b③：两侧的根一律拒绝。两种根形各说一句，不用"或"糊过去
    // （一次拒绝只可能来自一种形状，而两句话术长得不一样 —— 断"或"等于两条都没钉）。
    let (fx, runner, _g) = browser_tree(&["a.webp"]).await;
    let err = runner.browser_preview_purge("decklocal:", "").await.unwrap_err().to_string();
    assert!(err.contains("不能清空整个远端"), "远端那一支要说清拒的是根：{err}");
    assert!(err.contains("整个远端"), "{err}");
    assert!(!err.contains("排除规则"), "这一档拒的是隐式根，不是过滤器：{err}");

    let local = fwd(&fx.root());
    let err = runner.browser_preview_purge(&local, "").await.unwrap_err().to_string();
    assert!(err.contains("不能清空整个本地目录") && !err.contains("远端"), "本地那一支不能套用远端那句：{err}");
    assert!(err.contains("整个本地目录"), "{err}");

    // 反面半边：根下面那一层照旧走得通（这道闸不是"什么都不许清"）。
    std::fs::create_dir_all(fx.dst().join("layer")).unwrap();
    std::fs::write(fx.dst().join("layer").join("x.webp"), b"x").unwrap();
    let p = runner.browser_preview_purge("decklocal:", "layer").await.unwrap();
    assert_eq!(p.doomed, vec!["layer/x.webp".to_string()], "{:?}", p.doomed);
    runner.shutdown_rcd().await.unwrap();
}

#[test]
fn the_purge_count_comes_from_rclone_not_from_our_list() {
    // "清单条数"与"实际删除条数"是两个数，取错一个就是冒充（I7 那句"是否一致"就恒真了）。
    // 本地 purge 太快，端到端造不出两者不等的活场景（成功路径上它们必然相等），
    // 所以把**取值**抽成纯函数单测它。理由写在 `purge_done_count` 的文档里。
    let mut s = empty_stats();
    s.deletes = 7;
    s.deleted_dirs = 3;
    assert_eq!(purge_done_count(&s), 7, "必须是 rclone 的 deletes");
    assert_eq!(purge_deleted_dirs(&s), 3, "目录数是另一个数，别与 deletes 混用");
    assert_eq!(purge_done_count(&empty_stats()), 0, "空读数就是 0，不许凭空造一个");
}

#[tokio::test]
async fn a_purge_releases_the_lock_only_after_the_job_is_over() {
    // R33 的不变量在浏览器这一侧同样成立：guard 必须活得比 rclone 的 job 长。
    // 本地 purge 太快，"job 还在跑时锁还在"那个瞬间观察不到，所以这条只钉两件事：
    // 跑完之后锁已放开（不许永久锁住这棵树），以及这一档真的经过指向那棵树的申领。
    let (fx, runner, _g) = browser_tree(&["a.webp"]).await;
    runner.browser_mkdir("decklocal:", "keep").await.unwrap();
    seed(&fx.dst().join("keep"), &["x.webp", "y.webp"]);
    let key = scope_of("decklocal:", "keep");
    let p = runner.browser_preview_purge("decklocal:", "keep").await.unwrap();
    let r = runner.browser_run(&p.token).await.unwrap();
    assert_eq!(r.done, 2, "done 必须是 rclone 报的 deletes：{r:?}");
    assert!(r.agrees);
    assert!(r.has_list, "清空走的是预览那一条，清单是有的：{r:?}");
    assert!(runner.mutex().holder(&key).await.is_none(),
        "跑完了还占着锁，这棵树就永远动不了了：{key}");
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn rmdir_goes_through_the_same_preview_and_token_as_every_other_delete() {
    // 「删空目录」以前是点一下就删 —— 那是这一面上唯一一条不经过清单与凭证的删除路径，
    // 而 R78 那句"界面上没有第二条直接删的路"在它身上是假的。现在它和其它删除同一条路：
    // 预览（空不空在**发凭证之前**判）→ 一次性凭证 → `browser_run`。
    let (fx, runner, _g) = browser_tree(&[]).await;
    runner.browser_mkdir("decklocal:", "emptydir").await.unwrap();

    // 非空那一次连凭证都发不出来：判在动手之前，不是等 rclone 回一个 500 再翻译。
    runner.browser_mkdir("decklocal:", "full").await.unwrap();
    seed(&fx.dst().join("full"), &["inside.webp"]);
    let err = runner.browser_preview_rmdir("decklocal:", "full").await.unwrap_err().to_string();
    assert!(err.starts_with("这个目录不是空的"), "话术要先把这一档说明白：{err}");
    assert!(err.contains("1 个文件"), "里面有几个是我们自己数出来的，要说：{err}");
    assert!(!err.contains("operations/rmdir"), "内部方法名不上界面：{err}");
    assert!(fx.dst().join("full").join("inside.webp").exists(),
        "被拒的那一次里面那个东西要在：{:?}", files_in(&fx.dst()));

    // 目录不存在那一次**不许**被读成"它是空的"：读侧把列举的 404 当"这一侧没有东西"（F9/R64），
    // 写侧照抄就是把"没看见"当成"是空的"，本仓最忌的那一族。
    let gone = runner.browser_preview_rmdir("decklocal:", "never-existed").await.unwrap_err().to_string();
    assert!(gone.contains("不在了"), "要说得出它压根不在：{gone}");
    assert!(!gone.contains("这个目录不是空的"), "把没认出的失败演成一句知道的话：{gone}");
    assert!(!gone.contains("operations/rmdir"), "{gone}");

    // 空的那一次：预览交出清单（就那一个目录名）与凭证，动手只走 `browser_run`。
    let p = runner.browser_preview_rmdir("decklocal:", "emptydir").await.unwrap();
    assert_eq!(p.doomed, vec!["emptydir".to_string()], "清单上就是那一个目录：{p:?}");
    assert_eq!(p.action, "删空目录", "确认层的标题：{p:?}");
    let r = runner.browser_run(&p.token).await.unwrap();
    assert!(!fx.dst().join("emptydir").exists(), "空目录该删掉：{:?}", files_in(&fx.dst()));
    // 这一档现在**有**清单可核对（就一条），所以两个数都报 —— 与建目录/一次复制那一档不同。
    assert_eq!((r.has_list, r.confirmed, r.done, r.agrees), (true, 1, 1, true), "{r:?}");
    assert_eq!(r.note, "只删了那一个空目录", "那一句承诺要作为数据走出去：{r:?}");
    assert!(r.target.contains("emptydir"), "回显要说得出删的是哪个位置：{r:?}");

    // 凭证单次消费：同一张再交一次就是"这份确认不作数了"。
    let again = runner.browser_run(&p.token).await.unwrap_err().to_string();
    assert!(again.contains("不作数"), "{again}");

    // 预览与确认之间被塞进东西 → 动手前那一遍判出来，一个都没动，锁也不留在桌上。
    runner.browser_mkdir("decklocal:", "race").await.unwrap();
    let p2 = runner.browser_preview_rmdir("decklocal:", "race").await.unwrap();
    seed(&fx.dst().join("race"), &["late.webp"]);
    let raced = runner.browser_run(&p2.token).await.unwrap_err().to_string();
    assert!(raced.contains("一个都没动"), "被拒要说清没有半完成：{raced}");
    assert!(fx.dst().join("race").join("late.webp").exists(),
        "里面那个东西要在：{:?}", files_in(&fx.dst()));
    assert!(runner.mutex().holder(&scope_of("decklocal:", "race")).await.is_none(),
        "被拒的一次不许把锁留在桌上");
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn mkdir_is_idempotent_so_a_second_click_does_not_error() {
    // F14 实测：对已存在的目录再建一次还是 200。这条钉的是"我们不把幂等做成报错"。
    let (fx, runner, _g) = browser_tree(&[]).await;
    runner.browser_mkdir("decklocal:", "twice").await.unwrap();
    runner.browser_mkdir("decklocal:", "twice").await.unwrap();
    assert!(fx.dst().join("twice").is_dir());
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn every_write_entry_refuses_a_dot_segment_in_the_name_it_was_handed() {
    // 裁定 R110：`browser_list` 那道边界只挡"浏览到哪儿"，动手时交的 `remote` 是界面给的自由串
    // （清单里那个名字只是默认值，不是唯一来源）。配置侧的 parse 也管不到它。
    // 所以**每个动手入口**都要问一遍，用的还是同一个判据 —— 一处一套规矩迟早对不上。
    let (fx, runner, _g) = browser_tree(&["pick.webp"]).await;
    let bad = "a/../b";
    let cases: Vec<(&str, String)> = vec![
        ("copy", runner.browser_copy("decklocal:", "pick.webp", "decklocal:", bad).await
            .unwrap_err().to_string()),
        ("建目录", runner.browser_mkdir("decklocal:", bad).await.unwrap_err().to_string()),
        ("删空目录预览", runner.browser_preview_rmdir("decklocal:", bad).await.unwrap_err().to_string()),
        ("清空预览", runner.browser_preview_purge("decklocal:", bad).await.unwrap_err().to_string()),
        ("删除预览", runner.browser_preview_delete("decklocal:", "", vec![bad.into()]).await
            .unwrap_err().to_string()),
        ("移动预览", runner.browser_preview_move("decklocal:", bad, "decklocal:", "x.webp").await
            .unwrap_err().to_string()),
    ];
    for (which, err) in &cases {
        assert!(err.contains("两个点"), "{which} 没走那道边界：{err}");
        assert!(err.contains("不猜"), "{which} 那句得说清为什么不发：{err}");
    }
    // 反面半边：含点但**不是整段**的名字在同一个入口上照旧走得通
    // （少了这一半，"拒一切"的实现也能全绿）。
    runner.browser_mkdir("decklocal:", "a.b-c").await.unwrap();
    assert!(fx.dst().join("a.b-c").is_dir(), "含点不是点段，必须放行");
    runner.shutdown_rcd().await.unwrap();
}
// ---------------------------------------------------------------- Task 13：改名与移动（movefile）

#[tokio::test]
async fn a_rename_holds_both_sides_and_moves_exactly_one_object() {
    let (fx, runner, _g) = browser_tree(&["old-name.webp", "neighbour.webp"]).await;
    let p = runner.browser_preview_move("decklocal:", "old-name.webp", "decklocal:", "new-name.webp")
        .await.unwrap();
    assert_eq!(p.doomed, vec!["old-name.webp".to_string()], "移动的清单就是那一个名字");
    assert!(p.target.contains("old-name.webp") && p.target.contains("new-name.webp"),
        "I1：两端都要写全（只写目标端的话，「从哪搬走」靠操作员记忆）：{}", p.target);
    let r = runner.browser_run(&p.token).await.unwrap();
    assert_eq!((r.has_list, r.confirmed, r.done, r.agrees), (true, 1, 1, true), "{r:?}");
    assert_eq!(files_in(&fx.dst()), vec!["neighbour.webp", "new-name.webp"],
        "只该动被点中的那一个：{:?}", files_in(&fx.dst()));
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn moving_onto_an_existing_name_is_refused_at_preview_and_nothing_moves() {
    // F10 在这里不是"数据没落地"那么轻：move 撞同名会**删源**而目标一字不改
    //（同大小同 modtime 那一档，实测 10/10）。所以撞名必须问在**领凭证之前**。
    let (fx, runner, _g) = browser_tree(&["a.webp", "b.webp"]).await;
    let err = runner.browser_preview_move("decklocal:", "a.webp", "decklocal:", "b.webp")
        .await.unwrap_err().to_string();
    assert!(err.contains("已经有一个同名文件"), "{err}");
    assert!(err.contains("一个字节都没写"), "拦下来就得真的没动：{err}");
    assert_eq!(files_in(&fx.dst()), vec!["a.webp", "b.webp"],
        "被拒之后两侧都不该动：{:?}", files_in(&fx.dst()));
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_move_cannot_be_confirmed_after_a_panel_write_touched_its_source_side() {
    // 这一条钉的是"凭证的 scopes 覆盖**两端**"。形状是被 R105 那条规矩逼出来的：
    // 原来那版（预览 → 在磁盘上悄悄删掉源 → 确认）红不红跟 scopes 无关 —— 挡住它的是动手前
    // 那次 stat 核对，把源端从 scopes 里拿掉它也照样绿（那正是一条钉不住任何东西的证人）。
    // 现在走的是只可能由 scopes 决定的那条路：**面板自己**在源那一侧写了一次
    //（删掉同一棵树里另一个对象），它对凭证的影响只有"作废"一条路可走。
    let (fx, runner, _g) = browser_tree(&["a.webp", "b.webp"]).await;
    let out = fx.root().join("elsewhere");
    std::fs::create_dir_all(&out).unwrap();
    let p = runner.browser_preview_move("decklocal:", "a.webp", &fwd(&out), "a.webp").await.unwrap();

    // 源那一侧被面板自己写过一次：删掉邻居 `b.webp`（与 `a.webp` 同一棵树，不同对象）。
    let del = runner.browser_preview_delete("decklocal:", "", vec!["b.webp".into()]).await.unwrap();
    runner.browser_run(&del.token).await.unwrap();

    let err = runner.browser_run(&p.token).await.unwrap_err().to_string();
    assert!(err.contains("不作数"),
        "源侧被写过，这份凭证就不该还能确认（钉的是 scopes 里有源端）：{err}");
    assert!(!err.contains("对不上"),
        "不许把「没核对上」演成「作废」——两句话讲的是两件不同的事：{err}");
    assert_eq!(files_in(&out), Vec::<String>::new(),
        "被拒的那一次目标侧也不许多出东西：{:?}", files_in(&out));
    assert!(fx.dst().join("a.webp").exists(), "源要在（它没被搬走）：{:?}", files_in(&fx.dst()));
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_move_refused_by_the_second_existence_check_after_the_credential_is_issued() {
    // 动手前那**第二次**问目标在不在（I2c）。第一问在预览时，中间隔着操作员的一次点击。
    // 这一格必须用**不经面板**的一次外部写入来造：面板自己写目标那一侧会顺手把这份凭证作废
    //（它的 scopes 覆盖两端 —— 上一条证人钉的就是这件事），那就什么都测不到，只会拿到那句"不作数"。
    // 上一版我就踩了这个坑：第二次移动把第一份凭证顶掉了，红的不是我要问的那一句。
    let (fx, runner, _g) = browser_tree(&["a.webp"]).await;
    let out = fx.root().join("target-dir");
    std::fs::create_dir_all(&out).unwrap();
    let p = runner.browser_preview_move("decklocal:", "a.webp", &fwd(&out), "same.webp").await.unwrap();

    // 操作员看着确认框的这段时间里，别的程序往目标位置放了一个同名文件。
    std::fs::write(out.join("same.webp"), b"landed-after-the-preview").unwrap();

    let err = runner.browser_run(&p.token).await.unwrap_err().to_string();
    assert!(err.contains("已经有一个同名文件"), "第二次问话必须拦下来：{err}");
    // 那句拒绝要指得出**是哪个位置**：目标在本地那一侧时，`fs` 结尾没有分隔符，
    // 粘出来的那条路径操作员按它去找是找不到的（I1）。
    assert!(err.contains(&format!("{}/same.webp", fwd(&out).to_lowercase())),
        "拒绝句子中的目标位置要读得出来：{err}");
    assert!(!err.contains("不作数"),
        "这一格测的是「目标又被占了」，不是「凭证过期」——两句不许混：{err}");
    assert!(fx.dst().join("a.webp").exists(), "被拒的那一次源要在（move 会删源！）：{:?}", files_in(&fx.dst()));
    assert_eq!(files_in(&out), vec!["same.webp"], "目标那个文件一个字都不该被盖掉：{:?}", files_in(&out));
    assert_eq!(std::fs::read(out.join("same.webp")).unwrap(), b"landed-after-the-preview",
        "被拦下的那次覆盖没有发生");
    runner.shutdown_rcd().await.unwrap();
}

#[tokio::test]
async fn a_move_whose_source_vanished_outside_the_panel_is_refused_by_the_check() {
    // `run_move` 里"源还在不在"那一次 stat 核对的证人。
    // **为什么还要单独一条**：上面那条（面板自己在源侧写过一次）钉的是凭证的 scopes 覆盖两端，
    // 它红不到这一格；而"源在预览之后被面板之外的东西删掉"这一档，只有这次 stat 拦得住。
    // 两条各钉一道判据 —— 混在一条里就会有一条永远只是顺带通过（R105 那一枪量出来的形状）。
    let (fx, runner, _g) = browser_tree(&["a.webp", "b.webp"]).await;
    let out = fx.root().join("gone-side");
    std::fs::create_dir_all(&out).unwrap();
    let p = runner.browser_preview_move("decklocal:", "a.webp", &fwd(&out), "a.webp").await.unwrap();

    std::fs::remove_file(fx.dst().join("a.webp")).unwrap();   // 面板之外的人删掉了源

    let err = runner.browser_run(&p.token).await.unwrap_err().to_string();
    assert!(err.contains("变了") && err.contains("一个都没动"), "说的是刚才看到的那批已经不是我承诺的那批：{err}");
    assert!(!err.contains("不作数"), "这一档凭证还有效，是核对没过：{err}");
    assert_eq!(files_in(&out), Vec::<String>::new(),
        "被拒的那一次不许在目标侧落下一个文件：{:?}", files_in(&out));
    assert_eq!(files_in(&fx.dst()), vec!["b.webp"], "{:?}", files_in(&fx.dst()));
    runner.shutdown_rcd().await.unwrap();
}


#[tokio::test]
async fn a_local_destination_reads_back_as_the_path_it_actually_writes_to() {
    // I1 的现场形状：目标落在**本地**那一侧时，确认层正文与跑完那句回显必须还是同一条能打开的路径。
    // 这一格只能端到端钉：上面那条纯函数证人看的是 browser_target_label 自己，
    // 而调用点用的是不是它要在真跑一次的 target 上读。
    let (fx, runner, _g) = browser_tree(&["a.webp"]).await;
    let out = fx.root().join("readable-target");
    std::fs::create_dir_all(&out).unwrap();
    let want = format!("{}/a.webp", fwd(&out).to_lowercase());

    let p = runner.browser_preview_move("decklocal:", "a.webp", &fwd(&out), "a.webp").await.unwrap();
    assert!(p.target.contains(&want), "确认层正文第一行要写得出这个位置：{}", p.target);
    assert!(p.target.contains("decklocal:"), "两端都要写全（I1）：{}", p.target);

    let r = runner.browser_copy("decklocal:", "a.webp", &fwd(&out), "a.webp").await.unwrap();
    assert_eq!(r.target, want, "跑完之后那句回显与确认层同一条路径");
    assert!(out.join("a.webp").is_file(), "回显说的那个位置要真的落了东西：{:?}", files_in(&out));
    runner.shutdown_rcd().await.unwrap();
}
