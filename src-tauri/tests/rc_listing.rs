use std::collections::BTreeSet;
use std::path::Path;

use rclone_deck_lib::rc::listing::{
    WalkBudget, case_conflicts, list_files_recursive, normalize_actual, normalize_expected,
    normalize_expected_with_budget,
};
use rclone_deck_lib::rc::{types::ListEntry, ListError, RcClient, RcError};
use wiremock::matchers::{body_string, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(s: &MockServer) -> RcClient {
    RcClient::from_base_url(s.uri(), "u", "p")
}

fn list_body(remote: &str) -> String {
    let enc = |s: &str| {
        s.chars().map(|c| match c {
            ':' => "%3A".to_string(),
            '/' => "%2F".to_string(),
            c => c.to_string(),
        }).collect::<String>()
    };
    // 尾上是整棵树那一档的开关（`opt={"recurse":true,"filesOnly":true,"noModTime":true,
    // "noMimeType":true}` 的 form 编码）。桩匹配的是**完整请求体**，所以这个后缀是
    // "我们真的在请求整棵树的文件"的证据；把它拿掉，下面每一条挂载都桩不上，
    // 整个文件立刻全红——不是静默少测（换形状那一次就靠全红说明了还有谁在跟着走）。
    format!(
        "fs={}&remote={}&opt=%7B%22recurse%22%3Atrue%2C%22filesOnly%22%3Atrue%2C%22noModTime%22%3Atrue%2C%22noMimeType%22%3Atrue%7D",
        enc("r2:img"), enc(remote))
}

fn list_resp(entries: &[(&str, bool)]) -> ResponseTemplate {
    let list: Vec<serde_json::Value> = entries.iter().map(|(p, is_dir)| serde_json::json!({
        "Path": p,
        "Name": p.rsplit('/').next().unwrap_or(p),
        "Size": 1,
        "MimeType": "",
        "ModTime": "2026-09-28T00:00:00Z",
        "IsDir": is_dir,
    })).collect();
    ResponseTemplate::new(200).set_body_json(serde_json::json!({ "list": list }))
}

async fn mount_list(s: &MockServer, remote: &str, entries: &[(&str, bool)]) {
    Mock::given(method("POST"))
        .and(path("/operations/list"))
        .and(body_string(list_body(remote)))
        .respond_with(list_resp(entries))
        .mount(s)
        .await;
}

/// 打出去的 LIST 次数。R2 按请求计费，所以"用了多少请求"本身就是被测行为，
/// 不能只看返回值。
async fn list_calls(s: &MockServer) -> usize {
    s.received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/operations/list")
        .count()
}

/// 整棵树来自**一次**调用：这条同时钉住两件事——深层的名字进得来，以及
/// 返回里的目录条目不会变成"文件"（`IsDir` 必须被滤掉，否则 `covers` 自己会冒充一个对象）。
/// 请求数钉在 1 上：这是那次换法的本体（逐层 BFS 在这份夹具上要按目录数发请求）。
#[tokio::test]
async fn one_call_returns_the_whole_tree_and_directories_are_not_files() {
    let s = MockServer::start().await;
    mount_list(&s, "ideahub", &[
        ("ideahub/x.txt", false),
        ("ideahub/covers", true),
        ("ideahub/covers/a.webp", false),
        ("ideahub/covers/b.webp", false),
    ]).await;

    let files = list_files_recursive(&client(&s), "r2:img", "ideahub").await.unwrap();
    let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["ideahub/covers/a.webp", "ideahub/covers/b.webp", "ideahub/x.txt"]);
    assert_eq!(list_calls(&s).await, 1, "整棵树该一次调用拿完；多出来的就是有人在逐层下探");
}

#[tokio::test]
async fn missing_prefix_surfaces_as_not_found() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/operations/list"))
        .respond_with(ResponseTemplate::new(404)
            .set_body_json(serde_json::json!({ "error": "directory not found" })))
        .mount(&s)
        .await;

    let err = list_files_recursive(&client(&s), "r2:img", "nope").await.unwrap_err();
    assert!(matches!(err, ListError::NotFound), "{err:?}");
}

/// 列举失败必须让整次列举失败，不能交出"已经收到的那部分"。
/// 后者在 gate 那边长得和"桶里就是这些"一模一样——`operations_list` 是 fail-closed 的
/// 就是为了这一刻：空目录是 `{"list":[]}`，"没看见"不是。
#[tokio::test]
async fn a_response_without_a_list_array_fails_the_whole_listing() {
    let s = MockServer::start().await;
    // 200，但没有 list 数组：既不是空目录也不是"这一层没有图"。
    Mock::given(method("POST"))
        .and(path("/operations/list"))
        .and(body_string(list_body("ideahub")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "status": 200 })))
        .mount(&s)
        .await;

    let err = list_files_recursive(&client(&s), "r2:img", "ideahub").await.unwrap_err();
    assert!(matches!(err, ListError::Other(RcError::Rc { .. })), "{err:?}");
}

/// 桶根（`remote=""`）是 spec §3.2 量过、`RcClient` 也显式处理了的事实（`trim_matches('/')`）。
/// 这里把两端钉住：`""` 和 `"/"` 必须是同一次列举，而且返回的 key 仍然相对桶根。
#[tokio::test]
async fn the_bucket_root_remote_lists_the_bucket_root() {
    let s = MockServer::start().await;
    mount_list(&s, "", &[("a.webp", false), ("covers", true), ("covers/b.webp", false)]).await;

    let files = list_files_recursive(&client(&s), "r2:img", "").await.unwrap();
    let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["a.webp", "covers/b.webp"]);

    // 同一个根写成 "/"：请求体里的 remote 必须还是空的，不然桶根就变成一次 404。
    let slashed = list_files_recursive(&client(&s), "r2:img", "/").await.unwrap();
    assert_eq!(normalize_actual(&slashed), normalize_actual(&files));
    assert_eq!(list_calls(&s).await, 2, "两次调用各该打 1 次 LIST");
}

