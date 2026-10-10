//! 设置面的写路径（`config/edit.rs`）。这一片全是**纯函数与内存里的文本**，不起 daemon、不碰网络、
//! 不碰操作员真实的 `jobs.toml` / `rclone.conf`，所以它同时进 `npm test`（带 `--test-threads=1`）
//! 和 `npm run test:fast`。
//!
//! 为什么这些证人钉的是文本而不是界面：这一层唯一的承诺是"**没被点名的那些字节一个都不动**"，
//! 而那件事在界面上完全看不出来——注释没吃掉、行尾没换掉、密钥行没被顺手重排，屏幕上长得一模一样。

use rclone_deck_lib::config::edit::*;
use std::path::{Path, PathBuf};

/// 真机上 `rclone.conf` 的形状：**全 CRLF**、段内有中文注释、白名单之外还有键。
/// 这里不放真凭据，放的是哨兵值——后面密钥那几条证人扫的就是这些串。
const CONF_CRLF: &str = "[qiniu]\r\ntype = qiniu\r\nprovider = qiniu\r\n\
    access_key_id = SENTINEL-AK-8f2b1c\r\nsecret_access_key = SENTINEL-SK-6d44a0\r\n\
    endpoint = cs-zhangjiakou.qiniu.com\r\n\
    # 这行注释是操作员自己写的理由，任何一次保存都不许吃掉\r\n\
    chunk_size = 4194304\r\n\r\n\
    [r2]\r\ntype = s3\r\nprovider = Cloudflare\r\n\
    access_key_id = SENTINEL-AK2-11aa\r\nsecret_access_key = SENTINEL-SK2-22bb\r\n\
    region = auto\r\nendpoint = https://xxx.r2.dev\r\nupload_concurrency = 4\r\n";

/// jobs.toml 的形状：出厂那份就带成段的理由注释（R58/R60/R69 的落点就在那些注释上）。
/// **raw string**：里面是 TOML 的字面串写法（`'D:\logs'`），别让 Rust 的转义再改一遍反斜杠——
/// 断言比的是磁盘上的那串字节，不是源码里的那串字符。
const JOBS_COMMENTS: &str = r#"# rclone-deck 任务定义。这里不放任何密钥。

[defaults]
rclone_bin = 'C:\rclone\rclone.exe'
flags = ["--fast-list"]
exclude = ["*.log"]
log_dir = 'D:\logs'

[[job]]
id = "a"
name = "甲"
kind = "copy"
src = 'D:\one'
dst = "r2:one"

# 下面这条是删除的入口，这几行注释说的是它为什么危险。
[[job]]
id = "b"
name = "镜像：删多余的"
kind = "sync"
src = 'D:\two'
dst = "r2:two"
"#;

fn lines_of(s: &str) -> Vec<Line> {
    split_lines(s)
}

#[test]
fn split_then_join_is_the_original_bytes() {
    for src in [CONF_CRLF, JOBS_COMMENTS, "no trailing newline", "a\n\nb\n"] {
        assert_eq!(join_lines(&lines_of(src)), src, "切一遍拼回来必须一字不差：{src:?}");
    }
}

#[test]
fn replacing_one_line_leaves_every_other_byte_identical() {
    let old = lines_of(CONF_CRLF);
    let idx = 5; // `[qiniu]`=0、type=1、provider=2、access_key_id=3、secret_access_key=4 ⇒ endpoint=5
    assert!(old[idx].text.starts_with("endpoint"), "夹具的行号漂了：{:?}", old[idx]);
    let plan = EditPlan {
        replaced: vec![(idx, "endpoint = s3.us-west-002.cf.r2.dev".into())],
        inserted: vec![],
        deleted: vec![],
    };
    let new = apply_plan(&old, &plan);
    only_these_changed(&old, &new, &plan).unwrap();
    // CRLF 留着：行尾跟着每一行走，不是整文件统一换一遍（spec §10）。
    assert_eq!(new[idx].ending, "\r\n");
    let text = join_lines(&new);
    assert_eq!(text.matches('\r').count(), CONF_CRLF.matches('\r').count(), "行尾数量变了 = 整文件被重写过");
    assert!(text.contains("# 这行注释是操作员自己写的理由"), "注释被吃掉了");
    assert!(text.contains("chunk_size = 4194304"), "白名单之外的键被顺手删了");
}

#[test]
fn deleting_a_job_block_keeps_the_comment_lines_that_follow_it() {
    // 删一块 = 从 `[[job]]` 那一行到**这一块最后一个键行**。块尾之后的注释行留着：
    // 那些正是"为什么这么配"的落点，宁可留下一段没了主人的说明，也不许吃掉。
    let old = lines_of(JOBS_COMMENTS);
    let span = job_span(&old, "b").expect("id = \"b\" 那一块找得到");
    let plan = deletion_plan(&old, span);
    let new = apply_plan(&old, &plan);
    only_these_changed(&old, &new, &plan).unwrap();
    let text = join_lines(&new);
    assert!(text.contains("这几行注释说的是它为什么危险"), "删任务把注释吃了");
    assert!(!text.contains("id = \"b\""), "那一块没删掉");
    assert!(text.contains("id = \"a\""), "另一块被顺手改了");
}

// ---- 破坏枪：证人要能为对的理由红 ----
//
// 上面三条如果只测"改对了的样子"，那它们其实在证明"什么都没做"。下面三条各把一个实现细节
// 掰坏一次，红了才算钉住。Step 6 手动掰的那一把（行尾继承）在这里没有对应测试，
// 因为它的红由 `replacing_one_line_leaves_every_other_byte_identical` 那条的 `\r` 计数承接。

#[test]
fn the_fidelity_witness_bits_when_a_line_is_touched_that_nobody_asked_for() {
    // 方案说定改第 2 行（provider 那行的多余空格），实现在**没人点名的**第 3 行（access_key_id）
    // 尾巴上加了一个空格。断言必须因此红。
    let old = lines_of(CONF_CRLF);
    let sneaky = EditPlan {
        replaced: vec![(2, "provider = qiniu".into())], // 说定的是"把那两个空格规范化掉"
        inserted: vec![],
        deleted: vec![],
    };
    let new = apply_plan(&old, &sneaky);
    // 不篡改时必须过（否则这条证人只是恒红，拦不住任何东西）。
    only_these_changed(&old, &new, &sneaky).unwrap();
    let mut tampered = new.clone();
    tampered[3].text.push(' ');
    let err = only_these_changed(&old, &tampered, &sneaky).unwrap_err();
    assert!(err.to_string().contains("一个字节都不写"), "红话术没说出兜底：{err}");
}

#[test]
fn the_fidelity_witness_bits_when_a_line_disappears_without_being_named() {
    // 上一把枪测的是"有人多点了一个空格"，这一把测另一半：**少了一行**。
    // 少一行是删除类实现最容易犯的形状（区间多包了一行），它必须在这里现形而不是在 diff 里。
    let old = lines_of(CONF_CRLF);
    let plan = EditPlan {
        replaced: vec![(5, "endpoint = new.example".into())],
        inserted: vec![],
        deleted: vec![],
    };
    let mut lost = apply_plan(&old, &plan);
    lost.remove(0); // `[qiniu]` 那一行自己没了
    let err = only_these_changed(&old, &lost, &plan).unwrap_err().to_string();
    assert!(err.contains("一个字节都不写"), "{err}");
}

#[test]
fn an_ambiguous_section_name_is_refused_not_guessed() {
    let dup = format!("{CONF_CRLF}\r\n[qiniu]\r\ntype = s3\r\n");
    let err = section_span(&lines_of(&dup), "qiniu").unwrap_err().to_string();
    assert!(err.contains("2 次"), "要点名出现几次：{err}");
    assert!(err.contains("qiniu"), "要点名是哪个名字：{err}");
    // 反向半边：只有一段时必须给得出区间。
    assert!(section_span(&lines_of(CONF_CRLF), "qiniu").is_ok());
}

