use rclone_deck_lib::config::jobs::{Job, JobKind, JobsFile};
use rclone_deck_lib::pair::{mirror_sides_ok, pair_slot_of};
use rclone_deck_lib::state::{write_default_jobs_toml, JobView, DEFAULT_JOBS_TOML};

/// 夹具：内容丰富但纯属虚构的任务表。**为什么不由出厂配置喂**——出厂那份 2026-10-09 起
/// 瘦身成一条示范（见 `state.rs` 的 `DEFAULT_JOBS_TOML` 顶上那段），而这几条证人要的是
/// 多对位置 / 多条对账 / 会删的那几档。理由与形状要求写在夹具文件自己的头上。
const FULL: &str = include_str!("fixtures/full-jobs.toml");

/// 出厂配置必须能解析，而且**只**留一条示范（2026-10-09 起）。
///
/// 这条判据整个换过向。原来它钉的是"有 gate、有 sync、sync 的名字写清了会删"——
/// 那是在**为出厂配置里的会删任务背书**。现在出厂配置**一条会删的都不发**
/// （会删那一档必须由人自己配出来、配对了、看清了才按），所以判据反过来：
/// **不许有 sync、不许有 gate**，且任务条数就是 1。
///
/// 为什么要瘦身：改之前出厂那份抄的是作者本机的真实配置，别人装上 `rclone_bin`
/// 指向一个不存在的文件（后台起不来），而屏上摆着 10 条任务、其中 4 条写着会删。
/// 多对位置 / 多条对账 / 会删那几档的形状，改由 `fixtures/full-jobs.toml` 供。
#[test]
fn the_shipped_default_is_one_harmless_demo_and_nothing_that_deletes() {
    let f = JobsFile::parse(DEFAULT_JOBS_TOML).expect("出厂默认配置必须能解析");
    assert!(!f.jobs.is_empty(), "出厂连一条示范都没给：第一次用的人无从下手");
    assert_eq!(f.jobs.len(), 1,
        "出厂只该有一条示范（会删的、以及别人家的桶都不该跟着出厂发）：{:?}",
        f.jobs.iter().map(|j| (&j.id, &j.name)).collect::<Vec<_>>());
    assert!(!f.jobs.iter().any(|j| j.kind == JobKind::Sync),
        "出厂又发了一条会删东西的任务：{:?}", f.jobs);
    assert!(!f.jobs.iter().any(|j| j.kind == JobKind::Gate),
        "出厂又发了对账任务：它守的是作者自己的目录，别人装上没有意义：{:?}", f.jobs);
    // 剩下那一条必须是**只增不删**的那一档，且是「不删」写在名字里的。
    let only = &f.jobs[0];
    assert_eq!(only.kind, JobKind::Copy, "示范那一条不是 copy：{only:?}");
    assert!(only.name.contains("不删"),
        "示范那一条没在名字里说不删：{:?}", only.name);
}

/// 出厂配置里**不许出现任何一个人的家**：路径要么是绝对路径，要么留空由面板按当前用户算。
///
/// 2026-10-09 之前这条判据只管 `rclone_bin` 与 `log_dir` **都得是绝对路径**，而出厂那份
/// 写的正是作者那台的 `C:\Users\Administrator\...` 与 `D:\Sync\logs\...` —— 判据过了，
/// 别人装上却指向别人家。现在 `log_dir` 允许留空（`config::AppConfig::from_defaults`
/// 那一档按 `%LOCALAPPDATA%` 算），所以要钉的是**新契约**：
/// ① `rclone_bin` 仍必须是绝对路径（不赌 PATH 顺序）；② `log_dir` 要么绝对、要么空；
/// ③ **两个占位里都不许出现作者的用户名**。
#[test]
fn the_shipped_default_names_nobodies_home() {
    let f = JobsFile::parse(DEFAULT_JOBS_TOML).unwrap();
    assert!(std::path::Path::new(&f.defaults.rclone_bin).is_absolute(),
        "rclone_bin 必须是绝对路径：本机 PATH 上有两个 rclone，靠 PATH 顺序选是赌博");
    // log_dir：空 = 走默认落点（%LOCALAPPDATA%\rclone-deck\logs）；非空必须是绝对路径
    if !f.defaults.log_dir.trim().is_empty() {
        assert!(std::path::Path::new(&f.defaults.log_dir).is_absolute(),
            "log_dir 留空可以，非空就必须是绝对路径：{:?}", f.defaults.log_dir);
    }
    for (field, value) in [("rclone_bin", &f.defaults.rclone_bin), ("log_dir", &f.defaults.log_dir)] {
        assert!(!value.contains("Administrator"),
            "{field} 指向了作者那台机的家：{value:?}");
    }
    // 整份出厂配置都不许带别人的家目录（`D:\vault\...` 那种）
    assert!(!DEFAULT_JOBS_TOML.contains("Administrator"),
        "出厂配置里出现了作者的用户名：别人装上会指向一个不存在的家");
}

