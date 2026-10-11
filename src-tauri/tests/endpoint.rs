//! 端点模型（裁定 R60）。存在的理由：`fs_for_list` / `remote_part` / `remote_for_list` / `prefix`
//! 四个 helper 都只从 `job.dst` 里找 remote，而对齐的下载方向把 remote 放在 `src`。
//! 前提反了时它们**不报错**，只是安静地交出错的那一侧——而那一侧在下游是一份删除清单。

use std::path::Path;

use rclone_deck_lib::config::jobs::{Job, JobKind, JobsFile};
use rclone_deck_lib::endpoint::{endpoint_of, remote_split, Endpoint};

/// 夹具：内容丰富但纯属虚构的任务表。**为什么不由出厂配置喂**——出厂那份 2026-10-09 起
/// 瘦身成一条示范（见 `state.rs` 的 `DEFAULT_JOBS_TOML` 顶上那段），而这几条证人要的是
/// 多对位置 / 多条对账 / 会删的那几档。理由与形状要求写在夹具文件自己的头上。
const FULL: &str = include_str!("fixtures/full-jobs.toml");

/// 一条最小可用的对齐配置。调用点写的是"路径本来的样子"（单个 `\`），而 TOML 的基本字符串里
/// 反斜杠必须成对写：`\v`、`\a` 不在 TOML 的转义表里，直接插进去报的是解析器的
/// "missing escaped value"，那句子里既没有「恰好一侧」也没有「反斜杠」——证人会去断言一条
/// 语法错误的废话。所以这一层负责把反斜杠翻倍，两侧的 src/dst 都过它。
fn mirror_toml(src: &str, dst: &str) -> String {
    let q = |s: &str| s.replace('\\', r"\\");
    format!(
        "[defaults]\nrclone_bin = \"C:\\\\x\\\\rclone.exe\"\nflags = []\nexclude = []\nlog_dir = \"C:\\\\logs\"\n\n\
         [[job]]\nid = \"m\"\nname = \"镜像 x：以本地为主（删云端多余的）\"\nkind = \"sync\"\nsrc = \"{}\"\ndst = \"{}\"\n",
        q(src), q(dst))
}

#[test]
fn a_remote_on_either_side_parses_the_same_way() {
    let cases = [
        ("r2:img", "r2:img", ""),
        ("r2:img/imgprompt", "r2:img", "imgprompt"),
        ("qiniu:7zj5r8/sub/deep", "qiniu:7zj5r8", "sub/deep"),
    ];
    for (raw, fs, remote) in cases {
        assert_eq!(endpoint_of(raw), Endpoint::Remote { fs: fs.into(), remote: remote.into() }, "{raw}");
    }
}

#[test]
fn a_windows_drive_letter_is_not_a_remote() {
    // `D:` 的 scheme 长度是 1 —— 这正是原 `remote_split` 的判据，口径不许漂。
    for raw in ["D:\\vault\\img-r2", "D:/vault/img-r2"] {
        assert!(remote_split(raw).is_none(), "{raw} 该认成本地路径");
        assert!(matches!(endpoint_of(raw), Endpoint::Local(_)), "{raw}");
    }
}

#[test]
fn a_trailing_slash_never_reaches_the_listing_root() {
    // 实测：`remote` 结尾带斜杠 → 200 + **空列表**，不是 404。于是"这一侧什么都没有"，
    // 对齐的 doomed 会变成对端的全部。不报错，所以必须在这一层就剥掉。
    let Endpoint::Remote { remote, .. } = endpoint_of("r2:img/imgprompt/") else { panic!("该是 remote") };
    assert_eq!(remote, "imgprompt", "结尾斜杠没剥掉：{remote:?}");
}

#[test]
fn a_backslash_after_the_colon_is_reported_not_silently_converted() {
    // rclone 的 remote 路径只用 `/`。原实现是**拒绝**而不是改写——改写等于替一份定义删除范围
    // 的配置猜意图。这个判据现在要两侧都成立（对齐的 src 也可能是 remote）。
    assert_eq!(endpoint_of("r2:img\\ideahub\\covers").stray_backslash(), Some("ideahub\\covers"));
    assert_eq!(endpoint_of("r2:img").stray_backslash(), None);
    assert_eq!(endpoint_of("D:\\vault\\x").stray_backslash(), None, "本地路径的反斜杠不是分隔符问题");
}