#[test]
fn a_missing_trailing_newline_does_not_shift_the_last_line() {
    // 最后一行 ending == "" 那一档：切拼往返仍等长，且它被点名替换后**依然没有行尾**——
    // 一次保存不许顺手给文件末尾补一个换行（那也是一处没点名的改动）。
    let src = "[x]\r\nfoo = bar";
    let old = lines_of(src);
    assert_eq!(old.last().unwrap().ending, "");
    let plan = EditPlan {
        replaced: vec![(1, "foo = baz".into())],
        inserted: vec![],
        deleted: vec![],
    };
    let new = apply_plan(&old, &plan);
    assert_eq!(join_lines(&new), "[x]\r\nfoo = baz");
    only_these_changed(&old, &new, &plan).unwrap();
}

#[test]
fn an_inserted_block_takes_the_ending_the_file_already_uses() {
    // 新任务块落在一份全 CRLF 的文件上：那几行也必须是 CRLF。
    // 混进行尾比换掉行尾更难发现——两份工具都"能读"，而 diff 与 git 会一直显示整段变过。
    let old = lines_of(CONF_CRLF);
    let at = old.len();
    let plan = EditPlan {
        replaced: vec![],
        inserted: vec![(at, vec!["[newone]".into(), "type = s3".into()])],
        deleted: vec![],
    };
    let text = join_lines(&apply_plan(&old, &plan));
    assert!(text.ends_with("type = s3\r\n"), "{:?}", &text[text.len() - 24..]);
    assert_eq!(text.lines().count(), old.len() + 2, "插进去的两行没进区间");
}

#[test]
fn a_comment_line_is_not_a_key_even_though_it_contains_an_equals_sign() {
    // 定位器要是把 `# region = 旧的` 这种注释认成键，就会**改到注释上**并把真值留在原地。
    let with_comment = "# endpoint = 别改我这行\nendpoint = real.example\n";
    let old = lines_of(with_comment);
    assert_eq!(line_key(&old[0].text), None, "注释行被当成键了");
    assert_eq!(line_key(&old[1].text), Some("endpoint"));
    let plan = EditPlan {
        replaced: vec![(1, "endpoint = new.example".into())],
        inserted: vec![],
        deleted: vec![],
    };
    let text = join_lines(&apply_plan(&old, &plan));
    assert!(text.contains("# endpoint = 别改我这行"), "注释被动了");
    assert!(text.contains("endpoint = new.example"), "真值那行没改到");
}

// ---- Task 2：读侧的形状与两份文件的位置 ----

/// 形状判据（证人 2 的第①条）：密钥的**值**在读侧的类型里不存在——不是"藏起来不发"，
/// 是没有那个字段。这条比行为判据便宜，但它挡的是"以后有人加字段时顺手加了值"。
#[test]
fn the_read_shape_has_no_room_for_a_secret_value() {
    let lines = lines_of(CONF_CRLF);
    let block = build_remote_block("qiniu", &lines).expect("夹具里有 [qiniu] 那一段");
    let json = serde_json::to_value(&block).unwrap();
    let s = json.to_string();
    for needle in ["SENTINEL-AK-8f2b1c", "SENTINEL-SK-6d44a0"] {
        assert!(!s.contains(needle), "哨兵密钥上线了：{needle}");
    }
    // "存在"这件事照样交得出来：只有键名。
    let names: Vec<&str> = json["secrets"]["present"].as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(names, ["access_key_id", "secret_access_key"], "{names:?}");
    // 可编辑那一份只有白名单四项：chunk_size / upload_concurrency / access_key_id 都不许在里面。
    let keys: Vec<&str> = block.fields.iter().map(|f| f.key.conf_name()).collect();
    assert_eq!(keys, ["type", "provider", "endpoint", "region"], "{keys:?}");
    // 界面上给的出路是一条可复制的话，而且那句话里没有内部名词。
    assert!(block.secrets.command.contains("rclone config"), "{}", block.secrets.command);
    let said = format!("{}{}", block.secrets.note.short, block.secrets.command);
    for banned in ["rcd", "token", "dump", "config/get"] {
        assert!(!said.contains(banned), "密钥那一格的话里有 {banned}：{said}");
    }
    // 那句 note 只说"这几项不给看也不给改"。「不回显」那六个字是**界面上那颗标签**，
    // 由 `frontend_settings.rs` 钉着：两处各写一遍，屏上就会出现同一句挨着同一句。
    assert!(block.secrets.note.short.contains("密钥"), "{:?}", block.secrets.note.short);
    assert!(!block.secrets.note.short.contains("不回显"),
        "note 里又把界面那句标签抄了一遍：{:?}", block.secrets.note.short);
}

/// 行为判据（证人 2 的第②条）：整份读侧序列化出来的文本里搜不到哨兵——
/// 包括两句 note 的 detail，包括"这一栏列不出来"那一句。
#[test]
fn the_sentinel_never_reaches_any_string_the_read_side_sends_over_the_wire() {
    let dir = tempfile::tempdir().unwrap();
    let files = Files {
        jobs: dir.path().join("jobs.toml"),
        conf: Some(dir.path().join("rclone.conf")),
        // 这一份 rclone **不存在**：版本那一档该走"问不出"，而不是拿别的东西顶上。
        rclone_bin: PathBuf::from(r"C:\rclone\rclone.exe"),
    };
    std::fs::write(&files.jobs, JOBS_COMMENTS).unwrap();
    std::fs::write(files.conf.as_ref().unwrap(), CONF_CRLF).unwrap();
    let view = current(&files, vec![]).unwrap();
    let json = serde_json::to_string(&view).unwrap();
    assert!(!json.contains("SENTINEL"), "读侧整体上线了密钥：{json}");
    assert_eq!(view.defaults.rclone_bin_version, "", "那份 rclone 不存在，版本号是从哪来的");
    assert!(!view.defaults.rclone_bin_exists, "不存在的路径被说成存在");
    // 任务块来自装载侧那一次解析（同一份口径），两条都在、kind 是那三个分支 token 之一。
    assert_eq!(view.jobs.len(), 2, "jobs.toml 里两条任务没读全：{:?}", view.jobs);
    assert_eq!(view.jobs[1].job_kind, JobKindWire::Sync, "第二块 kind 读错了：{:?}", view.jobs[1]);
    assert_eq!(view.jobs[0].name, "甲", "{:?}", view.jobs[0]);
    // 远端两块按**文件里那几段的顺序**：名字就是段名，没有第二份名单可对照。
    let got: Vec<&str> = view.remotes.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(got, ["qiniu", "r2"], "{got:?}");
    assert!(view.remotes[1].fields.iter().any(|f| f.key == RemoteKey::Region && f.value == "auto"),
        "r2 那一段的 region 没读出来：{:?}", view.remotes[1].fields);
    // 这两条路径**要**上线：操作员要核对"面板跑在哪个家"（规程 R68 靠的就是这一格）。
    assert!(view.jobs_path.ends_with("jobs.toml"), "{}", view.jobs_path);
    assert!(view.conf_path.ends_with("rclone.conf"), "{}", view.conf_path);
    assert!(view.conf_note.is_empty(), "一切正常时那句不该有内容：{:?}", view.conf_note);
}