/// `log_dir` 留空时算出来的默认落点必须是**绝对路径**，且与 pid 文件同一个根。
#[test]
fn an_empty_log_dir_resolves_to_an_absolute_path_beside_the_pid_file() {
    use rclone_deck_lib::config::jobs::Defaults;
    use rclone_deck_lib::config::AppConfig;
    // 不设 RCLONE_DECK_HOME：走 %LOCALAPPDATA%\rclone-deck
    std::env::remove_var("RCLONE_DECK_HOME");
    let cfg = AppConfig::from_defaults(&Defaults {
        rclone_bin: "C:\\rclone\\rclone.exe".into(),
        flags: vec![],
        exclude: vec![],
        log_dir: "".into(),
        check_by_hash: false,
    }).unwrap();
    assert!(cfg.log_dir.is_absolute(), "空 log_dir 算出来的不是绝对路径：{:?}", cfg.log_dir);
    assert!(cfg.pid_path.is_absolute());
    // 同源：日志目录是 pid 文件那个根的兄弟
    assert_eq!(cfg.log_dir.parent(), cfg.pid_path.parent(),
        "日志与 pid 不在同一个根下：{:?} vs {:?}", cfg.log_dir, cfg.pid_path);
}

#[test]
fn the_shipped_default_carries_no_secret_shaped_key() {
    // 这不是安全边界，是回归网：将来有人往出厂配置里塞凭据时它会响。
    let lower = DEFAULT_JOBS_TOML.to_lowercase();
    for word in ["access_key", "secret", "token", "password", "api_key"] {
        assert!(!lower.contains(word), "出厂 jobs.toml 里出现了 {word}");
    }
}

/// spec §5.2 的判据不是"ideahub 需要对账"，而是**任何会被外部引用的前缀都必须有自己的 gate 任务**，
/// 并且点名 §4 默认要生成 `ideahub-covers` 与 `imgprompt` 两条。那个被边缘缓存一年的
/// 404 就落在 `imgprompt/` 下——只配一条拦不住它，而 §9 最后一行的标准是"拦住**且覆盖所有被外部
/// 引用的前缀**"。这条证人把那两个前缀钉死：新增被外部引用的目录时，忘了加 gate 就会红。
#[test]
fn every_externally_referenced_prefix_has_its_own_gate_job() {
    let f = JobsFile::parse(FULL).unwrap();
    let gates: Vec<&str> = f.jobs.iter()
        .filter(|j| j.kind == JobKind::Gate)
        .map(|j| j.dst.as_str())
        .collect();
    for prefix in ["r2:img/ideahub/covers", "r2:img/imgprompt"] {
        assert!(gates.contains(&prefix),
            "出厂配置里 {prefix} 没有自己的 gate 任务；现有闸门任务：{gates:?}");
    }
}

#[test]
fn write_default_creates_the_parent_directory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("jobs.toml");
    write_default_jobs_toml(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_JOBS_TOML);
}

#[test]
fn job_view_renders_the_kind_as_a_lowercase_word() {
    let job = Job {
        id: "r2-up".into(), name: "R2 图床上传".into(), kind: JobKind::Copy,
        src: "D:\\vault\\img-r2".into(), dst: "r2:img".into(),
        open_dir: Some("D:\\vault\\img-r2\\imgprompt".into()), manifest: None,
    };
    let v = JobView::new(&job, None, None, false);
    assert_eq!(v.kind, "copy");
    assert_eq!(v.id, "r2-up");
    assert!(v.locked_by.is_none());
}

#[test]
fn job_view_names_who_holds_the_lock() {
    let job = Job {
        id: "r2-up".into(), name: "R2 图床上传".into(), kind: JobKind::Copy,
        src: "a".into(), dst: "b".into(), open_dir: None, manifest: None,
    };
    let v = JobView::new(&job, Some("R2 图床下载".into()), None, false);
    assert_eq!(v.locked_by.as_deref(), Some("R2 图床下载"));
}

// ---- 裁定 R69：端点类型是后台算好带出去的数据，不是前端的第二份正则 ----
//
// `MirrorConfirm` 的「哪一侧是全集」和 `canInventory` 的显隐判断以前各自拿正则认盘符/具名 remote。
// 三份口径（`endpoint::remote_split`、`types.ts` 的 `WINDOWS_PATH`、`localWins`）互相声明一致，
// 但一致处只写在注释里：手写的 UNC（`\\server\share\img`）在 Rust 是本地、在那条前端正则里不是，
// 于是确认层会说"云端是全集"，而内核其实在删云端多余的对象。现在前端只读这两个字段。
// 字段名同时钉住：改名等于让前端读到 undefined，而 `undefined` 在布尔判断里是 false，不是报错。
#[test]
fn the_endpoint_types_reach_the_wire_as_data_under_those_names() {
    let f = JobsFile::parse(FULL).unwrap();
    let views: Vec<_> = f.jobs.iter().map(|j| JobView::new(j, None, None, false)).collect();

    let up = views.iter().find(|v| v.id == "r2-up").expect("出厂少了 r2-up");
    assert!(up.src_is_local && !up.dst_is_local, "上传行应该 src 本地、dst 远端：{up:?}");

    // 下载方向是"前提反过来"的那一行：remote 在 src。它必须由数据而不是正则说清，
    // 否则对齐确认层会按 src 是本地来念"云端是全集"。
    let down = views.iter().find(|v| v.id == "r2-mirror-remote-wins").expect("出厂少了那条反向镜像");
    assert!(!down.src_is_local && down.dst_is_local, "反向镜像行说反了：{down:?}");

    let json = serde_json::to_string(up).unwrap();
    assert!(json.contains("\"srcIsLocal\":true"), "srcIsLocal 没上线：{json}");
    assert!(json.contains("\"dstIsLocal\":false"), "dstIsLocal 没上线：{json}");
}

