//! Task 8 的读侧证人：两道闸的判定、能力位那句话、以及 F16 量到的本地 `fs` 形状。
//!
//! 这一条目标**不起 daemon**（纯函数 + wiremock），所以它没有 `#![cfg(windows)]`，
//! `Cargo.toml` 里也没有它的 `[[test]]` 条目——那套 ceremony 只属于要起真 rclone 的目标
//! （`browser_runner.rs` 是 Task 9 建的那个，`Runner::browser_list` 需要 `Arc<RcdHandle>`，
//! 而 `RcdHandle` 只能由 `RcdSupervisor::start` 交出来，wiremock 桩不出 `tauri::State` 那一层）。
//!
//! `client()` / `mount_json()` 在这里又抄了一份：`tests/rc_client.rs` 与
//! `tests/rc_listing.rs` 各有一份。**再出现一个消费者就把这几份抽进 `tests/wire/mod.rs`**——
//! 本任务不动那两个目标，动它们等于动它们自己的证人。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rclone_deck_lib::endpoint::Endpoint;
use rclone_deck_lib::rc::types::{FsInfo, ListEntry};
use rclone_deck_lib::rc::RcClient;
use rclone_deck_lib::runner::browser::{
    browser_entries, browser_scope, clear_gates, dot_segment_refusal, feature_note, filtered_note,
    local_fs_id, parent_of, usage_of, ShortNote,
};
use rclone_deck_lib::scope::{has_dot_segment, Scope};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(s: &MockServer) -> RcClient {
    RcClient::from_base_url(s.uri(), "u", "p")
}

async fn mount_json(s: &MockServer, p: &str, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(s)
        .await;
}

fn entry(path: &str, name: &str, size: u64, is_dir: bool) -> ListEntry {
    ListEntry { path: path.into(), name: name.into(), size, is_dir }
}

// ---------------------------------------------------------------- 两道闸与那句说明

// I8：rcd 启动的 --exclude 作用于 operations/list，所以浏览器看到的是**过滤后的**桶。
// 而 F11 量到 purge 不受这个过滤器管——"这里就是全部"那句暗示不只是不完整，是**危险的**。
// 所以这句话必须出现、必须说出规则条数，并且必须说清动作的执行单位是名字。
#[test]
fn a_filtered_daemon_says_so_and_how_many_rules_are_hiding() {
    let note = filtered_note(&["Thumbs.db".into(), "*.DS_Store".into()]);
    assert!(note.contains("2 条排除规则"), "条数要出来，操作员才知道规则不是零：{note:?}");
    assert!(note.contains("名字"), "要说清动作的执行单位：{note:?}");
    assert_eq!(filtered_note(&[]), ShortNote::none(), "没有规则时不要凭空造一句提示");
    assert!(!note.contains("listed"), "`listed` 永不上界面");
    assert!(!note.contains("exclude"), "内部旗标名不上界面（BANNED 那一族的口径）：{note:?}");
}

#[test]
fn clearing_a_prefix_is_offered_only_when_nothing_is_hidden_and_not_at_a_root() {
    // 两道闸都在数据层，而且**一次算完**（R74 的规则 + F12 的根）：后台给"能不能 + 那一句"，
    // 界面不许自己数规则，也不许从"能不能"反推"为什么"。
    let (ok, note) = clear_gates(&[], "r2:", "img");
    assert!(ok && note.is_empty(), "有前缀、无规则 ⇒ 给清空，句子留空：{note:?}");
    let (ok, note) = clear_gates(&["Thumbs.db".into()], "r2:", "img");
    assert!(!ok, "有排除规则时清空前缀会删没显示出来的东西（F11）");
    assert!(note.contains("1 条排除规则"), "规则那一档的原因要说得出：{note:?}");
    let (ok, note) = clear_gates(&[], "r2:", "");
    assert!(!ok, "站在远端根上不给清空（F12：alias→本地连目录本身一起删）");
    assert!(note.contains("远端"), "远端的根要说成远端：{note:?}");
    let (ok, note) = clear_gates(&[], "D:/vault", "");
    assert!(!ok && note.contains("本地目录"), "本地那一侧的根要说成本地目录，不能由调用方传词：{note:?}");
    // 桶名与整块本地目录都算"有前缀"：这一档判的是参数，不是 Scope（裁定 R112）。
    assert!(clear_gates(&[], "r2:img", "2026").0);
    // `.` 就是"这一层自己"：函数自己不说谎，不靠调用点先挡点段（那两个调用点确实也挡）。
    assert!(!clear_gates(&[], "r2:", ".").0, "把 `.` 当成有前缀 = purge 打在整个远端上");
    // 根的**说法**由形状决定：`r2:img` 是远端里的一整桶，不是本地目录。
    assert!(clear_gates(&[], "r2:img", "").1.contains("远端"), "{:?}", clear_gates(&[], "r2:img", ""));
}