/// 段的名字只有一个来源（那一份文件），所以"名栏列出来了、脚边却挂着这一栏列不出来"
/// 那种两句同时说是结构上不可能的：真窗口抓到过一次，原因是名单来自别处。
#[test]
fn the_remote_names_are_the_config_files_own_sections_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let conf = CONF_CRLF.replace("chunk_size = 4194304", "chunk_size = 4194304\r\n\r\n[qiniu]\r\ntype = s3\r\n");
    std::fs::write(dir.path().join("jobs.toml"), JOBS_COMMENTS).unwrap();
    std::fs::write(dir.path().join("rclone.conf"), &conf).unwrap();
    let files = Files {
        jobs: dir.path().join("jobs.toml"),
        conf: Some(dir.path().join("rclone.conf")),
        rclone_bin: PathBuf::from(r"C:\rclone\rclone.exe"),
    };
    let view = current(&files, vec![]).unwrap();
    // 同名段出现两次：那一段**读不出**（改哪一段说不清），跳过它并点名，另一段照常列出。
    let got: Vec<&str> = view.remotes.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(got, ["r2"], "够不着的那一段混进来了：{got:?}");
    assert!(view.conf_note.contains("这一栏列不出来"), "{:?}", view.conf_note.short);
    assert!(view.conf_note.contains("qiniu"), "要点名是哪一段对不上：{:?}", view.conf_note.detail);
    // 名字里**不会**出现位置形状的串（`fxa:a` 那种是任务里在用的位置，不是段名）。
    for r in &view.remotes {
        assert!(!r.name.contains(':'), "段名里带着位置：{}", r.name);
    }
    // 没有 conf 可问 ⇒ 那一句还在，但只有一句：不许同时端出一栏"列好了"的远端。
    let none = Files { jobs: dir.path().join("jobs.toml"), conf: None,
                       rclone_bin: PathBuf::from(r"C:\rclone\rclone.exe") };
    let view = current(&none, vec![]).unwrap();
    assert!(view.remotes.is_empty() && view.conf_note.contains("这一栏列不出来"),
        "问不出路径却有远端：{:?}", view.remotes);
}

/// `section_names` 那一层：按文件顺序、同名只交一次、`[[job]]` 那种数组表头不算段。
#[test]
fn section_names_lists_headers_once_in_file_order() {
    let lines = split_lines("[a]\nk = v\n\n# 注释\n[b]\nk = v\n[a]\nk = v\n[[job]]\nid = \"x\"\n");
    assert_eq!(section_names(&lines), ["a", "b"], "{:?}", section_names(&lines));
    // 尾随空格与引号里的方括号都不该把它读成两段：`is_header` 只认整行首尾。
    let odd = split_lines("  [c]  \nnot a header = [x]\n");
    assert_eq!(section_names(&odd), ["c"], "{:?}", section_names(&odd));
}

/// 日志目录那一格：读侧**不创建**它，也不试写（那是副作用，而且会在 `edit.rs` 之外
/// 造出第二处写文件的地方）。它只分得清"还没有"与"在那儿"，写不进去那句由启动路径说。
#[test]
fn the_log_dir_cell_does_not_create_the_directory_and_says_it_is_not_there_yet() {
    let dir = tempfile::tempdir().unwrap();
    let here = dir.path().join("logs");
    let jobs = JOBS_COMMENTS.replace(r"'D:\logs'", &format!("'{}'", here.display()));
    std::fs::write(dir.path().join("jobs.toml"), &jobs).unwrap();
    let files = Files {
        jobs: dir.path().join("jobs.toml"),
        conf: None,
        rclone_bin: PathBuf::from(r"C:\rclone\rclone.exe"),
    };
    let view = current(&files, vec![]).unwrap();
    assert!(!here.exists(), "读侧把目录创建出来了：那是一件副作用");
    assert!(!view.defaults.log_dir_exists, "还没有的目录被说成存在");
    assert!(view.defaults.log_dir_note.contains("还没有"), "{:?}", view.defaults.log_dir_note);
    assert!(view.defaults.log_dir_note.contains("启动"),
        "要指出是谁会创建它，操作员才知道下一步不是自己建：{:?}", view.defaults.log_dir_note);
    // 远端那一栏连文件在哪都不知道 ⇒ 那一句要说「这一栏列不出来」，不能留白。
    assert!(view.conf_note.contains("这一栏列不出来"), "{:?}", view.conf_note);

    std::fs::create_dir_all(&here).unwrap();
    let view = current(&files, vec![]).unwrap();
    assert!(view.defaults.log_dir_exists, "{:?}", view.defaults);
    assert!(view.defaults.log_dir_note.is_empty(), "目录在那儿却还在解释：{:?}", view.defaults);
}

/// 两个只读探针的解析层：钉在**实测的 stdout 形状**上。
#[test]
fn the_probe_parsers_are_pinned_to_the_measured_stdout() {
    // v1.75.1 实测：`config file` 交两行、`version` 第一行是 `rclone v1.75.1`。
    // 换一种"更宽松"的解法（找第一个像路径的行、在整段里 grep v[0-9]）在别的版本上会
    // 悄悄解出错的那一行，而那一行会被当成"你的配置在这里"——所以认不出就交 None。
    let conf_out = "Configuration file is stored at:\nC:\\Users\\me\\AppData\\Roaming\\rclone\\rclone.conf\n";
    assert_eq!(parse_config_file_stdout(conf_out),
               Some(PathBuf::from(r"C:\Users\me\AppData\Roaming\rclone\rclone.conf")));
    assert_eq!(parse_version_stdout("rclone v1.75.1\n- os/version: Microsoft Windows 11 Pro\n"),
               Some("1.75.1".to_string()));
    assert_eq!(parse_config_file_stdout("rclone configuration file:\n\n"), None);
    assert_eq!(parse_config_file_stdout("Configuration file is stored at:\n"), None);
    assert_eq!(parse_version_stdout(""), None);
    assert_eq!(parse_version_stdout("rclone v\n"), None);
}

// ---- Task 3：plan_bytes —— 造新字节、装载侧校验、只改了这些行 ----

fn files_in(dir: &Path, jobs: &str, conf: &str) -> Files {
    let j = dir.join("jobs.toml");
    let c = dir.join("rclone.conf");
    std::fs::write(&j, jobs).unwrap();
    std::fs::write(&c, conf).unwrap();
    // rclone_bin 这一份**不存在**：plan 这条通路不起进程，而"改路径要指着真存在的文件"
    // 那一档只在真的动了这一项时才检查（见 changing_the_binary_…那条）。
    Files { jobs: j, conf: Some(c), rclone_bin: PathBuf::from(r"C:\rclone\rclone.exe") }
}

fn defaults_intent(bin: &str, flags: &[&str], exclude: &[&str]) -> Intent {
    Intent::Defaults {
        rclone_bin: bin.into(),
        log_dir: r"D:\logs".into(),
        flags: flags.iter().map(|s| s.to_string()).collect(),
        exclude: exclude.iter().map(|s| s.to_string()).collect(),
    }
}

fn job_add(id: &str, name: &str, kind: JobKindWire, src: &str, dst: &str) -> Intent {
    Intent::JobAdd {
        id: id.into(), name: name.into(), job_kind: kind,
        src: src.into(), dst: dst.into(), open_dir: String::new(), manifest: false,
    }
}