/// 这条钉的是"前端不再自己判"。**只禁某一种正则写法挡不住绕过**（`job.src[1] !== ':'`、
/// `job.src.includes(':')` 都不含那条字面量），所以禁的是**判断函数体内**读端点原始串这件事：
/// 分类只能读后台给的字段，显示（比如把路径印出来）当然还能读 `job.src`——那是另一回事。
#[test]
fn the_frontend_reads_the_endpoint_fields_instead_of_re_matching_them() {
    let types = include_str!("../../src/types.ts");
    let confirm = include_str!("../../src/components/MirrorConfirm.tsx");
    // 先确认两边都真的在用字段：没有这一条，下面那三条"没读到原始串"会因为整段被删掉而一起变绿。
    assert!(types.contains("job.srcIsLocal") && types.contains("job.dstIsLocal"),
        "canInventory 没有读后台的端点字段");
    assert!(confirm.contains("job.srcIsLocal"), "localWins 没有读后台的端点字段");

    for (name, src, head) in [
        ("types.ts", types, "export function canInventory"),
        ("MirrorConfirm.tsx", confirm, "function localWins"),
    ] {
        let body = fn_body(src, head);
        for raw in ["job.src;", "job.src)", "job.src[", "job.src.", "job.dst;", "job.dst)", "job.dst[", "job.dst."] {
            // `job.srcIsLocal` 也以 `job.src` 开头，所以比的是"后面接的不是 I"的那几种写法。
            assert!(!body.contains(raw), "{name} 的 {head} 在判断里直接读端点原始串 {raw}：那等于又立一份口径");
        }
        assert!(!body.contains(".test("), "{name} 的 {head} 里还有正则：分类必须读后台字段");
    }
}