/// rc 在 Windows 的本地后端上会把 `Path` 写成反斜杠（实测）。今天这条只剩**归一化**要钉：
/// 一次调用没有"把反斜杠拼进下一个 `remote=`"那一类二次伤害了，但两侧吃的是同一批 key，
/// 反斜杠进了集合就和正斜杠那侧对不上，于是删除清单少报或多报。
#[tokio::test]
async fn backslash_paths_are_normalized_to_one_coordinate_system() {
    let s = MockServer::start().await;
    mount_list(&s, "ideahub", &[("ideahub\\covers\\a.webp", false)]).await;

    let files = list_files_recursive(&client(&s), "r2:img", "ideahub").await.unwrap();
    assert_eq!(normalize_actual(&files), BTreeSet::from(["ideahub/covers/a.webp".to_string()]));
    assert_eq!(list_calls(&s).await, 1);
}

#[test]
fn normalize_actual_keeps_the_prefix_rclone_already_put_there() {
    let entries = vec![
        ListEntry { path: "ideahub/covers/a.webp".into(), name: "a.webp".into(), size: 1, is_dir: false },
        ListEntry { path: "ideahub/covers".into(), name: "covers".into(), size: 0, is_dir: true },
        ListEntry { path: "ideahub\\covers\\b.webp".into(), name: "b.webp".into(), size: 1, is_dir: false },
        ListEntry { path: "".into(), name: "".into(), size: 0, is_dir: false },
    ];
    assert_eq!(normalize_actual(&entries), BTreeSet::from([
        "ideahub/covers/a.webp".to_string(),
        "ideahub/covers/b.webp".to_string(),
    ]));
}

#[test]
fn normalize_expected_joins_the_prefix() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.webp"), b"x").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/b.webp"), b"x").unwrap();

    assert_eq!(normalize_expected(dir.path(), "ideahub/covers/", &[]).unwrap(), BTreeSet::from([
        "ideahub/covers/a.webp".to_string(),
        "ideahub/covers/sub/b.webp".to_string(),
    ]));
}

#[test]
fn normalize_expected_with_a_bucket_root_prefix_adds_no_slash() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.webp"), b"x").unwrap();

    assert_eq!(normalize_expected(dir.path(), "", &[]).unwrap(),
        BTreeSet::from(["a.webp".to_string()]));
}

#[test]
fn normalize_expected_applies_the_default_excludes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.webp"), b"x").unwrap();
    std::fs::write(dir.path().join("Thumbs.db"), b"x").unwrap();
    std::fs::write(dir.path().join(".DS_Store"), b"x").unwrap();

    let got = normalize_expected(dir.path(), "p/", &["Thumbs.db".into(), "*.DS_Store".into()]).unwrap();
    assert_eq!(got, BTreeSet::from(["p/a.webp".to_string()]));
}

/// 造一个**真的**磁盘重解析点：Windows junction（`mklink /J`）指向 `target`。
/// 选 junction 不选目录符号链接，是因为符号链接要 SeCreateSymbolicLinkPrivilege
/// 或开发者模式（这台机器实测 `os error 1314 客户端没有所需的特权`），而 junction 不要额外权限。
/// junction 的**规范路径**会解析成目标的真实路径，而 `file_type()` 认得出它是重解析点
/// （实测 `sym=true dir=false`）——这两个事实合起来才是环/镜像守卫的地基。
///
/// `link` 要逐段 `join` 拼出来，别写成 `join("sub/name")`：路径里混进正斜杠的话，
/// cmd 的 mklink 会把 `/name` 当成开关，实测报「无效参数 - "link_to_root"」。
fn mklink_junction(link: &Path, target: &Path) {
    assert!(link.parent().unwrap().is_dir(), "junction 的父目录要先存在：{link:?}");
    let out = std::process::Command::new("cmd")
        .arg("/C")
        // 路径原样交给 Command：它自己按 MSVC 规则给带空格的路径加引号。
        .args(["mklink", "/J", &link.display().to_string(), &target.display().to_string()])
        .output()
        .expect("启动 cmd.exe 失败，无法创建 junction");
    assert!(out.status.success(),
        "创建 junction 失败（这台机器上跑不了真实重解析点夹具）：{}",
        String::from_utf8_lossy(&out.stderr));
    // 链接自己必须是重解析点：这是夹具的唯一硬条件，目标存不存在都一样。
    let meta = std::fs::symlink_metadata(link).expect("mklink 报成功，但链接没有重解析点元数据");
    assert!(meta.is_symlink(), "夹具造出来的不是重解析点：{link:?}");
}

/// 指向**存在**的目录的 junction：夹具自己确认它解析得开（`is_dir()` 实测为 true）。
fn junction(link: &Path, target: &Path) {
    mklink_junction(link, target);
    assert!(link.is_dir(), "junction 建好了但没解析成目录：{link:?}");
}