#[test]
fn an_intent_the_loader_would_reject_writes_nothing() {
    // 三发"装载侧现在就拒"的形状：id 撞车、对齐两侧同端、flag 不在白名单。
    // 每一发都断两次：那句拒绝不是空的，**而且文件一个字节都没动**。
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("id 撞车", job_add("a", "新的一条", JobKindWire::Copy, r"D:\three", "r2:three")),
        // 名字里带那个字，这样它过的就是**装载侧**那句（不是面板自己那条名字判据）。
        ("两侧同端", job_add("c", "镜像：删多余的", JobKindWire::Sync, r"D:\three", r"D:\four")),
        ("flag 越白名单", defaults_intent(r"C:\rclone\rclone.exe", &["--exclude"], &[])),
    ];
    for (label, intent) in cases {
        let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
        let before = std::fs::read(&files.jobs).unwrap();
        let err = plan_bytes(&intent, &files).unwrap_err().to_string();
        assert!(!err.is_empty(), "{label} 那句拒绝是空的");
        assert_eq!(std::fs::read(&files.jobs).unwrap(), before, "{label} 之后文件被动过了");
    }
    // 撞车那一发要点名是哪条 id——操作员得知道撞的是谁。
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let err = plan_bytes(&job_add("a", "n", JobKindWire::Copy, r"D:\t", "r2:t"), &files)
        .unwrap_err().to_string();
    assert!(err.contains("a"), "要点名撞上的那条 id：{err}");
    assert!(!err.contains("rcd") && !err.contains("token"), "拒绝话术漏了内部名词：{err}");
    // 两侧同端那一发拿的是**装载侧那一句**（不是面板另编的）：它说得出"恰好一侧是 remote"。
    let err = plan_bytes(&job_add("c", "镜像：删多余的", JobKindWire::Sync, r"D:\t", r"D:\u"), &files)
        .unwrap_err().to_string();
    assert!(err.contains("恰好一侧"), "没把装载侧那句转达上来：{err}");
}

/// 反向那一半：**改**一条不存在的任务也不能悄悄追加一条新的（那会让"我要改甲"变成"多了一条"）。
#[test]
fn editing_a_job_that_is_not_there_is_refused_instead_of_appending_one() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let before = std::fs::read(&files.jobs).unwrap();
    let err = plan_bytes(&Intent::JobSet {
        id: "nope".into(), name: "改名".into(), job_kind: JobKindWire::Copy,
        src: r"D:\one".into(), dst: "r2:one".into(), open_dir: String::new(), manifest: false,
    }, &files).unwrap_err().to_string();
    assert!(err.contains("nope"), "要点名找不到哪一条：{err}");
    assert_eq!(std::fs::read(&files.jobs).unwrap(), before, "被拒的一次改了文件");
    // 而且新加那一条撞车时也不算"更新"（两条通路各拦各的，没有互相顶替）
    assert!(plan_bytes(&job_add("b", "n", JobKindWire::Copy, r"D:\t", "r2:t"), &files).is_err());
}

#[test]
fn a_sync_name_without_the_delete_word_is_refused_with_a_reason() {
    // 裁定 3：面板写进来的会删的行，名字里必须有「删」。出厂配置那条判据在 app_state.rs，
    // 只管出厂那几条；这一条管操作员与面板新建的每一条，而且句子只有 `delete_word_in_name_reason` 一份。
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let err = plan_bytes(&job_add("z", "镜像 R2", JobKindWire::Sync, r"D:\z", "r2:z"), &files)
        .unwrap_err().to_string();
    assert!(err.contains("删"), "要说清名字里缺哪个字：{err}");
    assert_eq!(err, delete_word_in_name_reason(), "拒绝的句子不是那份唯一出处：{err}");
    assert!(!err.contains("rcd") && !err.contains("token"), "话术里有内部名词：{err}");
    // 放行半边：名字里带「删」的 sync 过得去。
    assert!(plan_bytes(&job_add("z", "镜像 R2：多余的会被删掉", JobKindWire::Sync,
        r"D:\z", "r2:z"), &files).is_ok());
}

#[test]
fn a_new_job_without_an_id_is_refused_and_writes_nothing() {
    // 装载侧那句查重只认"两条非空的撞了"，空编号它不报 —— 所以这一发必须由面板写侧自己拦。
    // 拦不住的话落出去的是 `id = ""`：界面上多一条谁也指不回的任务，而它看起来跟正常的一样。
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let before = std::fs::read(&files.jobs).unwrap();
    let err = plan_bytes(&job_add("", "没有编号的一条", JobKindWire::Copy, r"D:\t", "r2:t"), &files)
        .unwrap_err().to_string();
    assert!(err.contains("编号"), "那句拒绝要说清缺的是编号：{err}");
    assert!(!err.contains("rcd") && !err.contains("token") && !err.contains("id 为空"),
        "话术里有内部名词或工程口径：{err}");
    assert_eq!(std::fs::read(&files.jobs).unwrap(), before, "被拒的一次改了文件");
    // 放行半边：填了编号的新条目过得去。
    assert!(plan_bytes(&job_add("brand-new", "有编号的一条", JobKindWire::Copy,
        r"D:\t", "r2:t"), &files).is_ok());
    // 而"改这条"那一档的空编号走的还是上面那一句（找不到 id 为 的那条）——两条通路各拦各的，
    // 不许把空编号的更新悄悄当成新加。
    let err = plan_bytes(&Intent::JobSet {
        id: String::new(), name: "改名".into(), job_kind: JobKindWire::Copy,
        src: r"D:\one".into(), dst: "r2:one".into(), open_dir: String::new(), manifest: false,
    }, &files).unwrap_err().to_string();
    assert!(err.contains("找不到"), "空编号的更新该报找不到，而不是新加：{err}");
}

#[test]
fn an_empty_bin_and_a_wrong_bin_get_two_different_sentences() {
    // 两档不能共用一句：操作员清空那一格时，"那个位置没有可执行文件：。"说的是一个他
    // 根本没指过的路径 —— 那种句子把他往"去查那个路径"支走，而真正缺的是"你还没填"。
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let before = std::fs::read(&files.jobs).unwrap();
    let empty = plan_bytes(&defaults_intent("", &["--fast-list"], &[]), &files).unwrap_err().to_string();
    assert!(empty.contains("那一格是空的"), "空的那一格要说清是空的：{empty}");
    assert!(!empty.contains("那个位置没有可执行文件"), "空路径被套上了'指错位置'那句：{empty}");
    let wrong = plan_bytes(
        &defaults_intent(r"C:\nope\nope.exe", &["--fast-list"], &[]), &files,
    ).unwrap_err().to_string();
    assert!(wrong.contains("没有可执行文件"), "指错的那一档该说位置不对：{wrong}");
    assert!(wrong.contains("nope.exe"), "要点名是哪个位置：{wrong}");
    assert_eq!(std::fs::read(&files.jobs).unwrap(), before, "被拒的两次里有一次动了文件");
    // 半边放行：**没动这一项**时不拿现有的路径去判存在性（操作员没碰它就不该被拦）。
    assert!(plan_bytes(&defaults_intent(r"C:\rclone\rclone.exe", &["--fast-list"], &[]), &files).is_ok());
}

#[test]
fn plan_rows_are_exactly_the_lines_that_differ() {
    let dir = tempfile::tempdir().unwrap();
    // 换成一个**真的存在**的路径（这一档的判据是"你指的那个文件得在那儿"）。
    let other = dir.path().join("other-rclone.exe");
    std::fs::write(&other, b"not a real rclone").unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let p = plan_bytes(&defaults_intent(&other.display().to_string(), &["--fast-list"], &["*.log"]),
        &files).unwrap();
    // 四样里只有 rclone_bin 变了 ⇒ 差异行**只有一条**。多一条就是"顺手改了别的"。
    let keys: Vec<&str> = p.changes.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, ["rclone_bin"], "{keys:?}");
    assert!(p.changes[0].before.contains(r"C:\rclone\rclone.exe"), "{:?}", p.changes[0]);
    assert!(p.changes[0].after.contains("other-rclone.exe"), "{:?}", p.changes[0]);
    assert_eq!(p.path, files.jobs);
    // 只改这一项时，flags 与 exclude 那两句连带**不许**跟着出现：说了没做的事也是说谎。
    assert!(!p.notes.iter().any(|n| n.contains("启动") || n.contains("看到的不是全部")),
        "{:?}", p.notes);
    // 没动的字节仍然逐字相同（夹具里那条理由注释还在）。
    assert!(String::from_utf8_lossy(&p.bytes).contains("这几行注释说的是它为什么危险"));
    // 落地的字节里那一行确实换了，而且用的是 TOML 字面串（反斜杠不转义）。
    let landed = String::from_utf8_lossy(&p.bytes).into_owned();
    assert!(landed.contains(&format!("rclone_bin = '{}'", other.display())), "{landed}");
}