#[test]
fn feature_bits_explain_and_never_disable() {
    // F6 + F14 是同族事实：alias→本地实测 `Copy=false` 而 copyfile 成功、`Purge=false` 而 purge 真删。
    // 这四个位的形状由 F16 在**真 daemon** 上量到（`tools/rc-contract-probe.mjs` §7：
    // 本地 fs 与 alias→本地回的是同一份 `Copy=false / Purge=false / FilterAware=true`），
    // 这里钉的是那份读数翻成人话之后不许变成"不能做"。
    let mut features = HashMap::new();
    features.insert("Copy".to_string(), false);
    features.insert("Purge".to_string(), false);
    features.insert("FilterAware".to_string(), true);
    let info = FsInfo { features, name: "local".into(), root: "//?/D:/x".into() };
    let note = feature_note(&info);
    assert!(note.contains("没报"), "要说'这个后端没报这个能力位'，不是'不能做'：{note:?}");
    assert!(note.contains("排除规则"), "FilterAware=true 正是那句过滤提示的来源（F11）：{note:?}");
    assert!(!note.contains("Purge=false"), "内部位名与 `键=值` 形状都不许上界面：{note:?}");
    // 空表（后端什么都没报）不该凭空造一句话。
    let none = FsInfo { features: Default::default(), name: "s3".into(), root: String::new() };
    assert_eq!(feature_note(&none), ShortNote::none(), "没报任何东西就什么都别说，也别断'全都支持'");
}

#[test]
fn the_parent_of_an_object_name_is_the_directory_it_sits_in() {
    // 这里只交出"所在目录"那一段的**字符串形状**。谁拿它去申领锁是 Task 9 的事，而那一格交出去
    // 的键必须是 `browser_scope(fs, parent_of(remote))` 算出来的（理由见 browser.rs 里 `parent_of`
    // 的文档）——本任务里没有任何东西执行"锁打在所在目录上"，所以这里不钉那一句。
    assert_eq!(parent_of("pick.webp"), "");
    assert_eq!(parent_of("in/pick.webp"), "in");
    assert_eq!(parent_of("in/deep/pick.webp"), "in/deep");
    assert_eq!(parent_of(""), "");
}

// ---------------------------------------------------------------- F16：本地 fs 的那一档