#[test]
fn a_mirror_job_needs_exactly_one_remote_side() {
    for (src, dst) in [("D:\\vault\\a", "r2:img"), ("r2:img", "D:\\vault\\a")] {
        JobsFile::parse(&mirror_toml(src, dst)).unwrap_or_else(|e| panic!("合法配置被拒：{e}"));
    }
    // 两侧都是本地：对 rclone 合法，但面板没有"权威的一端"就圈不出删除范围，也正是 §5.1 那个
    // "远端清单退化成再扫一遍本地"的假绿形状。
    let both_local = JobsFile::parse(&mirror_toml("D:\\vault\\a", "D:\\vault\\b")).unwrap_err().to_string();
    assert!(both_local.contains("恰好一侧") && both_local.contains("remote"), "{both_local}");
    let both_remote = JobsFile::parse(&mirror_toml("r2:img", "qiniu:7zj5r8")).unwrap_err().to_string();
    assert!(both_remote.contains("恰好一侧"), "{both_remote}");
    // src 那一侧混进反斜杠同样要拒（原来只看 dst）
    let bad_src = JobsFile::parse(&mirror_toml("r2:img\\a\\b", "D:\\vault\\a")).unwrap_err().to_string();
    assert!(bad_src.contains("反斜杠"), "src 侧的分隔符问题没人管：{bad_src}");
    // 拒绝话术里不许有 Rust 枚举名（R56 第 2 条：那是给操作员看的句子）。
    // 三条句子都要过这一关，只查一条等于另外两条没人看（M4）。
    for msg in [&both_local, &both_remote, &bad_src] {
        assert!(!msg.contains("Sync") && !msg.contains("JobKind"), "{msg}");
    }
}