/// 存在性判据只在**这一项被改动**时检查：不改它就不拿一个操作员没碰过的路径去拦他。
#[test]
fn changing_the_binary_to_a_missing_file_is_refused_naming_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let missing = dir.path().join("does-not-exist.exe");
    let err = plan_bytes(&defaults_intent(&missing.display().to_string(),
        &["--fast-list"], &["*.log"]), &files).unwrap_err().to_string();
    assert!(err.contains("does-not-exist.exe"), "要点名是哪个路径：{err}");
    assert!(err.contains("没有"), "{err}");
    // 相对路径同样拒（装载侧那句会说"必须是绝对路径"）：这里比它先拦的是"存不存在"。
    assert!(plan_bytes(&defaults_intent(r"rclone", &["--fast-list"], &["*.log"]), &files).is_err());
    // 反向半边：这一项一个字没动时，即便它指向一个不存在的路径也**不该**由这一档拒
    // （面板现在就是这么跑起来的；要拦的是"改成另一个"那一件事）。
    assert!(plan_bytes(&defaults_intent(r"C:\rclone\rclone.exe", &["--stats"], &["*.log"]),
        &files).is_ok());
}

#[test]
fn the_side_effect_sentences_appear_only_with_the_fields_that_cause_them() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let p = plan_bytes(&defaults_intent(r"C:\rclone\rclone.exe", &["--transfers", "32"],
        &["secret/*"]), &files).unwrap();
    assert!(p.notes.iter().any(|n| n.contains("重开") && n.contains("启动")),
        "flags 变了却没说「要重开」那一句：{:?}", p.notes);
    assert!(p.notes.iter().any(|n| n.contains("看到的不是全部")),
        "exclude 从空变成非空，那句连带没进来：{:?}", p.notes);
    let keys: Vec<&str> = p.changes.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, ["flags", "exclude"], "差异行必须按字段顺序出：{keys:?}");
    assert!(p.changes[0].after.contains("--transfers"), "{:?}", p.changes[0]);
}

/// exclude 从非空回到空时，那句连带也要说（操作员得知道"清空这一层"那颗按钮从此点得动了）。
#[test]
fn clearing_exclude_says_the_button_that_becomes_live() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let p = plan_bytes(&defaults_intent(r"C:\rclone\rclone.exe", &["--fast-list"], &[]), &files).unwrap();
    assert!(p.notes.iter().any(|n| n.contains("清空")), "exclude 被清空却没说那一档：{:?}", p.notes);
}

#[test]
fn removing_a_gate_job_says_the_prefix_loses_its_gate() {
    // spec §5.3 尾段：删一条对账类任务会拿走一道发布保护。判据来自那块自己的 kind 行。
    // 反向半边也钉着：删一条复制任务时那句闸门**不许**出现。
    const GATE_JOBS: &str = r#"
[defaults]
rclone_bin = 'C:\rclone\rclone.exe'
flags = []
exclude = []
log_dir = 'D:\logs'

[[job]]
id = "g"
name = "封面对账"
kind = "gate"
src = 'D:\one'
dst = "r2:img/one"
manifest = { kind = "localdir" }
"#;
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), GATE_JOBS, CONF_CRLF);
    let p = plan_bytes(&Intent::RemoveJob { id: "g".into() }, &files).unwrap();
    assert!(p.notes.iter().any(|n| n.contains("闸门")), "{:?}", p.notes);
    // 正文是那一**对**位置，不是任务名：名字可以撞，配对才是身份。
    assert!(p.subject.contains('→'), "正文里没有那串位置：{}", p.subject);
    assert!(!p.subject.contains("封面对账"), "正文写成了任务名：{}", p.subject);
    // 删干净之后新字节仍能解析（一块删掉不许留下半截键行）。
    assert!(!String::from_utf8_lossy(&p.bytes).contains("id = \"g\""));

    let copy = GATE_JOBS.replace(r#"kind = "gate""#, r#"kind = "copy""#)
        .replace(r#"manifest = { kind = "localdir" }"#, "");
    let files = files_in(dir.path(), &copy, CONF_CRLF);
    let p = plan_bytes(&Intent::RemoveJob { id: "g".into() }, &files).unwrap();
    assert!(!p.notes.iter().any(|n| n.contains("闸门")), "复制那一类也念了闸门那句：{:?}", p.notes);
}

/// 那句「这一条会删东西」指向的是卡上的一颗按钮：按钮改名而这句话没跟着改，人就会去找一颗
/// 不存在的按钮（那次"只看到试跑、没看到真跑"正是这个形状 —— 句子指在"看"那一颗上）。
#[test]
fn the_sync_note_names_the_entry_that_actually_exists() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let p = plan_bytes(&Intent::JobSet {
        id: "b".into(), name: "镜像：删多余的".into(), job_kind: JobKindWire::Sync,
        src: r"D:\two".into(), dst: "r2:two".into(), open_dir: String::new(), manifest: false,
    }, &files).unwrap();
    let hits = p.notes.iter().filter(|n| n.contains("会删东西")).count();
    assert_eq!(hits, 1, "那句该正好一条：{:?}", p.notes);
    assert!(p.notes.iter().any(|n| n.contains("会删东西") && n.contains("让两边对齐")),
        "那句没指到现在卡上那颗按钮：{:?}", p.notes);
    assert!(!p.notes.iter().any(|n| n.contains("会删东西") && n.contains("看看要删什么")),
        "那句还指着旧名字：{:?}", p.notes);

    // 复制那一档没有"先看清要删什么"这一层，这句不该跟着出现。
    let p = plan_bytes(&Intent::JobSet {
        id: "a".into(), name: "甲".into(), job_kind: JobKindWire::Copy,
        src: r"D:\one".into(), dst: "r2:one".into(), open_dir: String::new(), manifest: false,
    }, &files).unwrap();
    assert!(!p.notes.iter().any(|n| n.contains("会删东西")), "复制那一类也念了删除那句：{:?}", p.notes);
}

#[test]
fn an_empty_value_removes_the_line_and_says_so() {
    // 裁定 6：空串 = 删那一行（`region` 在 conf 里、`open_dir` 在 jobs.toml 里都属这一档）。
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let p = plan_bytes(&Intent::Remote {
        remote: "r2".into(), remote_type: "s3".into(), provider: "Cloudflare".into(),
        endpoint: "https://xxx.r2.dev".into(), region: String::new(),
    }, &files).unwrap();
    let gone = p.changes.iter().find(|c| c.key == "region").expect("region 那一行要出现在差异里");
    assert_eq!(gone.kind, ChangeKind::Removed);
    let landed = String::from_utf8_lossy(&p.bytes).into_owned();
    assert!(!landed.contains("region ="), "那一行还在：{landed}");
    // 没点名的行连密钥带注释一起原样。
    assert!(landed.contains("SENTINEL-SK2-22bb"), "密钥那行被吃了");
    assert!(landed.contains("upload_concurrency = 4"), "白名单之外的键被顺手删了");
    assert_eq!(p.path, files.conf.unwrap());
    assert_eq!(p.subject, "r2", "远端那一块的正文是远端名：{}", p.subject);
}