/// 取一个函数从头部到**第一个配对的收尾大括号**为止的源码文本。
/// 够用了：这两个判断都是单表达式函数。找不到就 panic，别静默返回空串——空的 `body`
/// 会让上面每一条断言全部为真，那正是"永远绿的证人"。
fn fn_body(src: &str, head: &str) -> String {
    let from = src.find(head)
        .unwrap_or_else(|| panic!("源文本里找不到 {head}：判断换了形状或改了名，就来看这条证人"));
    let rest = &src[from..];
    let open = rest.find('{').expect("{head} 后面没有函数体");
    let mut depth = 0usize;
    for (i, c) in rest[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return rest[open..=open + i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("{head} 的大括号没有闭合");
}

/// 顶栏要显示日志目录，所以这个名字得上线；空串是"还不知道"，不是"目录是空"。
/// 这条存在的理由是一条**操作规程**：用夹具家跑面板时必须一眼看出面板走在夹具里，
/// 而不是落进 `%APPDATA%\rclone-deck`（那起事故记在 R58/R68）——而顶栏以前没有这一行。
#[test]
fn the_log_dir_reaches_the_wire_so_the_top_bar_can_show_it() {
    use rclone_deck_lib::state::RcdStatus;
    let mut locks = std::collections::BTreeMap::new();
    locks.insert("a=>b".into(), "R2 图床上传".into());
    let s = RcdStatus {
        ok: true, alive: true, port: 5599, pid: 41024, rclone_version: "v1.75.1".into(),
        execute_id: "e1".into(), startup_error: None, locks,
        log_dir: "C:\\fixture\\logs".into(),
        config_path: "C:\\fixture\\jobs.toml".into(),
        active_count: 1, size_only: false,
    };
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains("\"logDir\":\"C:\\\\fixture\\\\logs\""), "logDir 没上线或值被改：{json}");
    // 配置文件那条路径也走同一条路：界面写死它 = 面板跑在别的家上时那句话在骗人（R68）。
    assert!(json.contains("\"configPath\""), "configPath 没上线，顶栏那句只能自己编路径：{json}");
    // "现在有几件事正在做"也必须是后台数好的：界面数 `locks` 的键就是第二份判定（I5），
    // 而键按 Scope 分，一次动作可以申领多个键 —— 数键会把一件事报成两件。
    assert!(json.contains("\"activeCount\":1"), "activeCount 没上线，顶栏只能自己数键：{json}");
    assert!(json.contains("\"alive\":true"), "存活那枚没上线，顶栏只能永远说「后台就绪」：{json}");

    // 反面：读不到落点那一档，空串必须是空串——不许拿一个猜的路径去填。
    // （`rcd_status` 在后台没起来时**会**试着从配置读 log_dir：那一刻正是最需要知道
    //   "面板跑在哪个家"的时候；读不到才留空串，界面仍不说。）
    let down = RcdStatus {
        ok: false, alive: false, port: 0, pid: 0, rclone_version: String::new(), execute_id: String::new(),
        startup_error: Some(rclone_deck_lib::note::note_for_text("无法启动 rclone rcd：程序没找到")),
        locks: Default::default(), log_dir: String::new(),
        config_path: String::new(), active_count: 0, size_only: false,
    };
    let json = serde_json::to_string(&down).unwrap();
    assert!(json.contains("\"logDir\":\"\""), "读不到落点那一档不该编一个日志路径：{json}");
    assert!(json.contains("\"activeCount\":0"), "{json}");
    assert!(json.contains("\"alive\":false"), "起不来那一档也要明确说不在：{json}");
    // 起不来那一句上线时**已经分成两段**：给人看的那一段里不许有内部名词
    // （顶栏是常驻面，原文摆上去就等于界面上常驻一个 `rcd`）。
    let note = down.startup_error.clone().expect("这一档就是起不来那一档");
    assert!(!note.short.contains("rcd"), "摘要里还有内部名词：{}", note.short);
    assert!(note.detail.contains("rcd"), "排障原文没留着，那一段就被藏了：{note:?}");
    assert!(json.contains("\"startupError\":{\"short\""), "那一份没按两段上线：{json}");
}

/// 「只比大小」那一枚改的是**每一趟的判定口径**，所以它得上线成一枚数据，让顶栏能常驻说一句。
///
/// 口径由后台算（`config::jobs::compares_by_size`，读的是启动时那一份 `Defaults.flags` ——
/// 与 rcd 的 argv 同源，见下面那条"同一份 Defaults"的证人）。界面自己从 flags 猜就是第二份判定，
/// 而 flags 从来没上过屏：猜的那一份迟早会和跑的那一份分家，分家那天屏上就有一句谎话。
#[test]
fn the_comparison_basis_reaches_the_wire_so_the_top_bar_can_say_it() {
    use rclone_deck_lib::state::RcdStatus;
    let status = |size_only: bool| serde_json::to_string(&RcdStatus {
        ok: true, alive: true, port: 5599, pid: 41024, rclone_version: "v1.75.1".into(),
        execute_id: "e1".into(), startup_error: None, locks: Default::default(),
        log_dir: "C:\\fixture\\logs".into(),
        config_path: "C:\\fixture\\jobs.toml".into(),
        active_count: 0, size_only,
    }).unwrap();
    assert!(status(true).contains("\"sizeOnly\":true"), "开着那一档时判定没上线：{}", status(true));
    // 反面同样要钉：默认那一档必须说 false，界面才会把这句话整项藏了
    // （常驻一句没在跑的比法，比不说更坏）。
    assert!(status(false).contains("\"sizeOnly\":false"), "默认比法那一档要说 false：{}", status(false));
}

/// `rcd_status` 里那一枚必须**是问出来的**，不是写死的。
///
/// 形状判据而不是值判据：后台没起来那一档填 `false` 是对的（那一刻没在跑），所以"文件里有 `size_only: false`"
/// 不构成红。红的是**就绪那一档**不再读 `runner` 那一份 —— 硬编码成任何一边都能让上面那条上线证人绿着，
/// 而屏上那句从此永远不说、或永远乱说。
#[test]
fn the_served_basis_is_asked_from_the_runner_not_hard_coded() {
    let cmds = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/commands.rs"))
        .expect("读 commands.rs");
    let at = cmds.find("pub async fn rcd_status").expect("没有 rcd_status 那一个命令");
    let body = &cmds[at..];
    let end = body.find("\n}\n").map(|i| &body[..i]).unwrap_or(body);
    assert!(end.contains("size_only: runner.size_only()"),
        "就绪那一档不再问 runner 那一份判定（那就是把它写死了）：{end}");
}

/// 顶栏那一行的判据是**有没有**这个值，不是它长什么样：空串必须整项不显示。
/// 钉源码文本是因为这是界面侧唯一能钉的地方（本仓没有 JS 测试运行器）。
///
/// 另一半补的是：起不来那一档**也要**把落点说出来。旧写法在这一档把日志目录
/// 整项藏了（后台填空串、界面按"没有"处理），而那恰恰是最需要知道"面板跑在哪个家"的时候
/// （规程 R68 就是靠这一格当场发现两次误启动的）。数渲染点：两处 —— 就绪那一档与起不来那一档。
#[test]
fn the_top_bar_hides_the_log_dir_when_it_is_not_known() {
    let bar = include_str!("../../src/components/RcdStatusBar.tsx");
    assert!(bar.contains("status.logDir === ''"),
        "顶栏没有按\"有没有值\"决定显不显示：空路径会比不显示更像骗人");
    assert!(bar.contains("{status.logDir}"), "顶栏没把路径本身摆出来（只显示一个标签等于没说）");
    let renders = bar.matches("<LogDir status={status} />").count();
    assert_eq!(renders, 2,
        "落点那一格必须在**就绪**与**起不来**两档都渲染（数到 {renders} 处）：\
         起不来的时候把它藏了，就等于在最需要它的那一刻拆掉规程 R68 那道眼前证据");
}

// ---- 裁定（Task 7-9 之后补的口径）：同一份 Defaults 必须同时喂给 rcd 与 runner ----
//
// `bootstrap` 里 rcd 的启动参数（`RcdSpec::from_config` 展开的 `--exclude`）和 runner 的本地
// 遍历排除（`checks::expected_keys(job, &self.defaults.exclude)`）是**同一份清单的两个匹配器**。
// 一旦 `bootstrap` 把 `jobs.toml` 解析两次、各喂一个 `Defaults`，两侧就会漂移，而漂移的下游
// 是一次删除（本地认为该排除的对象，rcd 不排；对账集合就此错位）。这里用一个 exclude **不同于
// 出厂默认**的夹具证人：两侧拿到的必须是同一份——断言的是两个派生值相等，不是某个常量。
#[test]
fn the_rcd_and_the_runner_get_their_exclude_from_one_and_the_same_defaults() {
    use rclone_deck_lib::config::AppConfig;
    use rclone_deck_lib::config::jobs::Defaults;
    use rclone_deck_lib::state::shared_defaults;

    // 夹具的 exclude 特意带一个出厂默认里没有的标记串：漂移一旦发生就落不到同一个值上。
    let marker = "只属于这份夹具的排除项-9f3c2a.webp".to_string();
    let fixture = Defaults {
        rclone_bin: "C:\\绝对路径\\rclone.exe".into(),
        flags: vec!["--fast-list".into()],
        exclude: vec![marker.clone()],
        log_dir: "C:\\绝对路径\\logs".into(),
        check_by_hash: false,
    };
    // 出厂默认必须和夹具不同，否则这条证人就退化成"两个常量相等"，抓不到"解析了两次"。
    let shipped = JobsFile::parse(DEFAULT_JOBS_TOML).unwrap();
    assert!(!shipped.defaults.exclude.contains(&marker),
        "夹具的 exclude 标记落进了出厂默认，证人失去区分力");

    let app = AppConfig::from_defaults(&fixture).expect("夹具必须能进 AppConfig");
    let (spec_for_rcd, defaults_for_runner, _jobs) =
        shared_defaults(&app, fixture.clone(), vec![]);

    // rcd 拿到的启动参数清单 == runner 会拿到的那份清单（同一个 Defaults 分出去的两个出口）。
    assert_eq!(spec_for_rcd.exclude, defaults_for_runner.exclude,
        "rcd 的 --exclude 和 runner 的排除清单不是同一份：漂移的下游是删除");
    // 而且它就是夹具那份，不是恰好都为空的假绿。
    assert_eq!(spec_for_rcd.exclude, fixture.exclude);
}

// ---------------------------------------------------- spec §3：行上那两个数带着"几点算的"（Task 14）

#[test]
fn the_last_inventory_reaches_the_wire_as_data_with_the_time_it_was_measured() {
    // 少了时刻，一次旧读数就会被当成现状；少了字段，前端就只能自己去算——两件都不许。
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/state.rs")).expect("读得到 state.rs");
    // 计划原文写的是 `fn_body(&src, "pub fn job_view")`，那个名字不存在（`JobView` 的构造器叫
    // `new`），`fn_body` 会 panic 在"找不到"——那是空转不是红（裁定 R115）。
    let body = fn_body(&src, "pub fn new(job: &Job");
    assert!(body.contains("last_summary"), "JobView 没把最近一次盘点带出去：{body}");
    let types = include_str!("../../src/types.ts");
    assert!(types.contains("lastSummary"), "TS 那一份形状里没有它：界面上就是读不到");
    assert!(types.contains("measuredAt"), "行上要说「几点算的」，这个字段必须在");
}

#[test]
fn the_panel_never_runs_an_inventory_on_its_own() {
    // 结构证人：初始化路径里没有盘点调用点。算一行摘要要列两侧，开窗刷 10 行就是几十次计费 LIST。
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/state.rs")).expect("读得到 state.rs");
    let body = fn_body(&src, "pub async fn bootstrap");
    assert!(!body.contains("run_inventory"), "开面板就盘点：{body}");
}

// ---- 一条配对 = 一张卡，方向与模式是卡上的两个选择框 ----
//
// 操作员的原话是「以本地为主、以云端为主应该是选择框，这样更直观吧，而不是 N 条不同的条目」。
// 数一下出厂那 10 条：2 对位置 × {上传、下载、镜像两个方向} + 2 条对账 —— 方向被当成了身份，
// 所以"哪条是合适的"在界面上没有答案。
// 分组键必须由**后台**算：它问的是"这是不是同一对目录"，而锁问的正是同一件事（`Scope`：
// 同一棵树、大小写不敏感、两种斜杠）。两份口径迟早会漂，所以这里复用 `Scope::key()`，
// 不在前端再拼一个方向串（同一族的旧错：`Job::mutex_key()` 那个 `src=>dst` 串，见 `scope.rs` 头上）。

#[test]
fn the_pair_key_is_one_per_position_pair_and_ignores_direction_and_kind() {
    let f = JobsFile::parse(FULL).unwrap();
    let views: Vec<_> = f.jobs.iter().map(|j| JobView::new(j, None, None, false)).collect();
    let key_of = |id: &str| views.iter().find(|v| v.id == id)
        .unwrap_or_else(|| panic!("出厂少了 {id} 这一条")).pair_key.clone();

    // 同一对位置的四条（只补，不删的两个方向 + 让两边对齐的两个方向）必须是同一个键。
    let first = key_of("r2-up");
    for id in ["r2-down", "r2-mirror-local-wins", "r2-mirror-remote-wins"] {
        assert_eq!(first, key_of(id),
            "{id} 与 r2-up 是同一对位置，键却不同：分组会把一张卡拆回四条");
    }
    // 另一对不许并进来（键相等在界面上就等于"它们是同一张卡"）。
    assert_ne!(first, key_of("qiniu-up"), "r2 与七牛两对位置被并成了一张卡");
    // 对账那一类守的是**前缀**（§5.2），不是这一对的两个端点：它不许蹭配对卡。
    assert_ne!(first, key_of("ideahub-covers"), "对账那条被并进了配对的卡里");
    assert_ne!(key_of("ideahub-covers"), key_of("imgprompt"), "两条对账自己也不该同键");
}

/// 键的规范化与锁同源：反斜杠、大小写、结尾斜杠都不许造出"另一对位置"。
/// 否则界面上会出现两张长得一样的卡，而按下去动的是同一棵树——那比"N 条条目"更糟。
#[test]
fn the_pair_key_normalizes_separators_and_case_like_the_lock_does() {
    let key = |src: &str, dst: &str| {
        JobView::new(&Job {
            id: "k".into(), name: "k".into(), kind: JobKind::Copy,
            src: src.into(), dst: dst.into(), open_dir: None, manifest: None,
        }, None, None, false).pair_key
    };
    assert_eq!(key("D:\\vault\\img-r2", "r2:img"), key("d:/vault/img-r2/", "r2:img"),
        "同一个目录的两种写法算出了两对位置");
    // 方向反过来还是同一对（这是"选择框"这件事的根）。
    assert_eq!(key("D:\\vault\\img-r2", "r2:img"), key("r2:img", "D:\\vault\\img-r2"));
    // 桶名不同就是不同的一对，不许因为 root 相同而并卡。
    assert_ne!(key("D:\\vault\\img-r2", "r2:img"), key("D:\\vault\\img-r2", "r2:imgprompt"));
}

/// 后台不许再自己摆一份"四格表"。那张表原来在这里（`state::PairView` + `pair_views`），
/// 它是界面上那张卡的第二份实现：卡是 `src/pairs.ts` 按 `pairKey`/`pairSlot`/`kind`
/// **划分**出来的，而划分要用那把坐标系（键、槽位、这一对能不能当对齐）住在 `src/pair.rs`，
/// 被 `config`（对齐那一档能不能建）与 `runner::drafts`（这张卡还缺哪几格）一起读着。
/// 一份没人走的实现替"四格各只有一条"作证，等于那条安全性有两个户口、零个读者。
/// 这一条钉的是**别再长回来**：`src/` 里再出现那个产物或它的构造函数就红。
#[test]
fn the_backend_does_not_build_a_second_pair_card() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let banned = ["struct PairView", "fn pair_views"];
    let mut hits = Vec::new();
    collect_named(&root, &banned, &mut hits);
    assert!(hits.is_empty(), "后台里又长出一份配对卡：{hits:?}");
}

