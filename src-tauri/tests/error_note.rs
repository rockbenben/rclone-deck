//! 错误话术分两层（操作员那两条报告之后的那一片）。
//!
//! 钉的是**行为**而不是源文本：`note_for_text` 是个纯函数，可以直接喂真串。
//! 为什么不用源文本扫描：前端那条"界面不许出现内部名词"的证人扫的是 TSX 字面量，
//! 而这些串是运行时从 Rust 的错误模板拼出来的 —— 它全绿，界面上照样出现
//! `rc 调用 operations/list 失败：error in ListJSON: … AccessDenied …`（操作员当天贴的就是这句）。

/// 这些串是**真的会上界面的那些**：前半来自 `rc/mod.rs`、`rcd/mod.rs`、`config/mod.rs`、
/// `runner/mod.rs` 的 `#[error(…)]` 模板，最后一条是操作员在浏览器那一栏读到的原文。
const REAL_INTERNAL_STRINGS: &[&str] = &[
    "rc 调用 operations/list 失败：error in ListJSON: operation error S3: ListBuckets, https response error StatusCode: 403, RequestID: , HostID: , api error AccessDenied: Access Denied",
    "rc 调用 core/lsf 返回 HTTP 400：bad parameter",
    "rc 调用 operations/rmdir 失败：directory not empty",
    "rc 调用 sync/copy 失败：object not found",
    "无法连接 rcd：error trying to connect: Connection refused (os error 10061)",
    // 真窗口里量到的那一条（把面板自己拉起的 rcd 停掉之后 reqwest 的措辞）：
    // 它既不含 "error trying to connect" 也不含 "Connection refused"，当时落到了通用的
    // "这一步没做成" —— 认得的原因说成不认得，也是一种说错话。
    "无法连接 rcd：error sending request for url (http://127.0.0.1:5599/operations/list)",
    "rcd 响应超时：operations/list（等了 90 秒）",
    "无法启动 rclone rcd：程序没找到",
    "rcd 在 20s 内没有响应 rc 请求：timeout",
    "jobs.toml 解析失败：expected a table",
    "pid 文件读写失败：找不到路径",
    "面板没能进入可用状态：config/get 不允许",
    "rc 调用 config/listremotes 失败：didn't find section \"r3\"",
];

/// 界面上不许出现的内部名词。与前端那条证人（`frontend_theme.rs`）同一族清单，
/// 但这一份扫的是**运行时真的会渲染出来的那一段**。
const BANNED_ON_SURFACE: &[&str] = &[
    "rcd", "rc 调用", "operations/", "core/", "sync/", "config/", "copyfile",
    "HTTP", "ListBuckets", "ListJSON", "AccessDenied", "StatusCode", "RequestID",
    "jobs.toml", "token", "pid 文件", "didn't find section",
];

#[test]
fn every_internal_error_shape_gets_a_human_sentence_and_keeps_the_original() {
    for raw in REAL_INTERNAL_STRINGS {
        let n = rclone_deck_lib::note::note_for_text(raw);
        assert!(!n.short.is_empty(), "没给出摘要：{raw}");
        for banned in BANNED_ON_SURFACE {
            assert!(!n.short.contains(banned),
                "摘要里还有内部名词 {banned}（原文：{raw}）：{}", n.short);
        }
        // 排障那一份必须一个字不少 —— 少了它，"给人看的那句"就变成藏信息。
        assert_eq!(n.detail, *raw, "原文没被完整留在 detail 里：{raw}");
        // 摘要不许只是把原文截短：它得是另一句话（否则这一层什么都没做）。
        assert_ne!(n.short, *raw, "摘要就是原文本身：{raw}");
    }
}