/// 新加一条任务 = 整块追加在文件末尾，前面补一个空行；不动任何已有行。
#[test]
fn a_new_job_is_appended_as_one_block_and_touches_no_existing_line() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let before = join_lines(&split_lines(JOBS_COMMENTS));
    let p = plan_bytes(&job_add("newone", "新的一条", JobKindWire::Copy, r"D:\n", "r2:n"), &files).unwrap();
    let landed = String::from_utf8_lossy(&p.bytes).into_owned();
    assert!(landed.starts_with(&before[..before.len() - 1]), "文件前面的内容被动过：{landed}");
    assert!(landed.contains("[[job]]\nid = \"newone\""), "新块没落在末尾：{landed}");
    assert!(landed.contains(r#"src = 'D:\n'"#), "路径写成了字面串以外的样子：{landed}");
    // 每一条键行都是一行差异，而**没有任何一条**是 changed/removed。
    assert!(p.changes.iter().all(|c| c.kind == ChangeKind::Added), "{:?}", p.changes);
    let keys: Vec<&str> = p.changes.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, ["id", "name", "kind", "src", "dst"], "{keys:?}");
}

/// `open_dir` 空与非空的三档：原来没有 → 加一行；原来有、清空 → 删那一行；两边都有且相同 → 不出现在差异里。
#[test]
fn the_optional_open_dir_line_is_added_removed_and_left_alone_as_appropriate() {
    let dir = tempfile::tempdir().unwrap();
    let set = |open: &str| Intent::JobSet {
        id: "a".into(), name: "甲".into(), job_kind: JobKindWire::Copy,
        src: r"D:\one".into(), dst: "r2:one".into(), open_dir: open.into(), manifest: false,
    };
    // 夹具那一块没有 open_dir：给一个值 ⇒ Added。
    let p = plan_bytes(&set(r"D:\one\sub"), &files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF)).unwrap();
    assert_eq!(p.changes.len(), 1, "{:?}", p.changes);
    assert_eq!(p.changes[0].kind, ChangeKind::Added);
    assert!(String::from_utf8_lossy(&p.bytes).contains(r#"open_dir = 'D:\one\sub'"#));
    // 空串 ⇒ 那一行本来就没有 ⇒ 零差异。
    let p = plan_bytes(&set(""), &files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF)).unwrap();
    assert!(p.changes.is_empty(), "{:?}", p.changes);
    // 有 open_dir 的那一块清空 ⇒ Removed，且只动那一行。
    let with_open = JOBS_COMMENTS.replace(r#"dst = "r2:one""#, "dst = \"r2:one\"\nopen_dir = 'D:\\one\\sub'");
    let p = plan_bytes(&set(""), &files_in(dir.path(), &with_open, CONF_CRLF)).unwrap();
    assert_eq!(p.changes.len(), 1, "{:?}", p.changes);
    assert_eq!(p.changes[0].kind, ChangeKind::Removed);
    assert!(!String::from_utf8_lossy(&p.bytes).contains("open_dir"));
}

/// 面板生成的名字必须过面板自己那条判据（它是"手打名字"那一档的正面半边）。
#[test]
fn the_generated_name_passes_the_rule_it_would_refuse_a_human_for() {
    let n = suggested_job_name(r"D:\vault\img-r2", "r2:img");
    // 2026-10-09：判据里那个「镜像」跟着词表一起搬到了「让两边对齐」（`镜像` → `对齐` 的
    // 全项目迁移）。**真正要守住的是「删」那个字** —— `delete_word_in_name_reason` 只要求它，
    // 「对齐」那一半只是这一族在别处已经统一的说法。所以这里两半都钉：删是判据，对齐是口径。
    assert!(n.contains("让两边对齐") && n.contains("删"), "{n}");
    // 两侧都是本地时也给得出一个不含糊的名字：那种意图被装载侧那句拒，理由不是"名字为空"。
    let n = suggested_job_name(r"D:\a", r"D:\b");
    assert!(n.contains("删") && !n.is_empty(), "{n}");
    // 而且落在界面上时它过 `delete_word_in_name_reason` 那条判据（同一条句子，一处出处）。
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let auto = job_add("zz", "", JobKindWire::Sync, r"D:\z", "r2:z");
    assert!(plan_bytes(&auto, &files).is_ok(), "空名字该由面板生成，而不是被拒");
    let p = plan_bytes(&auto, &files).unwrap();
    let row = p.changes.iter().find(|c| c.key == "name").expect("生成的名字要作为一行差异看得见");
    assert!(row.after.contains("删"), "{:?}", row.after);
}

/// 校验与断言的**顺序**是 spec §4 明写的两条不能换。判据按源文本的位置钉。
/// 这两步住在 `finish` 里（两条通路共用一份），所以扫的是那一段，外加"两条通路都走它"。
#[test]
fn validation_runs_before_the_line_assertion() {
    let src = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/config/edit.rs")).unwrap();
    let tight: String = src.lines().filter(|l| !l.trim_start().starts_with("//")).collect();
    let body = tight.split("fn finish").nth(1).expect("finish 不在这个文件里");
    let body = body.split("fn plan_jobs").next().unwrap_or(body);
    let v = body.find("JobsFile::parse").expect("校验不在 finish 里");
    let a = body.find("only_these_changed").expect("断言不在 finish 里");
    assert!(v < a, "断言排在校验前面了：spec §4 明写这两步不能换");
    // 顺序换了不算，两条通路绕开这一步也不算：jobs 那一支必须经过校验。
    for f in ["fn plan_jobs", "fn plan_conf"] {
        let arm = tight.split(f).nth(1).unwrap_or("");
        assert!(arm.contains("finish("), "{f} 没走那份共用的校验与断言");
    }
    assert!(tight.split("fn finish").nth(1).unwrap()
        .split("fn plan_jobs").next().unwrap().contains("if jobs"),
        "finish 不再区分两份文件：jobs.toml 那一支的校验可能被 conf 那一支共用掉");
}

/// 差异行为空 ⇒ `plan_bytes` 仍然交得出方案（是否发凭证由 `WriteRegistry` 判，Task 4）。
/// 这一条钉的是"空差异不是一种错误"，界面上那句「这一项现在就是你要的值」由 Task 4 给。
#[test]
fn an_intent_that_changes_nothing_produces_no_rows_rather_than_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let p = plan_bytes(&defaults_intent(r"C:\rclone\rclone.exe", &["--fast-list"], &["*.log"]),
        &files).unwrap();
    assert!(p.changes.is_empty(), "{:?}", p.changes);
    assert_eq!(p.bytes, p.original, "零差异也该是同一份字节");
}

// ---- Task 4：凭证与落盘 ----

/// 一个**存在**的 rclone 路径：`Intent::Defaults` 改了 `rclone_bin` 时装载前判据要它在那儿。
fn real_bin(dir: &Path) -> String {
    let p = dir.join("rclone-copy.exe");
    std::fs::write(&p, b"not a real rclone").unwrap();
    p.display().to_string()
}

fn defaults_to(dir: &Path, flags: &[&str], exclude: &[&str]) -> Intent {
    Intent::Defaults { rclone_bin: real_bin(dir), log_dir: r"D:\logs".into(),
        flags: flags.iter().map(|s| s.to_string()).collect(),
        exclude: exclude.iter().map(|s| s.to_string()).collect() }
}

fn tmp_leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|n| n.contains("rclone-deck-tmp")).collect()
}