fn collect_named(dir: &std::path::Path, banned: &[&str], hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("读不到 src/") {
        let path = entry.expect("目录项可读").path();
        if path.is_dir() { collect_named(&path, banned, hits); continue; }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") { continue; }
        let text = std::fs::read_to_string(&path).expect("源文件可读");
        for (i, line) in text.lines().enumerate() {
            for b in banned {
                if line.contains(b) {
                    hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                }
            }
        }
    }
}

/// 走线两边同名。这一条不是形式主义：前端漏声明这个字段时读到的是 `undefined`，
/// 而 `undefined === undefined` —— **所有卡会并成一张**，界面上看着像"终于清爽了"。
#[test]
fn the_pair_key_reaches_the_wire_under_that_name() {
    let types = include_str!("../../src/types.ts");
    assert!(types.contains("pairKey"), "TS 那一份形状里没有 pairKey：分组会把所有卡并成一张");
    let rs = include_str!("../../src-tauri/src/state.rs");
    assert!(rs.contains("pub pair_key: String"), "Rust 那一份没带这个字段：{rs}");
    // 一条任务站 a 还是 b 也必须由后台说：前端若自己再判一次"哪侧是本地、谁排前面"，
    // 就是第二份口径 —— 它漂开的那一次，同一对目录会被分成两张卡，或者两张卡动同一棵树。
    assert!(types.contains("pairSlot"), "TS 那一份没接这个字段：前端只能自己排序号，那是第二份口径");
    assert!(rs.contains("pub pair_slot: &'static str"), "Rust 那一份没带槽位：{rs}");
    assert!(types.contains("aIsLocal"), "卡上那两句标签没有可读的标志：前端会退回正则认路径");
    assert!(rs.contains("pub a_is_local: bool"), "Rust 那一份没带这个标志：{rs}");
    // 出厂那几条各自落在不同的格子里。原来这一条由 `pair_views` 那四个 `Option` 的形状担保，
    // 那个产物删掉之后判据不能跟着没：这里按界面真正读的三元组（哪张卡 / 哪一档模式 / 哪一侧）自己数。
    // 撞格不是难看而已——同一格装两条时，界面上显示的是其中一条，真会跑的可能是另一条。
    let f = JobsFile::parse(FULL).unwrap();
    let views: Vec<_> = f.jobs.iter().map(|j| JobView::new(j, None, None, false)).collect();
    for v in &views {
        assert!(v.pair_slot == "a" || v.pair_slot == "b", "{v:?} 的槽位不是 a/b");
    }
    let cs = cell_rows(&views);
    let uniq: std::collections::BTreeSet<_> = cs.iter().map(|r| (&r.0, &r.1, &r.2)).collect();
    assert_eq!(cs.len(), uniq.len(), "有一格装进了两条任务（看不见的那条才是真会跑的）：{cs:?}");
    assert_eq!(card_count(&views), 2, "出厂的两对位置并出的卡数不对：{cs:?}");
}