/// 指向**不存在**的目标的 junction——"挂着外部图片盘的那个链接，盘拔了"。
/// 实测：这种条目 `path.is_dir()` 返回 **false**（`file_type()` 仍说 sym=true），
/// 所以"按 is_dir() 分文件还是目录"的写法会把它当成一个文件收进 key 集合。
fn dangling_junction(link: &Path, missing_target: &Path) {
    mklink_junction(link, missing_target);
    assert!(!link.is_dir(), "夹具本该是死链接，结果它解析开了：{link:?}");
}

/// 在另一条线程上跑 `normalize_expected`，并区分它到底怎么没通过。
/// 环没被拦住有两种故障表现：绕出几十个假 key（本机实测），或者干脆永不返回——
/// 后者没法用 assert 观察，所以给一个有界的等待：超时 = 红。
/// （用 channel 而不是 `JoinHandle::join`，因为后者没有超时。）
fn scan_that_must_return(root: &Path, prefix: &str) -> BTreeSet<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let root = root.to_path_buf();
    let prefix = prefix.to_string();
    std::thread::spawn(move || {
        let _ = tx.send(normalize_expected(&root, &prefix, &[]));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        // 本机实测：没有守卫时走的是这一支，交回一份被污染的集合（假 key 见断言话术）。
        Ok(Ok(keys)) => keys,
        Ok(Err(e)) => panic!("normalize_expected 交回了一个 Err：{e}"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!(
            "normalize_expected 在 10 秒内没有返回：环没被拦住，本地扫描又变成永不结束的 walk 了"),
        // 走到这里说明扫描线程自己挂了（它的 panic 打在上面）：同样是"没交出结果"。
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!(
            "扫描线程没交出结果就结束了：环没被拦住"),
    }
}

/// 指回祖先的 junction：`sub/link_to_root -> root`。这是"我的文档被重定向"那类
/// Windows 安装的真实形状，也是个人 vault 里常见的目录结构。
///
/// 断言的集合没变，但 Step 0 之后**理由**变了：真正让 `link_to_root/...` 进不了期望集合的
/// 是"rclone 默认不跟随重解析点"这条实测事实（`build-notes.md`），不是"这个目录我来过没有"。
/// 环守卫（分支栈）现在兜的是不经过重解析点的环——这台机器上造不出来，所以它没有、
/// 也不该假装有一个专属夹具；`build-notes.md` 那张实测表里把这条讲清楚了。
#[test]
fn normalize_expected_does_not_follow_a_junction_back_to_an_ancestor() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.webp"), b"x").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/b.webp"), b"x").unwrap();
    junction(&dir.path().join("sub").join("link_to_root"), dir.path());

    // 期望的集合里一个 `link_to_root/...` 都不该有：那个目录已经被访问过了。
    assert_eq!(
        scan_that_must_return(dir.path(), "p/"),
        BTreeSet::from(["p/a.webp".to_string(), "p/sub/b.webp".to_string()]));
}

/// 互指的 junction 对（`x/to_y -> y`、`y/to_x -> x`）：环里没有"祖先"可言，
/// 按路径前缀判断深浅的写法拦不住它。
///
/// 断言的集合没变，但**理由不一样了**（Step 0 实测）：这两个链接在
/// "rclone 默认不跟随重解析点"那一条上就已经被拦掉了，根本走不到分支栈；
/// 分支栈管的是不经过重解析点的环（见 `list_files_recursive` 对等的那个闸门与文档注释）。
#[test]
fn normalize_expected_does_not_follow_a_pair_of_mutual_junctions() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("x")).unwrap();
    std::fs::create_dir(dir.path().join("y")).unwrap();
    std::fs::write(dir.path().join("x/a.webp"), b"x").unwrap();
    std::fs::write(dir.path().join("y/b.webp"), b"x").unwrap();
    junction(&dir.path().join("x").join("to_y"), &dir.path().join("y"));
    junction(&dir.path().join("y").join("to_x"), &dir.path().join("x"));

    assert_eq!(
        scan_that_must_return(dir.path(), "p/"),
        BTreeSet::from(["p/x/a.webp".to_string(), "p/y/b.webp".to_string()]));
}

/// **非环的镜像**：`root/mirror -> root/sub`（同一个目录被两条路径合法指向，不构成环）。
/// 这是 review 点名的那个缺口——之前整个套件里没有任何一个"合法镜像"夹具，
/// 于是"少算一条路径"这件事从来没有被测到过。
///
/// 策略由 Step 0 的实测决定（`docs/superpowers/evidence/build-notes.md`）：
/// `rclone copy <root> <dst>`（面板默认 flags，不含 `-L/--copy-links`）对这种树报
/// `NOTICE: link: Can't follow symlink without -L/--copy-links`，落进目标目录的只有
/// `sub/...`，一个 `link/...` 对象都没有。所以期望侧必须**也只有** `sub/...`：
/// - 期望侧多出 `mirror/...` → gate 的假"缺失"（挡住一次本可以通过的检查）；
/// - 期望侧少掉 `sub/...` → prune 的假孤儿，而孤儿清单是删除候选。
/// 后一种恰恰是"按先遇到的那条算"的写法会造成的：`mirror` 按字典序排在 `sub` 前面。
#[test]
fn normalize_expected_counts_a_junction_mirror_the_way_rclone_pushes_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::create_dir(dir.path().join("sub/deep")).unwrap();
    std::fs::write(dir.path().join("sub/a.webp"), b"x").unwrap();
    std::fs::write(dir.path().join("sub/deep/c.webp"), b"x").unwrap();
    junction(&dir.path().join("mirror"), &dir.path().join("sub"));

    assert_eq!(
        normalize_expected(dir.path(), "p/", &[]).unwrap(),
        BTreeSet::from([
            "p/sub/a.webp".to_string(),
            "p/sub/deep/c.webp".to_string(),
        ]));
}

