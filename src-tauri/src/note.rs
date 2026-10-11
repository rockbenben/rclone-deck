//! 一句人话 + 可展开的细节（[`ShortNote`]），以及**把一段错误文本分成这两段**的那一份判据。
//!
//! 为什么两段都由后台拼：界面不许自己拆句（`I5` / spec §4.1-1）——那等于让界面持有
//! "哪半句是摘要"这个判定，后台换一次措辞（多一个括号、少一个逗号）切出来的那半句就变了意思。
//!
//! 为什么要有 [`note_for_text`] 这一层：错误文本里有相当一部分是 **rclone 自己的原话**
//! （`rc 调用 operations/list 失败：… ListBuckets … AccessDenied …`），带着内部方法名、
//! HTTP 状态码与 JSON 尾巴。界面那条"不许出现内部名词"的证人扫的是 TSX 里的**字面量**，
//! 运行时从后台拼进来的这一族它一条也扫不到 —— 操作员在界面上读到的就是这种句子。
//! 所以话术也得作为数据走出去：短的那句给人看，长的那份（原文一个字不少）留给排障。
use serde::Serialize;

/// 一句人话 + 可展开的细节。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortNote {
    pub short: String,
    pub detail: String,
}

impl ShortNote {
    pub fn none() -> ShortNote { ShortNote { short: String::new(), detail: String::new() } }
    /// 只有摘要、没有更多可展开的那一档（`detail` 留空 ⇒ 界面上不出现「为什么？」）。
    pub fn plain<S: Into<String>>(short: S) -> ShortNote {
        ShortNote { short: short.into(), detail: String::new() }
    }
    /// 两段都留原文的那种：摘要与细节都在。
    pub fn with_detail<S: Into<String>, D: Into<String>>(short: S, detail: D) -> ShortNote {
        ShortNote { short: short.into(), detail: detail.into() }
    }
    /// 给证人用的"这两段里有没有这句话"。生产侧不读它：界面拿到的就是两个字段。
    pub fn contains(&self, s: &str) -> bool { self.short.contains(s) || self.detail.contains(s) }
    pub fn is_empty(&self) -> bool { self.short.is_empty() && self.detail.is_empty() }
}

/// 内部名词的清单：出现任何一个，就说明这段文本里有**不该直接给人看**的部分。
/// 与前端那条证人（`frontend_theme.rs` 的 `internal_names_stay_out_of_the_surface`）同族，
/// 但扫的是运行时真的会上界面的那一段，而不是源码字面量。
const INTERNAL_MARKERS: &[&str] = &[
    "rc 调用", "rcd", "operations/", "core/", "sync/", "job/", "config/", "copyfile",
    "HTTP", "ListBuckets", "ListJSON", "error in", "jobs.toml", "token", "pid 文件",
    "StatusCode", "RequestID", "AccessDenied", "didn't find section",
];

/// 认得出的形状 → 一句人话（原文整段留在 `detail`）。
/// 顺序有意义：先具体后笼统（`ListBuckets` 那一条要在通用的 403 之前）。
const PATTERNS: &[(&str, &str)] = &[
    ("ListBuckets", "它要列出这个账号下所有的存储位置，而这把钥匙没有这个权限。挑一个具体的位置再列。"),
    ("AccessDenied", "那边说没有权限：这把钥匙够不到这个位置。"),
    ("StatusCode: 403", "那边说没有权限：这把钥匙够不到这个位置。"),
    ("didn't find section", "没有这个位置：远端名打错了，或者配置里压根没有它。"),
    ("not empty", "这个目录不是空的，所以删不掉。"),
    ("directory not found", "那个目录已经不在了。"),
    ("object not found", "那个东西已经不在了。"),
    ("error trying to connect", "连不上后台。"),
    ("Connection refused", "连不上后台。"),
    // 真窗口量到的那一种：rcd 被停掉之后 reqwest 说的是这一句，而不是上面两种。
    // 少这一条不会出错，只会退化成通用的"这一步没做成"——那就是把已经认得的原因说成不认得。
    ("error sending request", "连不上后台。"),
    // 列举那一档的超时**单独一句**（操作员点「看差异」拿到的就是这句）。
    // 它和别的超时不是一件事：别的调用是几百毫秒的事，超了就是没连上/挂了；
    // 只有这一条是"那个位置东西太多，一次没列完"，说成"等它回话等太久了"会把人支去查网络。
    // 排在通用的"响应超时"**之前**：那份表是第一命中即返回。
    ("rcd 响应超时：operations/list", "云端那一长串没列完。"),
    ("响应超时", "等它回话等太久了，没等到。"),
    ("timeout", "等它回话等太久了，没等到。"),
];

/// 把一段已经拼好的错误文本分成"给人看的那句"与"排障要看的原文"。
///
/// 三条口径，缺一条就会说假话：
/// ① **我们自己拼的那一族本来就是人话**（"另一条任务正在写同一棵目录树：xxx"、
///    "端口 默认端口 被别的进程占着"），整句就是摘要，`detail` 留空 —— 界面上不该多出一个
///    点开来看还是同一句话的「为什么？」。
/// ② 认得出形状的那一族给一句人话，原文一个字不少地留在 `detail`（RequestID、状态码、
///    rclone 的措辞都是排障要用的）。
/// ③ **认不出的不编**：摘要写"这一步没做成"，原文全留。`rmdir_refusal` 早就是这个口径
///    ——认不出非空却编一句"不是空的"，比不给解释更坏。
pub fn note_for_text(text: &str) -> ShortNote {
    if text.is_empty() { return ShortNote::none(); }
    if !INTERNAL_MARKERS.iter().any(|m| text.contains(m)) {
        return ShortNote::plain(text.to_string());
    }
    for (needle, short) in PATTERNS {
        if text.contains(needle) {
            return ShortNote::with_detail(*short, text.to_string());
        }
    }
    ShortNote::with_detail("这一步没做成。", text.to_string())
}