// ---- 一张卡 = 一对位置：这四格怎么摆，问的是界面真拿到的那三枚字段 ----
//
// 这一组原来钉的是后台那份 `pair_views` 的产物（四个 `Option<String>` 格子）。那个产物没有生产读者，
// 已经删（`the_backend_does_not_build_a_second_pair_card` 钉它别再长回来），但它替界面上那张卡守的东西
// 一条都不许跟着丢：
//   ① 一对位置只并出一张卡；② 每一格最多装一条，而且说得出格子里站的是谁；
//   ③ a 在前、b 在后**不随方向变**；④ 两侧同型时两面各自有标志；⑤ 对账不进配对卡。
// 为什么"缺的那格必须真的缺"要在后台这一侧判：前端若把「只补，不删 / 让两边对齐」× 「a / b」写成固定矩阵，
// 少配一档的那对目录就会在框里给出一个**没有任务**的选项 —— 按下去动的是 undefined，比"N 条条目"更坏。
// `ui-fixture` 那两套家永远四档齐全，前端测不出缺档，所以这一条只有纯函数配得上证人。

/// 三条任务（上传、下载、只配了一个方向的对齐）→ 一张卡，框里只列存在的组合。
#[test]
fn a_pair_offers_only_the_combinations_that_have_a_job() {
    let jobs = vec![
        Job { id: "up".into(), name: "上传".into(), kind: JobKind::Copy,
            src: "D:\\v".into(), dst: "r2:img".into(), open_dir: None, manifest: None },
        Job { id: "down".into(), name: "下载".into(), kind: JobKind::Copy,
            src: "r2:img".into(), dst: "D:\\v".into(), open_dir: None, manifest: None },
        Job { id: "mir".into(), name: "镜像：删多余的".into(), kind: JobKind::Sync,
            src: "D:\\v".into(), dst: "r2:img".into(), open_dir: None, manifest: None },
    ];
    let views = views_of(&jobs);
    assert_eq!(card_count(&views), 1, "三条同一对位置的任务并成了多张卡：{:?}", cell_tags(&views));
    // 清单里**少的那一行**是这一条的重点：`sync:b` 不在，框里就不会出现一个没有任务的选项。
    assert_eq!(cell_tags(&views), ["copy:a=up", "copy:b=down", "sync:a=mir"],
        "四格里哪一格站着哪一条，读起来是这个样子");
    // a 在前、b 在后，**不随方向变**（方向是这一对上的一个选择，不是卡的身份）：
    // 恰好一侧是本地时，本地那一侧站 a，不管它写在 src 还是 dst。
    assert_eq!(pair_slot_of("D:\\v", "r2:img"), "a");
    assert_eq!(pair_slot_of("r2:img", "D:\\v"), "b");
    // 两格不叫 local/remote：复制任务允许两侧都是本地、也允许两侧都是 remote（只有对齐被强制
    // "恰好一侧是 remote"），叫 remote 就是替配置说谎。哪一侧是本地由 `aIsLocal` 与那一头的
    // `srcIsLocal`/`dstIsLocal` 带出去，界面上「以本地为主 / 以云端为主」那两句按这两枚标志挑。
    let up = views.iter().find(|v| v.id == "up").unwrap();
    assert!(up.a_is_local && !up.dst_is_local, "一侧本地一侧 remote 的那对，标志读反了：{up:?}");
}