/// 指向 **root 外面**的 junction（"vault 里挂一个外部图片目录"那种镜像）：
/// rclone 同样不跟随，所以那个目录的内容一张也不会进桶——期望侧不能出现 `mirror/...`，
/// 否则 gate 会永远报"这些图缺失"，而那些图是本地扫描根本传不上去的东西。
/// （旧写法在这里正好相反：它把 `mirror/z.webp` 当成期望 key 收了进去。）
#[test]
fn normalize_expected_ignores_a_junction_that_points_outside_the_root() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("z.webp"), b"x").unwrap();
    std::fs::write(root.path().join("a.webp"), b"x").unwrap();
    junction(&root.path().join("mirror"), outside.path());

    assert_eq!(
        normalize_expected(root.path(), "p/", &[]).unwrap(),
        BTreeSet::from(["p/a.webp".to_string()]));
}

/// 死链接（目标已经不存在的 junction）：实测 `path.is_dir()` 返回 **false**
/// （`file_type()` 仍报 sym=true），按 `is_dir()` 分文件/目录的写法会把它当成一个
/// **文件**收进 key 集合——桶里不会有这个对象（实测 rclone 直接 NOTICE 跳过），
/// 于是它变成 gate 一条永远消不掉的"缺失"。
#[test]
fn normalize_expected_does_not_turn_a_dead_junction_into_a_file_key() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.webp"), b"x").unwrap();
    dangling_junction(&dir.path().join("gone"), &dir.path().join("no-such-dir"));

    assert_eq!(
        normalize_expected(dir.path(), "p/", &[]).unwrap(),
        BTreeSet::from(["p/a.webp".to_string()]));
}

/// 封顶封的是**工作量**，不是深度：`read_dir` 次数（= 打开过的目录层数）一到预算就整次失败。
/// 为什么必须有这一条：环一旦被上面的判据漏掉、或者镜像链条本来就有限深但深到 2^m
/// （实测 rclone 自己 `-L` 撞环就是 255 次列举 / 128 个对象后死在路径长度上，
/// 见 `build-notes.md`），"少给几个 key"在下游是删除候选，所以这里**不交半个集合**。
#[test]
fn normalize_expected_fails_loud_at_the_read_dir_budget() {
    let dir = tempfile::tempdir().unwrap();
    // root/{a_dir/k, B_dir/k}：要打开的层数是 root, a_dir, a_dir/k, B_dir, B_dir/k = 5。
    for d in ["a_dir", "a_dir/k", "B_dir", "B_dir/k"] {
        std::fs::create_dir(dir.path().join(d)).unwrap();
    }

    // 默认预算下同一棵树是绿的（先把"预算不是因为树本身太大"这件事钉住）。
    assert_eq!(normalize_expected(dir.path(), "p/", &[]).unwrap(), BTreeSet::new());

    let err = normalize_expected_with_budget(
        dir.path(), "p/", &[], WalkBudget { read_dirs: 3, keys: usize::MAX }).unwrap_err();
    let msg = err.to_string();
    // 报错要说出撞的是哪个上限、上限是多少：只有"扫描失败"四个字的话，
    // 操作员没法判断该调预算还是该去查目录树。
    assert!(msg.contains("read_dir") && msg.contains("3"), "{msg}");
    // 走到 Err 这一支本身就是说：一个 key 都没交回来。
}

/// key 数上限同理：镜像链条真正烧掉的是"吐出来的 key 数"。
#[test]
fn normalize_expected_fails_loud_at_the_key_budget() {
    let dir = tempfile::tempdir().unwrap();
    for n in 1..=5 {
        std::fs::write(dir.path().join(format!("{n}.webp")), b"x").unwrap();
    }

    assert_eq!(normalize_expected(dir.path(), "p/", &[]).unwrap().len(), 5);

    let err = normalize_expected_with_budget(
        dir.path(), "p/", &[], WalkBudget { read_dirs: usize::MAX, keys: 2 }).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("key") && msg.contains("2"), "{msg}");
}

/// 扫描顺序必须是"这棵树"的函数，不是"文件系统枚举"的函数。
///
/// 可观察量是"撞预算时正要打开的那一层"：实测本机对同时含 `a_dir` 和 `B_dir` 的目录，
/// `read_dir` 给的是不区分大小写的顺序 `["a_dir", "B_dir", ...]`，而字节序是
/// `["B_dir", "a_dir"]`（`build-notes.md`）。预算掐在 3，两种顺序就会停在不同的层上。
///
/// 为什么这条重要（不是洁癖）：不排序时"哪条拼写先被枚举到"决定期望集合长什么样，
/// prune 的预览和它自己的复核可能在**没动过的盘**上给出两个不同的孤儿清单，
/// Task 9 的漂移守卫就会报一次"桶变了"。
#[test]
fn normalize_expected_walks_in_a_fixed_order_not_the_filesystems_order() {
    let dir = tempfile::tempdir().unwrap();
    for d in ["a_dir", "a_dir/k", "B_dir", "B_dir/k"] {
        std::fs::create_dir(dir.path().join(d)).unwrap();
    }
    let budget = WalkBudget { read_dirs: 3, keys: usize::MAX };

    let msg = normalize_expected_with_budget(dir.path(), "p/", &[], budget).unwrap_err().to_string();
    // 排序以后先走 B_dir（字节序里 'B' < 'a'），第 4 次打开才是 a_dir。
    assert!(msg.contains("a_dir"), "停在的那一层不是字典序算出来的那一层：{msg}");

    // 同一棵树再扫一次，停在同一处。
    let again = normalize_expected_with_budget(dir.path(), "p/", &[], budget).unwrap_err().to_string();
    assert_eq!(again, msg);
}