#[test]
fn our_own_human_sentences_pass_through_without_a_redundant_expander() {
    // 这一族本来就是说给人听的：整句就是摘要，`detail` 留空 ⇒ 界面上不出现一个
    // 点开来看还是同一句话的「为什么？」。
    for raw in [
        "另一条任务正在写同一棵目录树：photos-to-r2",
        "端口 5599 被别的进程占着：进程号 4112（rclone.exe）。面板不静默换端口，请先关掉它",
        "这个目录不是空的：里面有 3 个文件，所以删不掉。要连里面的一起清掉，用「清空这一层（连里面的一起）」。",
        "这一层变了，一个都没动。刚才给你看的是 3 个，现在是 4 个。要按新的清单再来一次，点「重新预览」。",
        "不能清空整个远端。根上一次误点就是整个远端。先进到要清的那一层。",
        // 这一条本来在"内部串"那份清单里，是它自己把分类问出来的：模板是我们自己的话，
        // 后面接的是操作系统的原话（"拒绝访问。"），整句里没有内部名词 ⇒ 走"整句即摘要"。
        // 判据是**有没有内部名词**，不是"这句是不是从错误类型里出来的"。
        "读写配置失败：拒绝访问。",
    ] {
        let n = rclone_deck_lib::note::note_for_text(raw);
        assert_eq!(n.short, *raw, "我们自己那句人话被改写了：{raw}");
        assert_eq!(n.detail, "", "多出一份与摘要重复的细节（界面上会多一个没内容的展开）：{raw}");
    }
}

#[test]
fn an_unrecognized_internal_string_is_not_dressed_up_as_a_known_reason() {
    // 认不出的**不编**：摘要只能说"没做成"，不许猜一个原因（`rmdir_refusal` 那一条的口径）。
    // 反面半边很重要：编一句"不是空的"比不给解释更坏，因为它会把人指到错的地方去。
    let raw = "rc 调用 operations/about 失败：something entirely new and unmeasured";
    let n = rclone_deck_lib::note::note_for_text(raw);
    assert_eq!(n.short, "这一步没做成。", "{n:?}");
    assert_eq!(n.detail, raw, "原文要留着：{n:?}");
    assert!(!n.short.contains("不是空的"), "认不出却编了一个原因：{}", n.short);
}

#[test]
fn an_empty_string_is_the_third_state_not_a_sentence() {
    // 空串不是"没有原因"，是"还不知道"。给它一句摘要就是替后台编话。
    let n = rclone_deck_lib::note::note_for_text("");
    assert!(n.is_empty(), "{n:?}");
}

/// 两种超时不是一件事，话也不许是同一句。
///
/// 别的 rc 调用是几百毫秒的事，超了基本等于"后台没了"；只有 `operations/list` 那一档是
/// "那个位置东西太多，一次没列完"（操作员点「看差异」拿到的就是它）。
/// 把它说成"等它回话等太久了"会把人支去查网络，而真实原因在他的桶里。
#[test]
fn the_two_timeout_families_say_different_things() {
    let list = rclone_deck_lib::note::note_for_text("rcd 响应超时：operations/list（等了 90 秒）");
    let other = rclone_deck_lib::note::note_for_text("rcd 响应超时：core/stats（等了 30 秒）");
    assert_eq!(list.short, "云端那一长串没列完。", "{list:?}");
    assert_eq!(other.short, "等它回话等太久了，没等到。", "{other:?}");
    // 反向半边才是这条证人的全部意义：列举那一句不许被并回通用那句。
    assert!(!list.short.contains("等它回话"), "列举超时又被说成了\"没人回话\"：{list:?}");
    // 原文（带方法名与秒数）必须完整留在展开里——那是排障要的那一句。
    assert_eq!(list.detail, "rcd 响应超时：operations/list（等了 90 秒）", "{list:?}");
}

mod common;
use common::{count_in, read, strip_comments};