/// 两侧都是本地的复制对（配置收这一形，只有对齐不收）：两侧都标成本地，
/// 界面上就不许冒出「以云端为主」那种说法。
#[test]
fn a_pair_of_two_local_directories_says_so_on_both_sides() {
    let jobs = vec![Job {
        id: "bak".into(), name: "备份到移动盘".into(), kind: JobKind::Copy,
        src: "D:\\vault".into(), dst: "E:\\copy".into(), open_dir: None, manifest: None,
    }];
    let views = views_of(&jobs);
    let v = &views[0];
    assert!(v.src_is_local && v.dst_is_local, "两侧都是本地：{v:?}");
    assert!(v.a_is_local, "两侧都是本地，a 那一侧的标志却不是本地：{v:?}");
    // b 那一面没有自己的字段：界面按槽位从这两枚里挑（`src/pairs.ts` 里 `bIsLocal` 那一句）。
    // 拿"a 取反"去推的话，这一对会被说成"一本地一云端"，而那一对根本没有云端可说。
    let b_is_local = if v.pair_slot == "a" { v.dst_is_local } else { v.src_is_local };
    assert!(b_is_local, "两侧都是本地，b 那一面读出来不是本地：{v:?}");
    // 两侧同型时按各自的键排，同一对在任何书写顺序下都是同一个 a（D 在 E 前）。
    assert_eq!(pair_slot_of("D:\\vault", "E:\\copy"), "a");
    assert_eq!(pair_slot_of("E:\\copy", "D:\\vault"), "b");
    assert_eq!(cell_tags(&views), ["copy:a=bak"], "复制那条被摆进了不该有它的一格：{:?}", cell_tags(&views));
    // 「会删除」那一格建不出来的判据就是这一条（`runner::drafts` 拿它决定建不建那两格）：
    // 没有权威的一端就没有删除范围，两侧都本地 ⇒ 一格都不建，而不是猜一个当权威。
    assert!(!mirror_sides_ok(&v.src, &v.dst), "两侧都本地却被当成可以镜像的一对");
}