/// 两侧落在同一个坐标系里才是 gate 能用的东西：Task 5 做的是两个集合的差集，
/// 实际侧多拼一次前缀、或期望侧少拼一次，结果都是"全部缺失"。
/// 这里让远端和本地表示同一批文件，断言两侧归一化出来的 key 集合完全相同。
#[tokio::test]
async fn both_sides_land_in_the_same_coordinate_system() {
    let s = MockServer::start().await;
    mount_list(&s, "ideahub", &[
        ("ideahub/x.webp", false),
        ("ideahub/covers", true),
        ("ideahub/covers/a.webp", false),
    ]).await;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("x.webp"), b"x").unwrap();
    std::fs::create_dir(dir.path().join("covers")).unwrap();
    std::fs::write(dir.path().join("covers/a.webp"), b"x").unwrap();

    // dst = r2:img/ideahub 时：remote_for_list() 是 "ideahub"，prefix() 是带斜杠的 "ideahub/"。
    let files = list_files_recursive(&client(&s), "r2:img", "ideahub").await.unwrap();
    assert_eq!(
        normalize_actual(&files),
        normalize_expected(dir.path(), "ideahub/", &[]).unwrap());
}

/// 报错必须说得出是**哪一个**目录：一棵 vault 里可能有上万个目录，std 那句
/// "拒绝访问。(os error 5)" 不带路径，操作员没法去查那一个。
/// root 解析不出来（`src` 打错了、外接盘没挂）是这台机器上确定造得出来的一种，
/// 不需要 ACL 夹具（`icacls /deny` 造的目录 TempDir 删不掉，见 `build-notes.md` 的说明）。
#[test]
fn normalize_expected_names_the_directory_that_failed() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-src");
    let err = normalize_expected(&missing, "p/", &[]).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err:?}");
    assert!(err.to_string().contains("no-such-src"),
        "错误里没点名是哪个目录，操作员没法去查它：{err}");
}

/// 语义对等的夹具：**同一棵树**就是 `docs/superpowers/evidence/build-notes.md`
/// §Task 4 修复轮（下半）拿去一条条量 `rclone copy --exclude` 的那棵树（16 个文件、10 层目录）。
/// 每个条目都为了区分某一件事实而存在：
/// - `a/b/oldx.webp` vs `oldtop.webp` vs `a/oldbox/inner.webp` → `*` 跨不跨 `/`、模式在不在任意层生效；
/// - `node_modules/…` 与 `a/b/node_modules/x.webp` → 目录规则剪不剪子树、剪不剪任意层的同名目录；
/// - `data.db/inside.webp` + `top.db` + `a/b/keep.db` → `*.db`（文件）与 `*.db/`（目录）的分工；
/// - `.git/…` → 带点的目录名（review 举的那个例子）；
/// - `covera.webp` / `coverb.webp` → 字符类 `[ab]` 是 rclone 认而我们不实现的写法。
const EXCLUDE_TREE: &[&str] = &[
    ".git/config",
    ".git/objects/aa.txt",
    "a/b/deep/deepkeep.webp",
    "a/b/keep.db",
    "a/b/node_modules/x.webp",
    "a/b/oldx.webp",
    "a/keep.webp",
    "a/oldbox/inner.webp",
    "covera.webp",
    "coverb.webp",
    "data.db/inside.webp",
    "node_modules/deep/z.webp",
    "node_modules/y.webp",
    "oldtop.webp",
    "rootfile.webp",
    "top.db",
];