#[tokio::test]
async fn a_token_is_consumed_once_and_a_second_apply_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let reg = WriteRegistry::default();
    let intent = defaults_to(dir.path(), &["--fast-list"], &["*.log"]);
    let view = reg.plan(&intent, &files, &[]).await.unwrap();
    // 只有 rclone_bin 那一项变了 ⇒ 差异表只有一行（flags 与 exclude 照原样交回去）。
    // 只有 rclone_bin 那一项变了 ⇒ 差异表只有一行（flags 与 exclude 照原样交回去就不该出现）。
    assert_eq!(view.changes.len(), 1, "{:?}", view.changes);
    let keys: Vec<&str> = view.changes.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys, ["rclone_bin"], "{:?}", view.changes);
    assert!(view.path.ends_with("jobs.toml"), "确认层正文第一行得是完整路径：{}", view.path);
    assert!(view.token.starts_with("prev-"), "凭证的形状漂了：{}", view.token);

    reg.apply(&view.token, &files, &[]).await.unwrap();
    let after_first = std::fs::read(&files.jobs).unwrap();
    assert!(String::from_utf8_lossy(&after_first).contains("rclone-copy.exe"), "第一次没写进去");
    assert!(tmp_leftovers(dir.path()).is_empty(), "写完留着临时文件：{:?}", tmp_leftovers(dir.path()));

    // 同一个凭证再按一次：拿不到东西，也**一个字节都不再写**（后台先吃掉再判）。
    let err = reg.apply(&view.token, &files, &[]).await.unwrap_err().to_string();
    assert!(err.contains("不作数"), "{err}");
    assert_eq!(std::fs::read(&files.jobs).unwrap(), after_first, "花掉的凭证又写了一次");
    // 换一个假凭证同样写不了。
    let err2 = reg.apply("prev-not-issued-here", &files, &[]).await.unwrap_err().to_string();
    assert!(err2.contains("重新看一遍"), "拒绝要给出路：{err2}");
}

#[tokio::test]
async fn a_file_changed_by_someone_else_after_the_plan_refuses_the_write() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let reg = WriteRegistry::default();
    let intent = defaults_to(dir.path(), &["--stats"], &["*.log"]);
    let view = reg.plan(&intent, &files, &[]).await.unwrap();
    // 第三方：有人这时在编辑器里存了一份回来（spec §10 第二条那件事）。
    std::fs::write(&files.jobs, JOBS_COMMENTS.replace("--fast-list", "--transfers")).unwrap();
    let err = reg.apply(&view.token, &files, &[]).await.unwrap_err().to_string();
    assert!(err.contains("刚才看的那份"), "{err}");
    assert!(err.contains("一个字节都没写"), "要说清这一次动没动磁盘：{err}");
    // 磁盘上留的是**第三方那一版**，我们的字节没插进去。
    let now = String::from_utf8_lossy(&std::fs::read(&files.jobs).unwrap()).into_owned();
    assert!(now.contains("--transfers") && !now.contains("--stats"), "{now}");
    assert!(tmp_leftovers(dir.path()).is_empty(), "被拒的一次留下了临时文件：{:?}", tmp_leftovers(dir.path()));
}

#[tokio::test]
async fn a_running_job_refuses_the_plan_and_the_apply_before_anything_is_issued() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let before = std::fs::read(&files.jobs).unwrap();
    let reg = WriteRegistry::default();
    let intent = defaults_to(dir.path(), &["--fast-list"], &["*.tmp"]);
    let err = reg.plan(&intent, &files, &["图床批量上传".into()]).await.unwrap_err().to_string();
    assert!(err.contains("图床批量上传"), "要说得出是哪一条在跑：{err}");
    assert!(err.contains("重开"), "要说清为什么现在不许改：{err}");

    // 没在跑时领到的凭证，等到真跑起来了再按 → 仍然拒（动手之前那一道也要拦）。
    let view = reg.plan(&intent, &files, &[]).await.unwrap();
    let err = reg.apply(&view.token, &files, &["图床批量上传".into()]).await.unwrap_err().to_string();
    assert!(err.contains("图床批量上传"), "动手那一道没拦：{err}");
    assert_eq!(std::fs::read(&files.jobs).unwrap(), before, "被拒的一次改了磁盘");
}

#[tokio::test]
async fn an_intent_that_changes_nothing_issues_no_token() {
    // 「差异为空不给保存按钮」的后台半边：连凭证都发不出来，界面上就没有可按的那一颗。
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let reg = WriteRegistry::default();
    let err = reg.plan(&defaults_intent(r"C:\rclone\rclone.exe", &["--fast-list"], &["*.log"]),
        &files, &[]).await.unwrap_err().to_string();
    assert!(err.contains("没有要改的东西"), "{err}");
    assert_eq!(std::fs::read(&files.jobs).unwrap(), JOBS_COMMENTS.as_bytes(), "零差异也写了盘");
}

/// 保存成功那一句的证据：短槽是「重开面板后生效。」，detail 里要点名写在哪个文件。
#[tokio::test]
async fn a_saved_write_says_it_takes_effect_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let files = files_in(dir.path(), JOBS_COMMENTS, CONF_CRLF);
    let reg = WriteRegistry::default();
    let intent = defaults_to(dir.path(), &["--fast-list"], &["*.tmp"]);
    let view = reg.plan(&intent, &files, &[]).await.unwrap();
    let applied = reg.apply(&view.token, &files, &[]).await.unwrap();
    assert!(applied.note.short.contains("重开面板后生效"), "{:?}", applied.note.short);
    assert!(applied.note.detail.contains("jobs.toml"), "要说出写在哪：{:?}", applied.note.detail);
    let said = format!("{}{}", applied.note.short, applied.note.detail);
    for banned in ["rcd", "token", "write_atomically"] {
        assert!(!said.contains(banned), "保存成功那句里有 {banned}：{said}");
    }
    assert!(applied.path.ends_with("jobs.toml"), "{}", applied.path);
}

/// 证人 1（实施形状见计划里那条裁定）：**全仓写文件的位置是一份带理由的封闭名单**，
/// 名单之外出现写入 = 红；名单里每一条都必须真的在树上找到那一句（否则名单在骗人）。
#[test]
fn every_place_that_writes_to_disk_is_on_the_named_list() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let writers: &[(&str, &str)] = &[
        ("config/edit.rs", "用户那两份配置文件的唯一写通路"),
        ("state.rs", "首次运行时落出厂 jobs.toml：只写自己刚创建的文件，已存在就不动"),
        ("rcd/mod.rs", "rcd 的 stderr 落点与轮转后往新日志写那句说明"),
        ("rcd/pidfile.rs", "rcd 的 pid 记录（收尸的依据）"),
        ("runner/tasklog.rs", "每一次动作的任务日志（本项目承诺的那份可回溯证据）"),
        ("runner/lists.rs", "盘点顺手落的那份可 diff 名单：只写自己刚创建的文件，落在日志旁边的 lists/ 里"),
    ];
    let found = walk_rs(&root);
    assert!(found.len() > 12, "扫描没走完这棵树：只找到 {} 个 .rs", found.len());
    let mut outside: Vec<String> = Vec::new();
    let mut matched: Vec<String> = Vec::new();
    for entry in &found {
        let rel = entry.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        let text = std::fs::read_to_string(entry).unwrap();
        if !WRITER_NEEDLES.iter().any(|n| text.contains(n)) { continue; }
        match writers.iter().find(|(f, _)| *f == rel) {
            Some((_, reason)) => {
                assert!(!reason.is_empty(), "{rel} 在名单上却没有理由");
                matched.push(rel);
            }
            None => outside.push(rel),
        }
    }
    assert!(outside.is_empty(), "出现了名单之外的写入点：{outside:?}");
    // 名单的反向半边：列着却已经不写的，同样是在骗人。
    let stale: Vec<&str> = writers.iter().map(|(f, _)| *f)
        .filter(|f| !matched.iter().any(|m| m == f)).collect();
    assert!(stale.is_empty(), "名单上这几条已经不写文件了，删掉它们：{stale:?}");
    // 而配置文件那一条必须**确实**在写：不然整条证人是在证明"什么都没做"。
    let edit = std::fs::read_to_string(root.join("config/edit.rs")).unwrap();
    assert!(edit.contains("fs::write("), "edit.rs 里已经没有写盘通路了，这条证人随之失效");
}