/// 只配了对齐的一对：模式框里只该有「会删除」那一档。
#[test]
fn a_pair_with_only_one_mode_lists_only_that_mode() {
    let jobs = vec![Job {
        id: "mir".into(), name: "镜像：删多余的".into(), kind: JobKind::Sync,
        src: "r2:img".into(), dst: "D:\\v".into(), open_dir: None, manifest: None,
    }];
    let views = views_of(&jobs);
    let v = &views[0];
    // 对齐的下载方向：远端写在 src，但本地那一侧仍然站 a ⇒ 这一条落在 b 那格。
    assert_eq!(v.pair_slot, "b", "本地那一侧不管出现在 src 还是 dst，都排在卡面的前面：{v:?}");
    assert!(v.a_is_local && !v.src_is_local,
        "a 的标志跟着这条任务的 src 走了（它该说的是 a 那一侧，不是这一条的 src）：{v:?}");
    assert_eq!(card_count(&views), 1);
    // 「只补，不删」那两格不在清单里 = 「要做什么」那枚框里只出现「让两边对齐」那一档。
    assert_eq!(cell_tags(&views), ["sync:b=mir"], "{:?}", cell_tags(&views));
}

/// 对账守的是**前缀**（spec §5.2），不是这一对的两个端点：它永远不进配对卡。
/// 这条分界原来由 `pair_views` 里那一行 `if j.kind == JobKind::Gate { continue; }` 守着；
/// 产物删了，判据搬到**界面真在跑的那一处**（`src/pairs.ts::groupJobs`）：它若先按 pairKey 分组、
/// 再把对账挑出去，对账的前缀就占掉过一张卡的格子，界面上那条会伪装成"模式选项"。
#[test]
fn gate_jobs_never_join_a_pair_card() {
    let f = JobsFile::parse(FULL).unwrap();
    let gate_ids: Vec<&str> = f.jobs.iter().filter(|j| j.kind == JobKind::Gate)
        .map(|j| j.id.as_str()).collect();
    assert!(gate_ids.len() >= 2, "出厂那两条对账不见了：{gate_ids:?}");
    let ts = include_str!("../../src/pairs.ts");
    let group = &ts[ts.find("export function groupJobs").expect("找不到 groupJobs")..];
    let filter = group.find("kind === 'gate'")
        .expect("分组不再把对账分出去：它会并进配对卡");
    let guard = &group[filter..];
    let body = &guard[..guard.find('}')
        .expect("对账那一句没有块体：判不出它分完有没有接着往下走") + 1];
    // 只"分出去"不够：分完必须**跳过**后面的按 pairKey 归组，否则对账既进 gates 又占掉一张卡的格子。
    // （这一条是被破坏枪逼出来的：把 `continue` 抹掉，只看两句先后秩序的证人一声不响。）
    assert!(body.contains("continue"), "对账分出去了但没有跳过分组：{body}");
    let group_by_key = group.find("byKey.get(j.pairKey)")
        .expect("找不到按 pairKey 分组那一句：判据无从比较先后");
    assert!(filter < group_by_key, "对账是先并进卡、再挑出去的：它的前缀占掉了一张卡的格子");
    // 数据那一半：把对账分出去之后，出厂剩下的正好两张配对卡（r2 与七牛）。
    let views = views_of(&f.jobs);
    assert_eq!(card_count(&views), 2, "配对卡数不对：{:?}", cell_tags(&views));
}

fn views_of(jobs: &[Job]) -> Vec<JobView> {
    jobs.iter().map(|j| JobView::new(j, None, None, false)).collect()
}

/// 界面上那张卡的四格，按它**真正读到的那三枚字段**摆出来：`(哪张卡, 哪一档模式, 哪一侧, 哪一条)`。
/// 对账不进卡，与 `src/pairs.ts::groupJobs` 同一分界（上面那条证人钉的是它的先后）。
fn cell_rows(views: &[JobView]) -> Vec<(String, String, String, String)> {
    let mut rows: Vec<_> = views.iter().filter(|v| v.kind != "gate")
        .map(|v| (v.pair_key.clone(), v.kind.clone(), v.pair_slot.to_string(), v.id.clone()))
        .collect();
    rows.sort();
    rows
}

/// 同一对位置上的那一格一格：`"模式:槽位=任务 id"`。清单里少一行就是那一格没配，
/// 而"没配的那格不许出现在框里"这件事，只有把清单整个摆出来才看得见。
fn cell_tags(views: &[JobView]) -> Vec<String> {
    cell_rows(views).iter().map(|r| format!("{}:{}={}", r.1, r.2, r.3)).collect()
}

/// 有几张配对卡 = 有几个不同的 `pairKey`（同一分界，先把对账分出去）。
fn card_count(views: &[JobView]) -> usize {
    let mut keys = std::collections::BTreeSet::new();
    for r in cell_rows(views) { keys.insert(r.0); }
    keys.len()
}