/// 建出那棵树，返回临时目录 + "整棵树都数进来"时的期望 key 集合（前缀 `p/`）。
fn exclude_tree() -> (tempfile::TempDir, BTreeSet<String>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut all = BTreeSet::new();
    for rel in EXCLUDE_TREE {
        let path = dir.path().join(rel.replace('/', "\\"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"x").unwrap();
        all.insert(format!("p/{rel}"));
    }
    (dir, all)
}

/// 期望集合 = 整棵树减去实测"被 rclone 挡在桶外"的那些 key。
/// 逐条 `gone` 都取自 build-notes 那张表，不是推理出来的。
fn expect_excluding(all: &BTreeSet<String>, gone: &[&str]) -> BTreeSet<String> {
    all.iter()
        .filter(|k| !gone.iter().any(|g| *k == &format!("p/{g}")))
        .cloned()
        .collect()
}

/// 按实测语义扫一遍（unwrap：这些模式全都是我们能表达的，报错就该是红）。
fn scan_excluding(root: &Path, patterns: &[&str]) -> BTreeSet<String> {
    let owned: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
    normalize_expected(root, "p/", &owned)
        .unwrap_or_else(|e| panic!("exclude {patterns:?} 不该让扫描失败：{e}"))
}

/// 实测：`--exclude "*/old*"` 只挡住 `a/b/oldx.webp` 一个。
/// 三个候选都"看起来像"：根上的 `oldtop.webp`（没有那层目录）、`a/b/oldx.webp`（一层目录 + old 开头）、
/// `a/oldbox/inner.webp`（`*/old*` 字面上能对上目录 `a/oldbox`）。落地的只有第二个 →
/// `*` **不跨 `/`**，而且模式匹配的是**对象自己的路径**，不是它某一层目录。
/// 这一条是 finding 里唯一被实测证实的前提（旧实现三个都会挡住，挡多的那些在 gate 那边是假缺图）。
#[test]
fn exclude_star_does_not_cross_a_slash_the_way_rclone_measured_it() {
    let (dir, all) = exclude_tree();
    assert_eq!(
        scan_excluding(dir.path(), &["*/old*"]),
        expect_excluding(&all, &["a/b/oldx.webp"]));
}

/// 实测：`--exclude "old*"`（不带 `/`）挡住 `oldtop.webp` **和** `a/b/oldx.webp`，
/// 但**不**挡 `a/oldbox/inner.webp`。也就是"不带斜杠的模式在任意一层按名字匹配"。
#[test]
fn exclude_without_a_slash_matches_the_base_name_at_every_level_as_rclone_does() {
    let (dir, all) = exclude_tree();
    assert_eq!(
        scan_excluding(dir.path(), &["old*"]),
        expect_excluding(&all, &["oldtop.webp", "a/b/oldx.webp"]));
}

/// 实测：`--exclude "b/*"` 挡住 `a/b/keep.db` 与 `a/b/oldx.webp`——**带着 `/` 的模式也是在任意层对齐的**
/// （旧写法只拿它比对整条相对路径，`a/b/…` 这种两层的根就对不上）；
/// 但 `a/b/deep/deepkeep.webp` 照样落地：`b/*` 只吃一层，**不**剪目录。
#[test]
fn exclude_with_a_slash_still_aligns_at_any_level_but_only_one_level_deep() {
    let (dir, all) = exclude_tree();
    assert_eq!(
        scan_excluding(dir.path(), &["b/*"]),
        expect_excluding(&all, &["a/b/keep.db", "a/b/oldx.webp"]));
}

/// 实测：`--exclude "/b/*"` 一个都不挡（树里根层没有 `b`），`--exclude "/a/b/*"` 挡 `a/b/keep.db`
/// 与 `a/b/oldx.webp`。前导 `/` = 只从 src 根算起。这一条旧实现完全没有：它把 `p.contains('/')`
/// 拿去比对整条 rel，等价于"永远锚定"，于是操作员写的 `b/*` 我们挡不住而 rclone 挡得住
/// （期望侧多 key = 假缺图，方向安全，但仍然是两套匹配器对同一串字符串给出两个答案）。
#[test]
fn exclude_leading_slash_anchors_the_pattern_at_the_src_root_only() {
    let (dir, all) = exclude_tree();
    assert_eq!(scan_excluding(dir.path(), &["/b/*"]), all.clone(), "锚定的模式不该在 a/ 下面命中");
    assert_eq!(
        scan_excluding(dir.path(), &["/a/b/*"]),
        expect_excluding(&all, &["a/b/keep.db", "a/b/oldx.webp"]));
}

/// 实测：`--exclude "node_modules/"`（**尾部斜杠**）把 `node_modules/y.webp`、`node_modules/deep/z.webp`
/// 和任意层的 `a/b/node_modules/x.webp` 全挡住；`.git/` 同样把 `.git/config` 与 `.git/objects/aa.txt` 挡住。
/// 这才是 rclone 真正的"目录规则"，也是 review 的 Important 3 要的那个能力：一条模式就能圈住整棵子树。
#[test]
fn exclude_trailing_slash_prunes_the_whole_directory_subtree() {
    let (dir, all) = exclude_tree();
    assert_eq!(
        scan_excluding(dir.path(), &["node_modules/"]),
        expect_excluding(&all, &[
            "node_modules/y.webp", "node_modules/deep/z.webp", "a/b/node_modules/x.webp"]));
    assert_eq!(
        scan_excluding(dir.path(), &[".git/"]),
        expect_excluding(&all, &[".git/config", ".git/objects/aa.txt"]));
}

/// 实测：目录规则也允许通配——`--exclude "a/*/"` 剪掉 `a/b`（连 `a/b/deep`、`a/b/node_modules` 一起）
/// 和 `a/oldbox`，但 `a/keep.webp` 照常落地。
#[test]
fn exclude_wildcard_directory_rule_prunes_every_matching_directory() {
    let (dir, all) = exclude_tree();
    assert_eq!(
        scan_excluding(dir.path(), &["a/*/"]),
        expect_excluding(&all, &[
            "a/b/deep/deepkeep.webp", "a/b/keep.db", "a/b/node_modules/x.webp",
            "a/b/oldx.webp", "a/oldbox/inner.webp"]));
}

/// 实测：`--exclude "node_modules"`（不带尾部斜杠）**一个文件都不挡**，16 个全部落地；
/// `--exclude "data.db"` 也不挡 `data.db/inside.webp`（`*.db` 那一条同理，见下一个测试）。
/// **这条推翻了 review 的第二个前提**："裸目录名对 rclone 是整棵树、对我们只是一个同名文件"——
/// 量出来两边都只认对象自己的路径，所以"幻影 expected key → gate 永远报缺图"那个场景不存在。
/// 留这条测试是为了钉住"别把不剪子树的写法改成剪子树"：那种改动会把 rclone 真会推的 key 从期望侧拿掉，
/// 而少掉的 key 在 prune 那边就是删除候选。
#[test]
fn exclude_bare_directory_name_excludes_nothing_because_rclone_excludes_nothing() {
    let (dir, all) = exclude_tree();
    assert_eq!(scan_excluding(dir.path(), &["node_modules"]), all.clone());
    assert_eq!(scan_excluding(dir.path(), &["data.db"]), all.clone());
}

/// 实测：`--exclude "*.db"` 挡 `top.db` 与 `a/b/keep.db`（两个**文件**），
/// 不挡 `data.db/inside.webp`；写成 `--exclude "*.db/"` 才挡得住那个目录里的东西。
/// 文件规则与目录规则的分工就在尾部斜杠上，两边都必须照抄。
#[test]
fn exclude_matches_files_or_directories_by_the_trailing_slash_not_by_guessing() {
    let (dir, all) = exclude_tree();
    assert_eq!(
        scan_excluding(dir.path(), &["*.db"]),
        expect_excluding(&all, &["top.db", "a/b/keep.db"]));
    assert_eq!(
        scan_excluding(dir.path(), &["*.db/"]),
        expect_excluding(&all, &["data.db/inside.webp"]));
}

/// Important 3 的另一半：**剪掉的目录不能再下探**。可观察量是"打开过几层目录"：
/// `read_dirs: 1` 只允许打开 root 这一层，`node_modules/` 命中就得当场收住。
/// （没被剪掉时下探要 3 层，预算会把它炸出来——所以两个断言一起看才是"停下"这件事。）
#[test]
fn exclude_pruned_directory_is_not_walked_into_at_all() {
    let dir = tempfile::tempdir().unwrap();
    for rel in ["keep.webp", "node_modules/y.webp", "node_modules/deep/z.webp"] {
        let path = dir.path().join(rel.replace('/', "\\"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"x").unwrap();
    }
    let budget = WalkBudget { read_dirs: 1, keys: usize::MAX };
    let owned: Vec<String> = vec!["node_modules/".to_string()];
    assert_eq!(
        normalize_expected_with_budget(dir.path(), "p/", &owned, budget).unwrap(),
        BTreeSet::from(["p/keep.webp".to_string()]));

    // 同一棵树、同一个预算，模式去掉：要打开 3 层，预算就必须炸。
    let err = normalize_expected_with_budget(dir.path(), "p/", &[], budget).unwrap_err().to_string();
    assert!(err.contains("read_dir") && err.contains("1"), "{err}");
}

/// 实测：NTFS 名字不区分大小写，但 rclone v1.75.1 的过滤器**区分**——
/// `--exclude "NODE_modules/"` 一个都不挡。所以我们按字节比是对的；
/// 这条不是补覆盖率，是"别顺手 to_lowercase"的守卫（真发生镜像大小写不一致时，
/// 两边必须给出同一个答案，否则期望侧多出来的那批 key 就是 gate 永远消不掉的缺图）。
#[test]
fn exclude_matching_is_case_sensitive_because_rclone_is() {
    let (dir, all) = exclude_tree();
    assert_eq!(scan_excluding(dir.path(), &["NODE_modules/"]), all.clone());
    assert_eq!(scan_excluding(dir.path(), &["Old*"]), all.clone());
}

/// rclone 认、而我们**故意**不实现的写法：`**`（跨 `/`）、`[ab]`（字符类）、`{a,b}`（或）、
/// `?`（单字符）、`\`（转义）、`/`（整棵树）——全部实测过 rclone 会照办（见 build-notes 那张表）。
/// 面板只有 `*` 一档，所以这些模式**报错**，不静默当字面量：静默不匹配就是"期望侧和桶不等"，
/// 而这里的错法还多一层——`[ab]`/`{a,b}` 这类模式 rclone 是**真的**少推对象的，
/// 我们若按字面量不匹配就会把那些对象留在期望集合里 → gate 永远报缺图（挡住一次本可以通过的检查），
/// 而 `**` 那类更会在 prune 侧留下没人删的孤儿。报错的方向：一次都不给结论。
#[test]
fn exclude_fails_loud_on_rclone_syntax_we_deliberately_do_not_implement() {
    let (dir, all) = exclude_tree();
    for pattern in ["**/old*", "cover[ab].webp", "{rootfile,top}.webp", "old?.webp", r"old\.webp", "/"] {
        let owned: Vec<String> = vec![pattern.to_string()];
        let err = normalize_expected(dir.path(), "p/", &owned)
            .expect_err("rclone 认而我们表达不了的模式必须报错，不能静默不匹配");
        let msg = err.to_string();
        assert!(msg.contains(pattern), "报错必须点名是模式 {pattern:?}，实际是：{msg}");
        // 话术里要给出能用的写法，否则操作员只能去猜。
        assert!(msg.contains("*"), "{msg}");
    }
    // 空 exclude 不受影响，同一棵树照样全数（上面的 all 就是参照）。
    assert_eq!(normalize_expected(dir.path(), "p/", &[]).unwrap(), all.clone());
}

/// 远端大小写敏感、本地 NTFS 不敏感，所以"只差大小写"的一组 key 只可能出现在远端，
/// 通常是一次改名的残留。gate 不据此失败，但要把它显示出来。
#[test]
fn case_conflicts_groups_keys_that_differ_only_by_case() {
    let keys = BTreeSet::from([
        "p/A.webp".to_string(), "p/a.webp".to_string(), "p/b.webp".to_string(),
    ]);
    let got = case_conflicts(&keys);
    assert_eq!(got, vec![vec!["p/A.webp".to_string(), "p/a.webp".to_string()]]);
}

#[test]
fn case_conflicts_is_empty_when_nothing_shares_a_spelling() {
    let keys = BTreeSet::from(["a.webp".to_string(), "b.webp".to_string()]);
    assert!(case_conflicts(&keys).is_empty());
}

// 对齐的坐标系是"**相对各自根**"（spec §4.1）。`normalize_actual` 交的是桶全 key（gate 要那个），
// 这里要的是把列举根剥掉之后的名字——它必须正好等于 rclone 会删掉的那个名字，
// 也是操作员在目标端能看见的那一个。
// （`BTreeSet` / `ListEntry` 本文件顶部已经 import 过，重复的 use 是 E0252，这里只补新的那一个。）
use rclone_deck_lib::rc::listing::normalize_relative;

fn file(path: &str) -> ListEntry {
    ListEntry { path: path.into(), name: path.rsplit('/').next().unwrap().into(), size: 1, is_dir: false }
}

#[test]
fn normalize_relative_strips_exactly_the_listing_root() {
    let entries = vec![file("imgprompt/a.webp"), file("imgprompt/sub/b.webp"),
                       ListEntry { path: "imgprompt/sub".into(), name: "sub".into(), size: 0, is_dir: true }];
    let got = normalize_relative("imgprompt", &entries).unwrap();
    assert_eq!(got, BTreeSet::from(["a.webp".to_string(), "sub/b.webp".to_string()]));
}

#[test]
fn normalize_relative_at_the_bucket_root_strips_nothing() {
    // 出厂那四条对齐都是桶根，这一档什么都不剥。正因为四条都在这一档，
    // "前缀取错"那一族 bug 用出厂配置**测不出来**——上面那条带子路径的证人不是装饰。
    assert_eq!(normalize_relative("", &[file("top.webp")]).unwrap(),
               BTreeSet::from(["top.webp".to_string()]));
}

#[test]
fn normalize_relative_refuses_a_key_outside_the_listing_root_instead_of_guessing() {
    // 不以列举根开头 = 坐标系对不上。静默收下会交出一份范围错的删除清单，
    // 所以给不出结论就不给（口径与 `list_files_recursive` 的 fail-closed 一致）。
    let err = normalize_relative("imgprompt", &[file("other/a.webp")]).unwrap_err().to_string();
    assert!(err.contains("不以列举根开头"), "{err}");
}

#[test]
fn a_trailing_slash_on_the_listing_root_does_not_double_up() {
    // `endpoint_of` 已经在解析层剥掉结尾斜杠；这条钉的是"万一有人直接递带斜杠的串进来"，
    // 因为多剥一层少剥一层都会改变哪些 key 被认为在这一侧的根里。
    assert_eq!(normalize_relative("imgprompt/", &[file("imgprompt/a.webp")]).unwrap(),
               BTreeSet::from(["a.webp".to_string()]));
}

#[test]
fn a_key_equal_to_the_listing_root_is_not_a_file() {
    // 两种"空相对名"的来路都要挡住，因为一个空名字不是可删除的对象：
    // ① 列举根自己被当成文件回来（`imgprompt`）；② 根加一个斜杠回来（`imgprompt/`）。
    // 第二条才是 `if !rel.is_empty()` 那个守卫的证人——没有它，删掉那个守卫全套件照样绿。
    assert!(normalize_relative("imgprompt", &[file("imgprompt")]).unwrap().is_empty());
    assert!(normalize_relative("imgprompt", &[file("imgprompt/")]).unwrap().is_empty(),
        "空相对名被当成一个可删的对象收进了集合");
}

#[test]
fn a_lookalike_prefix_that_is_not_under_the_root_is_an_error_not_a_truncated_name() {
    // 只按字符串前缀剥会出这种事：`imgpromptx/a.webp` 以 `imgprompt` 开头，却不在它下面。
    // 剥错了就交出一个"看起来对"的相对名，而它指向的对象根本不在这一侧的根里——
    // 那正是一份范围错的删除清单。所以这里必须报错，不许返回 `x/a.webp`。
    let err = normalize_relative("imgprompt", &[file("imgpromptx/a.webp")]).unwrap_err().to_string();
    assert!(err.contains("不以列举根开头"), "{err}");
}

/// 裁定 R64 只把 **404** 降级成"这一侧是空集"。这里打的是另一档：**500 不许被读成空**。
///
/// 形状来自实测（`tools/rc-contract-probe.mjs` §4c）：`jobs.toml` 里把 remote 名打错，
/// rc 答 500 `didn't find section in config file ("…")`，而 remote 存在但那个目录没了才答 404。
/// 两者要是都走"空集"那一档，一个拼错的桶名就会被说成"云端什么都没有"，
/// 于是盘点报"全都还没上传"、对齐把对侧整棵交出来当删除清单。
///
/// 这条同时补上了原来那条"更深一层的 404 不许读成空集"的位置：列举换成一次调用之后，
/// "根 404 / 深层 404"这一对区分**没有了对象**（只有一个请求，404 只能来自被问的那一段），
/// 而"404 与 500 必须分家"这条界线是新形状下真正存在的那一条。
#[tokio::test]
async fn an_unknown_remote_is_not_read_as_an_empty_side() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/operations/list"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
            "error": "didn't find section in config file (\"r2\")", "status": 500 })))
        .mount(&s)
        .await;

    let ep = rclone_deck_lib::endpoint::endpoint_of("r2:img/ideahub");
    let err = rclone_deck_lib::runner::checks::side_set(&client(&s), &[], &ep)
        .await
        .expect_err("配置里没有这个 remote 名，不能当成\"这一侧是空的\"放行");
    // 话术要带着 rc 说出的那个名字，操作员才找得出是哪个字打错了。
    assert!(format!("{err}").contains("r2"), "500 的原因没传到话术里：{err}");
}