const WRITER_NEEDLES: &[&str] = &["fs::write(", "File::create(", "write_all("];

/// 递归列出 `.rs`。
fn walk_rs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() { out.extend(walk_rs(&p)); }
        else if p.extension().map(|e| e == "rs").unwrap_or(false) { out.push(p); }
    }
    out
}

/// 第二层：`edit.rs` **内部**也只许一条写通路，而且它只被 `apply` 调。
/// 这一条抓的是"在 edit.rs 里再开一个函数直接写配置文件"那种形状——文件级白名单对它无效。
#[test]
fn the_write_path_inside_edit_is_one_function_called_only_from_apply() {
    let src = std::fs::read_to_string(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/config/edit.rs")).unwrap();
    let bodies = function_bodies(&src);
    assert!(bodies.len() > 12, "函数切分没切开这个文件：只切出 {} 段", bodies.len());
    let named = |needle: &str| -> Vec<String> {
        bodies.iter().filter(|(_, b)| b.contains(needle)).map(|(n, _)| n.clone()).collect()
    };
    assert_eq!(named("fs::write("), ["write_atomically"], "写盘通路不止一条");
    assert_eq!(named("fs::rename("), ["write_atomically"], "rename 出现在别处");
    // 调用点：只有 `apply` 那一条。函数自己那一行当然写着它自己的名字，先排掉。
    let callers: Vec<String> = bodies.iter()
        .filter(|(n, b)| n != "write_atomically" && b.contains("write_atomically("))
        .map(|(n, _)| n.clone()).collect();
    assert_eq!(callers, ["apply"], "写入函数被别处调了");
    // 名单外的形状：临时文件名必须落在**同一目录**（跨盘的 rename 不是原子的）。
    let body = bodies.iter().find(|(n, _)| n == "write_atomically").map(|(_, b)| b.clone())
        .expect("write_atomically 不在了");
    assert!(body.contains("with_file_name"), "临时文件名不是从目标文件派生的：{body}");
    assert!(!body.contains("env::temp_dir") && !body.contains("tempfile"),
        "临时文件落在了别处的目录里：跨卷 rename 不是原子替换");
}

/// 把源文本按函数切成 `(名字, 含体)`。**数花括号**配平，缩进与注释都不算：
/// 注释里提一句 `fs::write(` 不该让证人红（红错理由的证人教人改注释，不教人改代码）。
fn function_bodies(src: &str) -> Vec<(String, String)> {
    let heads = ["pub async fn ", "pub fn ", "async fn ", "fn "];
    let mut out: Vec<(String, String)> = Vec::new();
    let mut name: Option<String> = None;
    let mut body = String::new();
    let mut depth = 0i32;
    let mut opened = false;
    for line in src.lines() {
        if name.is_none() {
            let t = line.trim_start();
            let Some(p) = heads.into_iter().find(|p| t.starts_with(p)) else { continue };
            let n = t[p.len()..].split('(').next().unwrap_or("").trim().to_string();
            if n.is_empty() { continue; }
            name = Some(n);
            body.clear();
            depth = 0;
            opened = false;
        }
        body.push_str(line);
        body.push('\n');
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        // `{` 可能落在签名的第二行（`pub async fn apply(…,\n  …) -> Result<…> {`）：
        // 那种写法下前面那几行深度一直是 0，必须等到真开了括号才算"进了体"。
        if line.contains('{') { opened = true; }
        if opened && depth <= 0 {
            out.push((name.take().unwrap(), std::mem::take(&mut body)));
        }
    }
    out
}

/// 「打开 rclone 的配置目录」那颗按钮开的是**哪一层**。
///
/// 钉的是四条会真出事的形状：Windows 与类 Unix 各取所在目录；问不出时不能开任何东西（空串
/// 交给 explorer 会打开"文档"，那不是这一颗说的事）；只有文件名没有目录时也不能开；
/// 那一份正好挂在根上时也不能开（父目录是空串，交出去等于开"文档"）。
#[test]
fn the_folder_to_open_is_the_one_holding_rclone_own_answer() {
    assert_eq!(
        config_folder_of("C:\\Users\\me\\AppData\\Roaming\\rclone\\rclone.conf").as_deref(),
        Some("C:\\Users\\me\\AppData\\Roaming\\rclone"),
        "Windows 那一份要交回它所在的目录");
    assert_eq!(
        config_folder_of("/home/me/.config/rclone/rclone.conf").as_deref(),
        Some("/home/me/.config/rclone"),
        "同一颗在类 Unix 的路径上也是取所在目录");
    assert_eq!(config_folder_of(""), None, "问不出就不该开任何东西");
    assert_eq!(config_folder_of("rclone.conf"), None, "只有文件名：那一层是谁都不知道");
    assert_eq!(config_folder_of("/rclone.conf"), None, "挂在根上：交出去的是空串，那不是目录");
}

/// 设置面那一份 `JobDraft` 必须带着**与运行那一面同一个**配对键与槽位。
///
/// 钉的是"两份派生值相等"，不是"有个键就行"：设置面现在按这枚键把任务分组显示，运行面按它
/// 挑卡 —— 两边各算一份的那天，就是"我改的这条不是我看到的那条"那天（同一族判据见
/// `app_state.rs` 里"exclude 与 flags 必须同一份 Defaults"那条）。
#[test]
fn the_settings_row_carries_the_same_pair_key_as_the_running_row() {
    use rclone_deck_lib::config::jobs::JobsFile;
    use rclone_deck_lib::state::JobView;
    // 夹具同上：出厂那份已瘦身，这条要的是多对位置
    const FULL: &str = include_str!("fixtures/full-jobs.toml");
    use std::collections::BTreeMap;

    let parsed = JobsFile::parse(FULL).expect("夹具必须能解析");
    assert!(parsed.jobs.len() >= 8,
        "这一条靠出厂那一份覆盖两种模式与两个槽位，条目太少就等于没在证明什么");

    let mut per_key: BTreeMap<String, usize> = BTreeMap::new();
    for j in &parsed.jobs {
        let view = JobView::new(j, None, None, false);
        let draft = JobDraft::from_job(j);
        assert_eq!(draft.pair_key, view.pair_key, "{} 的配对键有两份口径", j.id);
        assert_eq!(draft.pair_slot, view.pair_slot, "{} 的槽位有两份口径", j.id);
        assert_eq!(draft.src_is_local, view.src_is_local, "{} 说错了哪一侧是本地", j.id);
        assert_eq!(draft.dst_is_local, view.dst_is_local, "{} 的另一侧说错了", j.id);
        assert_eq!(draft.a_is_local, view.a_is_local, "{} 的这一对的 a 面说错了", j.id);
        *per_key.entry(draft.pair_key).or_insert(0) += 1;
    }
    // 分组要真的合起来东西：出厂那一份里必须至少有一对位置带着两条（同一对的两个方向或两种模式）。
    assert!(per_key.values().any(|n| *n >= 2),
        "出厂那一份里没有任何一对位置带两条任务，这一条就没在证明分组：{:?}", per_key);
}