/// 终态那句失败的话走的必须是**同一条通路**，而不是第三份裸串。
///
/// 上面几条钉的都是"命令返回的 `Err`"：那一路早就过 `note_for_text`。而
/// `TaskState::Failed.error` 是**数据字段**——它从 rclone 的 `job/status` 里原样穿过，
/// 分类器一次也没看过它，于是操作员在「任务失败」下面读到的是 `operation error S3: …`
/// 那一族原文（报的就是这一格）。
/// 所以这里钉的是**通路形状**：字段类型、后台构造它时过没过分类器、界面渲染时拿的是哪一段。
/// 措辞证人拦不住这一族回退——裸串回来的时候一句字面量都没改。
#[test]
fn the_terminal_failure_travels_as_two_parts_like_the_other_error_channels() {
    let runner = strip_comments(&read("src-tauri/src/runner/mod.rs"));
    // ① 那一臂的字段类型。取 `Failed { jobid: i64` 起手的 60 个**字符**（不是字节：
    //    后面紧跟的就是中文注释，按字节切会切在字中间）。
    let at = runner.find("Failed { jobid: i64")
        .unwrap_or_else(|| panic!("终态那一臂的起手形状变了，判据落不到字段类型上：{runner}"));
    let arm: String = runner[at..].chars().take(60).collect();
    assert!(arm.contains("ShortNote") && !arm.contains("error: String"),
        "终态那句又退回裸串（字段类型不是 ShortNote）：{arm}");
    // ② rclone 那一条生产者过分类器，而且**只**在这一处过——构造 `Failed` 的两处里另一处
    //    是自家拼的 `ShortNote`（不变量那一句），它不需要也不能再过一遍。
    assert_eq!(count_in(&runner, "TaskState::Failed {"), 2,
        "终态构造点数量变了，下面那两条按位判据就没覆盖了：{runner}");
    assert_eq!(count_in(&runner, "error: note_for_text(&status.error)"), 1,
        "rclone 的终态原话没走分类器（或直接塞了原文）：{runner}");
    // ③ 界面拿到的必须是那一份两段的，渲染走 `NoteLine`；明面不许出现 `description={state.error}`
    //    那种"把整段递上去"的写法（TS 那一臂同样是 `ShortNote`，见 ④）。
    let detail = strip_comments(&read("src/components/JobDetailPanel.tsx"));
    assert!(detail.contains("description={<NoteLine note={state.error} />}"),
        "「任务失败」下面渲染的不是分好两段的那份：{detail}");
    assert!(!detail.contains("description={state.error}"),
        "又把裸串整段递上明面：{detail}");
    // ④ TS 侧的类型也不能是 string（那边一退，界面就能自己拆句，判定回到界面里）
    let types = strip_comments(&read("src/types.ts"));
    let at = types.find("state: 'failed'")
        .unwrap_or_else(|| panic!("TS 里没有终态失败那一臂：{types}"));
    let arm: String = types[at..].chars().take(60).collect();
    assert!(arm.contains("error: ShortNote"), "TS 的终态字段不是 ShortNote：{arm}");
}


/// 起不来那一句**只许套一次外壳**（实测撞到：顶栏读作
/// 「面板没能进入可用状态：面板没能进入可用状态：jobs.toml 解析失败：…」）。
/// 病根不在那句外壳本身，在**存进 `AppState` 的已经是套过外壳的那一句**：
/// 之后每一次被问到（`AppState::runner()` 那一档）都会再套一次，操作员看到的就是两遍。
/// 所以存进去的必须是原因本身，外壳由呈现那一层加、只加一次。
#[test]
fn the_startup_reason_is_stored_bare_so_the_shell_is_never_doubled() {
    use rclone_deck_lib::config::ConfigError;
    use rclone_deck_lib::error::AppError;
    let e: AppError = ConfigError::Parse("TOML parse error at line 5".into()).into();
    let stored = e.reason();
    assert!(!stored.contains("面板没能进入可用状态"),
        "存进 AppState 的那句已经自带外壳（下一次被问到就是两遍）：{stored}");
    let shown = AppError::Startup(stored).to_string();
    assert_eq!(shown.matches("面板没能进入可用状态").count(), 1, "外壳不止一层：{shown}");
    assert!(shown.contains("TOML parse error at line 5"),
        "原因被外壳挤掉了（操作员得看得见坏在哪）：{shown}");
}