#[test]
fn a_local_entry_carries_the_spelling_f16_measured_and_only_that_one() {
    // F16（探针 §7 两条 PASS）：`D:\x\y` 与 `D:/x/y` 列出的是**同一棵树**、同一批 `Path`。
    // 所以左列交出的 `id` 是一个**转换**（正斜杠那一档），不是 `to_string_lossy()` 的原样：
    // 转正斜杠与 `Scope::norm` 的仓库内部规范同向，而 `describe()` 那句回显不会拼出
    // `D:\x\y/sub` 这种两种分隔符混在一起的读法。
    assert_eq!(local_fs_id(Path::new(r"D:\tmp\f16\tree")).as_deref(), Some("D:/tmp/f16/tree"));
    // 尾部那个分隔符探针也量了（"带尾斜杠的 fs 列出的是同一棵树"），合并它是为了让同一个目录
    // 在左列只有一行，不是因为 rclone 会拒绝它。
    assert_eq!(local_fs_id(Path::new(r"D:\tmp\f16\tree\")).as_deref(), Some("D:/tmp/f16/tree"));
    // 下面几种是 F16 **没量到**的形状，所以不发（`None`）——不是"rclone 会报错"（没量过就不说）。
    // 逐条说清**拦下它的是哪一步**：M6 拆成三枪之后，每一枪只打得到四道判据里的一道，
    // 这几句的账要照那个落点记，不能写成"形状闸门把它们都挡了"（那句不真）。
    // UNC 转换后是 `/srv/share/img`、相对路径是 `srv/share`，两个都**没有冒号**，
    // 在 `split_once(':')?` 那一步就返回 `None`，根本走不到形状闸门；而探针量到的那一档 `/D:/x`
    // 答的是"list 与 size 回 404、fsinfo 却回 200 并介绍一个别的根"，那种回答没法诚实地当读数用。
    assert_eq!(local_fs_id(Path::new(r"\\srv\share\img")), None, "UNC 没量到 ⇒ 不发");
    assert_eq!(local_fs_id(Path::new("srv\\share")), None, "相对路径没量到 ⇒ 不发");
    // 只有一个盘符：冒号有、分隔符也有，拦下它的是**后半截压完是空的**那一步（`rest.is_empty()`）。
    assert_eq!(local_fs_id(Path::new("D:\\")), None, "只有一个盘符（整块盘）没量到 ⇒ 不发");
    // 盘符后面不跟分隔符是另一种东西（Windows 里它跟着**当前目录**走），也不在量过的那一族里。
    // 这一句才是形状闸门里 `tail.starts_with('/')` 那道判据真正拦到的那一格。
    assert_eq!(local_fs_id(Path::new("D:tree")), None, "盘符相对写法没量到 ⇒ 不发");
    // 闸门剩下两道判据各钉一句——补之前**没有任何证人**能让它们变红（M6 的 ②③ 两枪一枪一道）：
    // 冒号前面那一位必须正好是**一个字母**，长一点、短一点、不是字母的都不是本地盘。
    assert_eq!(local_fs_id(Path::new("1:/x")), None, "非字母的盘符位不发：`1:` 不是盘符");
    assert_eq!(local_fs_id(Path::new("ab:/x")), None, "多字符的盘符位不发：`ab:` 那种根是远端名");
}

#[test]
fn an_entry_is_named_by_its_full_path_because_a_basename_is_not_an_identity() {
    // F16 的那条控件实测：递归列表里 `pick.webp` 与 `x/pick.webp` 的 `Name` **都是** `pick.webp`。
    // 而 Task 9+ 的写动作按**名字**执行（I2c：名字必须是操作员点中的那一个），
    // 所以交出去的那一个名字必须是 `Path`（fs 根内的相对名，写动作直接拿它当 `remote`），
    // 而不是会撞在一起的那一段。目录在前、大小写不敏感地按名字排。
    let entries = browser_entries(vec![
        entry("x/pick.webp", "pick.webp", 3, false),
        entry("pick.webp", "pick.webp", 5, false),
        entry("x", "x", 0, true),
        entry("Zebra.txt", "Zebra.txt", 1, false),
    ]);
    assert_eq!(
        entries.iter().map(|e| (e.name.as_str(), e.size, e.is_dir)).collect::<Vec<_>>(),
        [("x", 0, true), ("pick.webp", 5, false), ("x/pick.webp", 3, false), ("Zebra.txt", 1, false)]
    );
    // 两个同名的东西在这份清单里是两个**不同**的条目（上面的 `Name` 相同时才看得出这一格）。
    assert_eq!(entries.iter().filter(|e| e.name.ends_with("pick.webp")).count(), 2);
    // 大小写折叠后**相等**的那一对（`b.webp` 与 `B.webp`）必须由名字本身再比一次定先后。
    // 只折叠就收尾的话，这一档退成"谁先被 rclone 回来谁在前"，同一份清单两次刷新可以换序。
    let folded = browser_entries(vec![
        entry("b.webp", "b.webp", 1, false),
        entry("B.webp", "B.webp", 2, false),
    ]);
    assert_eq!(folded.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["B.webp", "b.webp"],
        "折叠后相等时行序由判据定，不是由谁先回来定：{:?}", folded);
}

#[test]
fn the_usage_numbers_are_the_objects_this_listing_actually_showed() {
    // F16 实测：`operations/size` 在本地这一族上**不看前缀**（问 `sub`、问一个根本不存在的前缀，
    // 三种 fs 形状都回整棵树的 13 B / 4 个，而且都是 200，没有一个错误可接）。
    // 同一份探针量到：**根上**递归列表求和 == `operations/size` 的读数（13 / 4，目录条目不计）。
    // 所以"这里的用量"按已经列到的那一份算——它按构造就是操作员看见的那一批。
    let entries = browser_entries(vec![
        entry("a.txt", "a.txt", 4, false),
        entry("note.log", "note.log", 6, false),
        entry("sub", "sub", 0, true),
        entry("sub/b.txt", "b.txt", 2, false),
        entry("sub/deep", "deep", 0, true),
        entry("sub/deep/c.txt", "c.txt", 1, false),
    ]);
    assert_eq!(usage_of(&entries), (13, 4), "字节与个数都只算文件（那两个目录条目不进数）");
    assert_eq!(usage_of(&[]), (0, 0), "空清单不要凭空造一个用量");
}

#[test]
fn a_browsed_local_tree_is_the_same_scope_the_locks_use() {
    // 读侧那句 `path` 回显与 Task 9 的申领必须来自**同一个坐标系**（裁定 R93：浏览器的写走
    // `acquire_scopes`，交 `Vec<Scope>`）。判据本体在 `Scope::of_endpoint`，这里只是不许再
    // 自己拼一份"这一棵树是谁"：写成 `Scope::remote(fs, prefix)` 会把 `r2:img` 整串当根，
    // 键就成了 `r2:img#2026`，而对齐任务交出的键是 `r2:#img/2026`（桶名在 prefix 里，裁定 R84）
    // ——根不相等 ⇒ 一次浏览器清空躲过它正在写的同一棵树，那正是 R90 要挡的那一格。
    let browsed = browser_scope("D:/vault/img", "");
    let job = Scope::of_endpoint(&Endpoint::Local(PathBuf::from(r"D:\vault\img")));
    assert_eq!(browsed.key(), job.key(), "浏览本地目录与任务写同一棵树时必须是同一个键");
    assert!(browser_scope("D:/vault/img", "2026").conflicts(&job), "点进下一层还是那棵树");
    assert!(!browser_scope("D:/vault/imgprompt", "").conflicts(&job), "长得像的邻居不是同一棵树");

    let cloud = Scope::of_endpoint(&Endpoint::Remote { fs: "r2:img".into(), remote: "2026".into() });
    assert_eq!(browser_scope("r2:img", "2026").key(), cloud.key(), "桶名要在 prefix 里（R84）");
    // 回显那句照旧读得出完整路径（I1），两种根形都不多一个斜杠。
    assert_eq!(browser_scope("r2:img", "2026").describe(), "r2:img/2026");
    assert_eq!(browser_scope("r2:img", "").describe(), "r2:img");
    assert_eq!(browser_scope("D:/vault/img", "2026").describe(), "d:/vault/img/2026",
        "本地那一支走 `Scope::local`，与任务行上那句回显同一个写法（大小写归一在 scope.rs）");
}

// ---------------------------------------------------------------- 边界：名字里的点段

#[test]
fn a_dot_segment_in_a_place_name_is_refused_while_every_real_name_passes() {
    // 评审 I-2（裁定：在边界拒，配置侧与运行时共用同一条判据，见裁定 R98）。
    // 钉的是"**它被拒**"，不是"它等于哪个键"：`img/../other` 与 `other` 在锁用的那个坐标系里
    // 本来就算出两个键（`Scope::norm` 只压空段，`scope.rs` 那段）——那正是缺陷本身。
    // 拒它的理由与 rclone 怎么解析 `..` **无关**，那一格没量过（裁定 R95/R89），
    // 所以这里既不写"rclone 会把它解析掉"，也不写"它会报错"。
    // 判据收在 `scope::has_dot_segment`：一条名字在"能不能进键"上全仓只许有一个答案。
    // 每次问的是**一个名字**（`fs` 与 `prefix` 是分别问的，两处调用同一个判据），
    // 所以这里喂的是它们各自的形状，不是拼起来的那一串。
    for name in ["img/../other", "./img", "a/./b", "sub/./", "D:/a/../b", "..", "."] {
        assert!(has_dot_segment(name), "{name} 含点段，必须拒");
    }
    // 反面半边：判据是"**整段等于**"，不是"含点"。少了这一半就会把正常名字挡在门外。
    for name in [
        "foo..bar.webp", "a.b.webp", "in-progress/2026", "sub/deep/c.webp",
        "D:/a.b/c/note.log", "deckb:", "r2:img/ideahub/covers",
    ] {
        assert!(!has_dot_segment(name), "正常名字必须照旧放行：{name}");
    }
    // 那句拒绝话术（抽成纯函数就是为了让这里够得着）：读得出是哪个位置，且不出现内部名词。
    let say = dot_segment_refusal("deckb:", "img/../other");
    assert!(say.contains("点"), "要说清为什么点不动：{say}");
    assert!(say.contains("deckb:") && say.contains("img/../other"), "要点名是哪个位置：{say}");
    for banned in ["fs", "prefix", "Scope"] {
        assert!(!say.contains(banned), "操作员看到的句子里不许有内部名词 {banned}：{say}");
    }
}

// ---------------------------------------------------------------- 客户端那一层的口径

#[tokio::test]
async fn a_capability_lookup_failure_still_returns_the_listing() {
    // I5/F6：能力位**只是解释**。它取不到时列表必须照常给出——
    // 一个解释性字段不能有权否决操作员要看的东西。
    let s = MockServer::start().await;
    mount_json(&s, "/operations/list", serde_json::json!({"list": [
        {"Path": "a.webp", "Name": "a.webp", "Size": 3, "IsDir": false}]})).await;
    Mock::given(method("POST")).and(path("/operations/fsinfo"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({"error": "boom"})))
        .mount(&s).await;
    // 这里**没有**挂 `/operations/size`：F16 量到它不看前缀（见上面那条证人），
    // 读侧不再问它，挂上来就是给一份没人读的响应做广告。
    // 评审 I-3：这条名字的口径落在**客户端这一层够得着的地方**——那一份列表真的回来了（挂上的
    // mock 真被打中，不是装饰），而 fsinfo 的失败是一个原样交出的 Err。
    // "把那个 Err 吞成空串"是**读侧的组合**（browser_list），那一层要 `Runner`，见 task-8-report.md
    // 的缺口 1——不在这里假装覆盖它。
    let listing = client(&s).operations_list("deckb:", "", rclone_deck_lib::rc::ListShape::Level).await.unwrap();
    assert_eq!(listing.len(), 1, "fsinfo 答 500 不影响 operations/list 给出那一份：{listing:?}");
    assert_eq!((listing[0].path.as_str(), listing[0].size, listing[0].is_dir),
        ("a.webp", 3, false), "回来的条目就是挂上去的那一条：{listing:?}");
    let err = client(&s).operations_fsinfo("deckb:").await.unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");
}