/// 报错要引用**操作员写进配置的那一整侧**，不是 `stray_backslash()` 交出去的那段尾巴。
/// 探测器只回"第一个反斜杠之后"的文本，于是 `dst = "r2:img/ideahub\covers"` 会被说成
/// 当前是 `"covers"`——那串字在他的文件里找不到，而这是一句关于删除范围的话。
/// 同一条兄弟检查（dst 那侧的旧检查，`jobs.rs` 的 `rejects_backslashes_after_the_remote_name`）
/// 一直是引用整侧的，这里要两侧（src / dst）、两种手滑形状（反斜杠在桶名段、在路径段）都对齐。
#[test]
fn the_backslash_refusal_quotes_the_whole_side_that_was_written() {
    // 形状 A：反斜杠紧跟在桶名后面（没有 `/`，整串都落在 fs 段）。
    // 形状 B：remote 是正常写法、路径里混进一个反斜杠（探测器交出的是尾巴 "covers"）。
    let cases = [
        ("src", "r2:img\\a\\b", "D:\\vault\\a", r"r2:img\\a\\b"),
        ("dst", "D:\\vault\\a", "r2:img\\a\\b", r"r2:img\\a\\b"),
        ("src", "r2:img/ideahub\\covers", "D:\\vault\\a", r"r2:img/ideahub\\covers"),
        ("dst", "D:\\vault\\a", "r2:img/ideahub\\covers", r"r2:img/ideahub\\covers"),
    ];
    for (which, src, dst, quoted) in cases {
        let err = JobsFile::parse(&mirror_toml(src, dst)).unwrap_err().to_string();
        assert!(err.contains(quoted),
            "{which} 侧的拒绝没引用写进配置的那一整侧（应为 {quoted}）：{err}");
        // 探测器的返回值不该出现在"当前是"后面：那才是这次要修的误导。
        assert!(!err.contains(r#""covers""#), "报错把片段当成了整个值：{err}");
    }
}

/// 对齐与它的复制行**关于同一组位置**（顺序无关）。锁按 Scope 冲突判，所以这一对必然互相挡。
/// 集合相等而不是字符串相等：`src=>dst` 与 `dst=>src` 在方向串下是两个键，要靠 `reverse_of`
/// 倒过来才对得上；两棵树的键集合本来就已经把这件事说完了。
fn same_scope_pair(a: &Job, b: &Job) -> bool {
    let mut x: Vec<String> = a.scope_keys();
    let mut y: Vec<String> = b.scope_keys();
    x.sort();
    y.sort();
    x == y
}

#[test]
fn the_four_mirrors_each_pair_with_a_copy_row_so_the_lock_covers_both() {
    // 对齐与同配对的 copy 必须共用同一把锁，否则"一边增量传、一边对齐删"挡不住。
    // 判据换了形状，**没换成更少的判据**：原来 `mutex_key()` 方向敏感，所以"配对"有两种形状
    //（同键 / 反键），这里仍**分开钉、各自点名那一行的 copy**——写成"同键**或**反键"的析取
    // 会让反方向那半边永远不必为真（出厂里每条对齐都先撞上了同键那条），这条注释的理由照旧。
    // Scope 之下两种形状是同一件事（两侧都进键，顺序不进键），但两句各查各的仍然各有牙：
    // 键集合少一侧时（`scope_keys` 只交 dst）反方向那一句立刻红。
    // 第三句是反向控制：不同树的复制行不许共组，否则前两句是恒真而不是判据。
    let f = JobsFile::parse(FULL).unwrap();
    let syncs: Vec<&Job> = f.jobs.iter().filter(|j| j.kind == JobKind::Sync).collect();
    assert_eq!(syncs.len(), 4, "R2/七牛 × 两个方向，实际 {} 条", syncs.len());
    let by_id = |id: &str| f.jobs.iter().find(|j| j.id == id)
        .unwrap_or_else(|| panic!("出厂配置里没有 {id}"));
    for (mirror, same_order_row, reverse_order_row, unrelated_row) in [
        ("r2-mirror-local-wins", "r2-up", "r2-down", "qiniu-up"),
        ("r2-mirror-remote-wins", "r2-down", "r2-up", "qiniu-down"),
        ("qiniu-mirror-local-wins", "qiniu-up", "qiniu-down", "r2-up"),
        ("qiniu-mirror-remote-wins", "qiniu-down", "qiniu-up", "r2-down"),
    ] {
        let m = by_id(mirror);
        assert!(same_scope_pair(m, by_id(same_order_row)),
            "{mirror} 与同方向的 {same_order_row} 不关于同一组位置——一边增量传、一边镜像删挡不住");
        assert!(same_scope_pair(m, by_id(reverse_order_row)),
            "{mirror} 与反方向的 {reverse_order_row} 不关于同一组位置——这一档就是「反方向也罩得住」那半边");
        assert!(!same_scope_pair(m, by_id(unrelated_row)),
            "{mirror} 与 {unrelated_row} 是不同的树，却被判成同一组位置：前两句就成了一句恒真");
    }
}

/// 产出的 API 方向不能被写反（M1）：`endpoint_src()` / `endpoint_dst()` 现在没有消费者，
/// 也就没有证人——把两个函数体调换，这个文件里原来那 8 条证人一条都不会红（破坏验证 (c)
/// 量过），而"哪一侧才是远端"正是这个模块存在的全部理由，也是起点事故的形状。
/// 这里用**出厂配置自己那两行对齐**当下证，不造字符串：上传方向的 remote 在 dst，
/// 下载方向的 remote 在 src，两侧正好互补。
#[test]
fn the_endpoint_helpers_report_the_side_the_remote_actually_is_on() {
    let f = JobsFile::parse(FULL).unwrap();
    let by_id = |id: &str| f.jobs.iter().find(|j| j.id == id)
        .unwrap_or_else(|| panic!("出厂配置里没有 {id}"));

    // 上传对齐（以本地为主，删云端多余的）：本地在 src，远端在 dst。
    let up = by_id("r2-mirror-local-wins");
    assert_eq!(up.endpoint_src().remote_target(), None,
        "上传镜像的 src 是本地目录，不许被当成远端去列举");
    assert_eq!(up.endpoint_dst().remote_target(), Some(("r2:img", "")));
    assert_eq!(up.endpoint_src().as_local(), Some(Path::new("D:\\vault\\img-r2")));
    assert_eq!(up.endpoint_dst().as_local(), None);

    // 下载对齐（以云端为主，删本地多余的）：完全互补——remote 在 src。
    let down = by_id("r2-mirror-remote-wins");
    assert_eq!(down.endpoint_src().remote_target(), Some(("r2:img", "")),
        "下载镜像要列举的是云端那一侧，它住在 src");
    assert_eq!(down.endpoint_dst().remote_target(), None,
        "下载镜像的 dst 是本地目录，把它当远端就是 §5.1 那个假绿");
    assert_eq!(down.endpoint_src().as_local(), None);
    assert_eq!(down.endpoint_dst().as_local(), Some(Path::new("D:\\vault\\img-r2")));
}

#[test]
fn qiniu_has_a_download_row_too() {
    // AHK 是四个方向（两个云各上下），出厂此前只有三个。
    let f = JobsFile::parse(FULL).unwrap();
    assert!(f.jobs.iter().any(|j| j.kind == JobKind::Copy
        && j.src == "qiniu:7zj5r8" && j.dst == "D:\\vault\\imagehosting"), "缺七牛下载");
}
