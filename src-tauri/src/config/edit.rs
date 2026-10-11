//! 用户那两份配置文件（`jobs.toml` 与 `rclone.conf`）的**唯一**写路径。
//! 别处也写盘——日志、pid 记录、盘点顺手落的那份名单、首次运行的出厂 `jobs.toml`——但都在那份
//! 带理由的封闭名单上：证人 `every_place_that_writes_to_disk_is_on_the_named_list` 白名单扫整棵树，
//! 名单外出现写入即红；`the_write_path_inside_edit_is_one_function_called_only_from_apply` 再钉
//! 这一面自己只有一个落盘出口。
//!
//! 三条规矩，每条都对着一个具体的事故形状：
//!
//! 1. **`rclone.conf` 是 rclone 自己的文件。**我们只改已有段里的白名单键（`type` / `provider` /
//!    `endpoint` / `region`），一次一行，不重排、不新增段、不删段。远端的名字就取这份文件自己的
//!    段名（`section_names`）：**没有第二份名单**，也就不用去问 `config/listremotes`——那是命令面上
//!    唯一被允许的那一类调用（`config/dump`/`config/get` 的响应含明文凭据，F4/I4），能不碰就不碰。
//! 2. **不引入 TOML/INI 的序列化库。**"读进来再序列化出去"会把满纸的理由注释吃掉——
//!    R58/R60/R69 的落点就在那些注释上。所以这里按**行**操作：被点名的行换掉，
//!    其余每一行连自己的行尾一起原样回到原地。
//! 3. **行尾跟着每一行走。**真机上这两份文件都是 CRLF（实测：整份 conf 全是 `\r\n`）。
//!    统一换行会让"改一行"的保存在 diff 里长成整文件重写，也会惹恼 git。
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::jobs::{Job, JobKind, JobsFile, Manifest};
use super::{AppConfig, ConfigError};
use crate::note::ShortNote;

#[derive(Debug, Error)]
pub enum EditError {
    /// 装载侧那套校验拒了新内容。**句子原样转达**，不在这里再编一遍（那会是第二份话术）。
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// 面板自己的判据拦下的（有任务在跑、要改的段有歧义、会删的名字里没有那个字……）。
    #[error("{0}")]
    Refused(String),
    #[error("读不出 {path}：{reason}")]
    Read { path: String, reason: String },
    #[error("要改的行数对不上：说定 {want} 行，改出来 {got} 行——这次一个字节都不写")]
    DiffShape { want: usize, got: usize },
    #[error("写不进去：{0}")]
    Write(String),
}

/// 一行：内容（不含行尾）+ **它自己的**行尾。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub ending: &'static str,
}

/// 切行。**只按 `\n` 切**，`\r` 归上一行的行尾——所以 CRLF 文件切出来每段 ending 都是 `"\r\n"`，
/// LF 文件是 `"\n"`，最后一行没有行尾时 ending 是 `""`。
pub fn split_lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c == '\n' {
            let ending = if cur.ends_with('\r') {
                cur.pop();
                "\r\n"
            } else {
                "\n"
            };
            out.push(Line { text: std::mem::take(&mut cur), ending });
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(Line { text: cur, ending: "" });
    }
    out
}

pub fn join_lines(lines: &[Line]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&l.text);
        out.push_str(l.ending);
    }
    out
}

/// 一段的行区间（`end` 不含）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// 这一行是不是某个键的赋值行？返回键名。**注释行不是键**（`#` 开头那行整行作废）。
pub fn line_key(text: &str) -> Option<&str> {
    let t = text.trim_start();
    if t.starts_with('#') {
        return None;
    }
    t.split_once('=').map(|(k, _)| k.trim())
}

/// 这一行是不是 `[table]` / `[[array.of.table]]` 的头？
fn is_header(text: &str) -> bool {
    let t = text.trim();
    t.starts_with('[') && t.ends_with(']')
}

/// conf 里那些段的名字：按文件里的顺序，同名只交一次，`[[…]]` 那种数组表头不算段。
///
/// 为什么名字由**这一份文件自己**报，而不是由调用方递进来：上一版拿的是浏览器那一栏的候选，
/// 而那些候选是**位置**（`fxa:a`、`fxa:b`、`fxa:c`），于是每一段都"找不到那一段"——
/// 同一屏上一栏明明列出来了，脚边却挂着"这一栏列不出来"，两句同时说（真窗口抓到过一次）。
/// 名字与段必须住在同一个来源里，"只有一个口径"这条才成立。
pub fn section_names(lines: &[Line]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        if !is_header(&l.text) {
            continue;
        }
        let t = l.text.trim();
        if t.starts_with("[[") {
            continue;
        }
        let name = t[1..t.len() - 1].trim().to_string();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// 赋值行右边那半：去掉首尾空白，剥一层配对的引号（TOML 的字面串与基本串、conf 的裸值都走这里）。
fn parse_simple_value(text: &str) -> Option<String> {
    let (_, v) = text.split_once('=')?;
    let v = v.trim();
    let unq = match (v.chars().next(), v.chars().last()) {
        (Some('\''), Some('\'')) | (Some('"'), Some('"')) if v.len() >= 2 => &v[1..v.len() - 1],
        _ => v,
    };
    Some(unq.to_string())
}

/// `[defaults]` 段：从那一行到**下一个头**（`[[job]]`）之前，或 EOF。
pub fn defaults_span(lines: &[Line]) -> Option<Span> {
    let start = lines.iter().position(|l| l.text.trim() == "[defaults]")?;
    let end = lines.iter().skip(start + 1)
        .position(|l| is_header(&l.text))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    Some(Span { start, end })
}

/// 块的头行下标（`[[job]]` 那种数组表；`header` 是精确匹配的那一行文本）。
fn header_indexes(lines: &[Line], header: &str) -> Vec<usize> {
    lines.iter().enumerate()
        .filter(|(_, l)| l.text.trim() == header)
        .map(|(i, _)| i)
        .collect()
}

/// id 那一条 `[[job]]` 块：从它自己的头行到下一个头行之前（或 EOF）。
/// 头与头**之间**的注释行落在后一块的区间里吗？不落——注释在上一块的区间尾部，
/// 所以 `deletion_plan` 必须自己把区间收在"最后一个键行"那一步，不然理由注释就跟着没了。
pub fn job_span(lines: &[Line], id: &str) -> Option<Span> {
    let heads = header_indexes(lines, "[[job]]");
    for n in 0..heads.len() {
        let start = heads[n];
        let end = heads.get(n + 1).copied().unwrap_or(lines.len());
        let mine = (start..end).any(|i| {
            line_key(&lines[i].text) == Some("id")
                && parse_simple_value(&lines[i].text).is_some_and(|v| v == id)
        });
        if mine {
            return Some(Span { start, end });
        }
    }
    None
}

/// conf 的 `[name]` 段。**同名段出现两次就拒**——"改哪一段"说不清，而这里改的是凭据的邻居。
pub fn section_span(lines: &[Line], name: &str) -> Result<Span, EditError> {
    let header = format!("[{name}]");
    let hits = header_indexes(lines, &header);
    match hits.len() {
        0 => Err(EditError::Refused(format!(
            "配置文件里没有名为 {name} 的那一段，所以这一项没有地方可写。请重新列一遍远端。"))),
        1 => {
            let start = hits[0];
            let end = lines.iter().skip(start + 1)
                .position(|l| is_header(&l.text))
                .map(|i| start + 1 + i)
                .unwrap_or(lines.len());
            Ok(Span { start, end })
        }
        n => Err(EditError::Refused(format!(
            "配置文件里「{name}」这一段出现了 {n} 次，改哪一段说不清——请先手工把重复的那一段去掉。"))),
    }
}

/// 一次编辑 = 三个动作，全带**改前**那份的行号（0 起）。
#[derive(Debug, Clone, Default)]
pub struct EditPlan {
    pub replaced: Vec<(usize, String)>,
    /// `(插在哪一行之前, 新行们)`。追加到段末尾就是"插在下一段那个头之前"。
    pub inserted: Vec<(usize, Vec<String>)>,
    pub deleted: Vec<usize>,
}

/// 一块的删除区间：从头行到**最后一个键行**（含）。块里一个键都没有时区间退化成头行本身。
/// 之后的空行与注释行**不进区间**——那正是规矩 2 要留下的东西。
pub fn deletion_plan(lines: &[Line], block: Span) -> EditPlan {
    let last_key = (block.start..block.end)
        .filter(|i| line_key(&lines[*i].text).is_some())
        .next_back()
        .unwrap_or(block.start);
    EditPlan {
        replaced: vec![],
        inserted: vec![],
        deleted: (block.start..=last_key).collect(),
    }
}

/// 方案落地。**行尾从被改那一行原来的那一份继承**；新行继承文件里最后用过的行尾。
pub fn apply_plan(lines: &[Line], plan: &EditPlan) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::with_capacity(lines.len() + plan.inserted.len() * 2);
    let replaces: std::collections::BTreeMap<usize, &String> =
        plan.replaced.iter().map(|(i, t)| (*i, t)).collect();
    let mut inserts: std::collections::BTreeMap<usize, &Vec<String>> =
        plan.inserted.iter().map(|(i, ts)| (*i, ts)).collect();
    let deletes: std::collections::BTreeSet<usize> = plan.deleted.iter().copied().collect();
    // 新行的行尾：跟着文件走。空文件（新任务块落在一份刚建好的 conf 上）才用 `\n`。
    let new_line_ending = lines.iter().rev()
        .find(|l| !l.ending.is_empty())
        .map(|l| l.ending)
        .unwrap_or("\n");

    for (i, l) in lines.iter().enumerate() {
        if let Some(news) = inserts.remove(&i) {
            for t in news {
                out.push(Line { text: t.clone(), ending: new_line_ending });
            }
        }
        if deletes.contains(&i) {
            continue;
        }
        match replaces.get(&i) {
            Some(text) => out.push(Line { text: (*text).clone(), ending: l.ending }),
            None => out.push(l.clone()),
        }
    }
    // 尾后插：index == lines.len() 那一档（新任务块追加到文件末尾走的就是这条路）。
    if let Some(tail) = inserts.remove(&lines.len()) {
        for t in tail {
            out.push(Line { text: t.clone(), ending: new_line_ending });
        }
    }
    debug_assert!(inserts.is_empty(), "apply_plan 收到了超出文件长度的插入行号");
    out
}

/// "没动的行逐字节相同"的证人判据。
///
/// 这是一次**结构性走查**，不是"数一遍不等行的个数"：块插入与删除会把后面的行整体挪位，
/// 按行号比会误报；而"把新侧按文本摘掉点名那些"也摘不干净——插入的一行空行与文件里
/// 本来就有的空行**文本相同**，按文本摘会把没动的那些一起摘掉（第一版就是这么错的）。
///
/// 所以这里按 `apply_plan` 的构造反过来走一遍 `old`：
/// - 该插的位置上必须正好是方案里那几行新内容；
/// - 被删的行在新侧必须不出现；
/// - 被替换的行必须是那段文本，而且**行尾仍归原来那一行**；
/// - 其余每一行必须与 `old` 逐字段相等（内容 + 行尾），且顺序不变；
/// - 走完 `old` 之后新侧不许还剩行（多插一行也是"没人点名的改动"）。
pub fn only_these_changed(old: &[Line], new: &[Line], plan: &EditPlan) -> Result<(), EditError> {
    let replaces: std::collections::BTreeMap<usize, &String> =
        plan.replaced.iter().map(|(i, t)| (*i, t)).collect();
    let inserts: std::collections::BTreeMap<usize, &Vec<String>> =
        plan.inserted.iter().map(|(i, ts)| (*i, ts)).collect();
    let deletes: std::collections::BTreeSet<usize> = plan.deleted.iter().copied().collect();
    let shape = || EditError::DiffShape { want: new.len(), got: 0 };

    let mut j = 0usize;
    let step = |at: &mut usize, want: &str, ending: Option<&'static str>| -> Result<(), EditError> {
        match new.get(*at) {
            Some(l) if l.text == want && ending.map_or(true, |e| l.ending == e) => { *at += 1; Ok(()) }
            _ => Err(shape()),
        }
    };
    for i in 0..old.len() {
        if let Some(news) = inserts.get(&i) {
            for t in news.iter() {
                step(&mut j, t, None)?;
            }
        }
        if deletes.contains(&i) {
            continue;
        }
        match replaces.get(&i) {
            // 被替换的那一行：文本是方案给的，行尾还是它自己原来那一份。
            Some(t) => step(&mut j, t, Some(old[i].ending))?,
            None => step(&mut j, &old[i].text, Some(old[i].ending))?,
        }
    }
    if let Some(news) = inserts.get(&old.len()) {
        for t in news.iter() {
            step(&mut j, t, None)?;
        }
    }
    if j != new.len() {
        // 新侧还留着没人点名的行：那就是多出来的改动。
        return Err(EditError::DiffShape { want: new.len(), got: j });
    }
    Ok(())
}

// ---- 读侧：这两份文件现在是什么、哪些格能从面板改 --------------------------------
//
// 这一半的全部设计只有一句话：**密钥的值在这份代码的类型里没有地方放**。
// 不是"发出去之前擦掉"，是构造它的那几个结构体不带那个字段。所以以后有人要加一项，
// 得先在这里加字段——那一步会被 `tests/config_edit.rs` 的形状证人拦下来问一遍。

/// conf 段里**允许改**的四项。这是白名单，不是黑名单：真 conf 里还有 `chunk_size`、
/// `upload_concurrency`，它们连"能不能改"这个问题都问不到（改错了 rclone 自己会拒，
/// 而面板不去猜哪些键属于哪个后端）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteKey {
    Type,
    Provider,
    Endpoint,
    Region,
}

impl RemoteKey {
    pub fn conf_name(&self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Provider => "provider",
            Self::Endpoint => "endpoint",
            Self::Region => "region",
        }
    }
    /// 界面上的说法。英文原词由 `FieldLabel` 紧跟在说法后面排成一枚等宽小片（那是操作员在自己
    /// 文件里认的那一行），句子本体是人话——`DESIGN.md` 的标点那条把"照抄的真字符"排除在文案之外。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Type => "用哪种存储",
            Self::Provider => "服务商",
            Self::Endpoint => "服务地址",
            Self::Region => "区域",
        }
    }
    /// 卡片那一章按这一列摆（`RemoteBlock` 的 `fields`）。差异行那边读的是 `plan_conf` 里手写的
    /// 同一张表 —— **两处必须同一序**，不然"这次改了哪几行"与卡片对不上。要换顺序就一起换。
    pub fn all() -> [RemoteKey; 4] {
        [Self::Type, Self::Provider, Self::Endpoint, Self::Region]
    }
}

/// 只看名字就该当成密钥的那些键。**按名字判**，不按值判（值是空的也可能是秘密）。
/// 名单是宽的一面：多算一个只是少了一项可编辑，少算一个就是把明文凭据端上界面。
pub const SECRET_KEYS: &[&str] = &[
    "access_key", "secret", "password", "passphrase", "token", "private_key", "client_id",
    "api_key", "credential", "bearer",
];

fn is_secret_key(k: &str) -> bool {
    let lower = k.to_ascii_lowercase();
    SECRET_KEYS.iter().any(|s| lower.contains(s))
}

/// 任务那一档在界面上的三种选择。**词表以 `JobKind::word()` 为准**（R56）：
/// 这里的 `copy`/`sync`/`gate` 是分支用的 token，跟"复制/让两边对齐（会删）/对账"那三个人话词不是一件事，
/// 也不跟着文案漂。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKindWire {
    Copy,
    Sync,
    Gate,
}

impl JobKindWire {
    pub fn from_kind(kind: &JobKind) -> Self {
        match kind {
            JobKind::Copy => Self::Copy,
            JobKind::Sync => Self::Sync,
            JobKind::Gate => Self::Gate,
        }
    }
    pub fn to_kind(&self) -> JobKind {
        match self {
            Self::Copy => JobKind::Copy,
            Self::Sync => JobKind::Sync,
            Self::Gate => JobKind::Gate,
        }
    }
    /// 进 TOML 的那个 token（`kind = "sync"`）。
    pub fn toml_token(&self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Sync => "sync",
            Self::Gate => "gate",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteField {
    pub key: RemoteKey,
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretState {
    /// **只有键名**。值类型上就不存在（形状判据，spec §5.2 第①条）。
    pub present: Vec<String>,
    pub note: ShortNote,
    /// 一条可复制的出路。面板不代跑它——那是操作员在自己终端上的事。
    pub command: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBlock {
    pub name: String,
    pub fields: Vec<RemoteField>,
    pub secrets: SecretState,
}

/// 一个段 → 一块远端卡片。**只读**：值留在 `lines` 里，从这里出去的一律不带密钥值。
pub fn build_remote_block(name: &str, lines: &[Line]) -> Result<RemoteBlock, EditError> {
    let span = section_span(lines, name)?;
    let mut fields = Vec::new();
    let mut present = Vec::new();
    for k in RemoteKey::all() {
        let at = (span.start..span.end).find(|i| line_key(&lines[*i].text) == Some(k.conf_name()));
        fields.push(RemoteField {
            key: k,
            label: k.label().into(),
            // 那一行不存在 = 空的（rclone 会用后端自己的默认），界面给一个空框，
            // 不说"没这一行"那种内部话。
            value: at.and_then(|i| parse_simple_value(&lines[i].text)).unwrap_or_default(),
        });
    }
    for i in span.start..span.end {
        if let Some(k) = line_key(&lines[i].text) {
            if is_secret_key(k) {
                present.push(k.to_string());
            }
        }
    }
    Ok(RemoteBlock {
        name: name.to_string(),
        fields,
        secrets: SecretState {
            present,
            note: ShortNote::plain("这几项是密钥，面板不看也不改。"),
            command: "rclone config".to_string(),
        },
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultsBlock {
    pub rclone_bin: String,
    pub rclone_bin_exists: bool,
    /// 空串 = **问不出**。不许拿正在跑的那一份后台的版本号顶它：那是两个不同的文件。
    pub rclone_bin_version: String,
    pub log_dir: String,
    pub log_dir_exists: bool,
    pub log_dir_note: ShortNote,
    pub flags: Vec<String>,
    pub exclude: Vec<String>,
}

/// 任务块里能改的那几项。`id` 在里面是**只读**的身份（裁定 5：改 id = 删了重建）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDraft {
    pub id: String,
    pub name: String,
    pub job_kind: JobKindWire,
    pub src: String,
    pub dst: String,
    pub open_dir: String,
    /// 本版只给 `localdir` 那一档（`d1` 装载侧现在就拒，`file` 不在这一面的范围里）。
    pub manifest: bool,
    /// 下面五枚**不是能改的东西**，是"这一条站哪一对位置、站这一对的哪一侧"——
    /// 与 `JobView::new` 用的是同一组函数（`pair_key_of` / `pair_slot_of` / `endpoint_of`），
    /// 所以设置面按它分出来的组，和运行那一面那张卡，是同一个坐标系。两份派生值相等由
    /// `config_edit.rs` 里那条证人钉着；分组一旦各算一份，"我改的这条"就不是"我看到的那条"。
    /// 它们不参与任何一次写盘：发出去的 `Intent` 里只有上面那几项。
    pub pair_key: String,
    pub pair_slot: String,
    pub src_is_local: bool,
    pub dst_is_local: bool,
    pub a_is_local: bool,
}

impl JobDraft {
    /// 从配置里那一条任务读出这一面要显示的那一条（含它所属的那对位置）。
    pub fn from_job(j: &Job) -> Self {
        let (src_ep, dst_ep) = (crate::endpoint::endpoint_of(&j.src), crate::endpoint::endpoint_of(&j.dst));
        let (src_is_local, dst_is_local) = (src_ep.as_local().is_some(), dst_ep.as_local().is_some());
        let slot = crate::pair::pair_slot_of(&j.src, &j.dst);
        Self {
            id: j.id.clone(), name: j.name.clone(), job_kind: JobKindWire::from_kind(&j.kind),
            src: j.src.clone(), dst: j.dst.clone(),
            open_dir: j.open_dir.clone().unwrap_or_default(),
            manifest: matches!(j.manifest, Some(Manifest::LocalDir)),
            pair_key: crate::pair::pair_key_of(&j.src, &j.dst),
            pair_slot: slot.to_string(),
            src_is_local, dst_is_local,
            a_is_local: if slot == "a" { src_is_local } else { dst_is_local },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub jobs_path: String,
    pub conf_path: String,
    /// 空 = 那一栏正常。非空时界面走「这一栏列不出来」那一档，细节进「为什么？」。
    pub conf_note: ShortNote,
    pub defaults: DefaultsBlock,
    pub remotes: Vec<RemoteBlock>,
    pub jobs: Vec<JobDraft>,
    pub running: Vec<String>,
}

/// 面板读哪两份文件、拿哪一份 rclone 去问它自己的配置在哪。
/// `conf` 是**问 rclone 自己**要的（`rclone config file`）：`config/show` 在这台机器上是 404，
/// 而按 `%APPDATA%` 猜会打丢 `RCLONE_CONFIG` 指的夹具（实测两条都吃这个变量），
/// 猜错的那一发直接写进操作员真文件上。
#[derive(Debug, Clone)]
pub struct Files {
    pub jobs: PathBuf,
    pub conf: Option<PathBuf>,
    pub rclone_bin: PathBuf,
}

/// 读一份文本。**不是 UTF-8 就拒**，不猜编码：这两份文件里出现非法字节通常意味着
/// 有人用错误的编码存过盘，此时"尽力解"会把好的一行也写坏。
pub fn read_text(path: &Path) -> Result<String, EditError> {
    let bytes = std::fs::read(path).map_err(|e| EditError::Read {
        path: path.display().to_string(), reason: e.to_string() })?;
    String::from_utf8(bytes).map_err(|e| EditError::Read {
        path: path.display().to_string(),
        reason: format!("这份文件里有不是 UTF-8 的字节（第 {} 个位置）——\
                         先手工确认它是用哪种编码存的，面板不猜编码", e.utf8_error().valid_up_to()),
    })
}

/// 拿配置里那一份 rclone **问一个只读问题**。全仓唯一在后台之外起 rclone 的地方，
/// 所以后台那两条规矩照搬：继承来的 `RCLONE_*` 先清（只留 CONFIG / CONFIG_PASS），
/// 且只问不动数据的东西（现在只有 `config file` 与 `version` 两个用途）。
pub fn ask_rclone(bin: &Path, args: &[&str]) -> Result<String, EditError> {
    let mut cmd = std::process::Command::new(bin);
    for (key, _) in std::env::vars_os() {
        if crate::rcd::rclone_env_is_scrubbed(&key.to_string_lossy()) {
            cmd.env_remove(key);
        }
    }
    let out = cmd.args(args).stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| EditError::Read {
            path: bin.display().to_string(), reason: e.to_string() })?;
    if !out.status.success() {
        return Err(EditError::Read {
            path: bin.display().to_string(),
            reason: format!("它回的不是成功（退出码 {:?}）", out.status.code()),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `Configuration file is stored at:\n<路径>\n`（钉住的 v1.75.1 实测 stdout：两行、stderr 空）。
/// **取第二行整行**，不改斜杠也不剥引号：那是 rclone 自己的拼法，改了就是第二份口径。
pub fn parse_config_file_stdout(stdout: &str) -> Option<PathBuf> {
    if !stdout.lines().next().unwrap_or("").starts_with("Configuration file is stored at") {
        return None;
    }
    let line = stdout.lines().nth(1)?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

/// `rclone version` 的第一行形如 `rclone v1.75.1`。认不出就 `None`，不猜。
pub fn parse_version_stdout(stdout: &str) -> Option<String> {
    let first = stdout.lines().next()?.trim();
    let v = first.strip_prefix("rclone v")?;
    (!v.is_empty()).then(|| v.to_string())
}

pub fn conf_path_of(bin: &Path) -> Option<PathBuf> {
    parse_config_file_stdout(&ask_rclone(bin, &["config", "file"]).ok()?)
}

/// 「打开这个目录」那一颗要开的**层** = rclone 自己报的那份 conf 的所在目录。
///
/// 三种情况交回 `None`，让上层什么都不开：空串（没问到）、只给了个文件名没有分隔符（它在哪一层
/// 没人知道），以及那一份正好挂在根上（父目录是空串 —— 把空串交给 explorer 开出来的是"文档"，
/// 那不是这一颗说的事）。尾巴上的换行不用这里刮：`conf_path_of` 那侧读 stdout 时已经逐行 trim 过。
pub fn config_folder_of(conf_path: &str) -> Option<String> {
    let (dir, _file) = conf_path.rsplit_once(['\\', '/'])?;
    (!dir.is_empty()).then(|| dir.to_string())
}

pub fn version_of(bin: &Path) -> Option<String> {
    parse_version_stdout(&ask_rclone(bin, &["version"]).ok()?)
}

/// jobs.toml 读不出来 ⇒ 整片都读不出来（不知道拿哪一份 rclone 去问路径，也不知道任务列什么）。
/// 这一条到界面上是「这一栏列不出来」+ 原文，**不**做"从坏文件里按行捞字段"的降级读：
/// 那要第二份 jobs.toml 口径。
pub fn env_from_config() -> Result<Files, EditError> {
    let jobs = AppConfig::jobs_toml_path().map_err(EditError::Config)?;
    let file = AppConfig::load_jobs().map_err(EditError::Config)?;
    let rclone_bin = PathBuf::from(&file.defaults.rclone_bin);
    let conf = conf_path_of(&rclone_bin);
    Ok(Files { jobs, conf, rclone_bin })
}

/// 三份读侧动作：`defaults` 与 `jobs` 用**装载侧那一次解析**的结果（同一份口径，
/// 界面上绝不会出现"面板能跑但这一格显示的不是它用的值"），`remotes` 从 conf 的行里读，
/// 版本那一档单独去问那一份文件本身。
///
/// 远端的名字也来自那一份文件（`section_names`）：**没有**第二份名单。以前这里是调用方递进来
/// 的一份，而递的是浏览器那一栏的候选——那些候选是**位置**（`fxa:a`），每一段都因此"找不到"，
/// 于是同一屏上远端明明列出来了、脚边却挂着"这一栏列不出来"。只住在文件里的另一个理由：
/// 只在环境变量里定义的远端在这里没有段可写，列出它等于许诺一个改不动的格子。
pub fn current(files: &Files, running: Vec<String>) -> Result<SettingsView, EditError> {
    let parsed = JobsFile::parse(&read_text(&files.jobs)?)?;

    let bin = &files.rclone_bin;
    // `log_dir` 留空 = 用默认落点（`config::AppConfig::from_defaults` 那一档）。这一面**照实
    // 显示算出来的那个绝对路径**，而不是把空串摆进输入框 —— 那一格空着，读的人会以为
    // "日志不知道放哪"，而它其实有确定答案。代价要说清：交出去的这一串同时是草稿的初值，
    // 所以**动了 defaults 里任何一项并保存，就把这一档算出来的绝对路径写回文件**（那份
    // `log_dir = ""` 的「跟随默认」从此变成一条钉死的路径）。差异行会把这一行点名出来
    // （`原来没有这一行`），确认层照那样列 —— 不是偷偷改，但也不许再写成"存回去的仍是原文"。
    let resolved = crate::config::AppConfig::from_defaults(&parsed.defaults).ok();
    let log_dir_path = resolved
        .as_ref()
        .map(|c| c.log_dir.clone())
        .unwrap_or_else(|| PathBuf::from(&parsed.defaults.log_dir));
    // 读侧**不创建、也不试写**：创建是写那一步的事（`rcd`、`runner::tasklog`、`runner::lists`
    // 各有一处 `create_dir_all`），而"写不进去"那一句由启动路径说才轮得到它——在只读界面上探针一次，
    // 等于在这一面制造第二个写文件的地方。
    let log_dir_note = if log_dir_path.is_dir() {
        ShortNote::none()
    } else {
        ShortNote::plain("这个目录还没有，面板启动时会创建它。")
    };
    let defaults = DefaultsBlock {
        rclone_bin: parsed.defaults.rclone_bin.clone(),
        rclone_bin_exists: bin.is_file(),
        rclone_bin_version: version_of(bin).unwrap_or_default(),
        log_dir: log_dir_path.display().to_string(),
        log_dir_exists: log_dir_path.is_dir(),
        log_dir_note,
        flags: parsed.defaults.flags.clone(),
        exclude: parsed.defaults.exclude.clone(),
    };

    let mut conf_note = ShortNote::none();
    let mut remotes = Vec::new();
    let conf_path = files.conf.as_deref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    match &files.conf {
        None => conf_note = ShortNote::with_detail("这一栏列不出来。",
            "问不出 rclone 的配置文件在哪（那要拿配置里那一份 rclone 去问它自己）。"),
        Some(path) => {
            let lines = match read_text(path).map(|t| split_lines(&t)) {
                Ok(lines) => Some(lines),
                Err(e) => {
                    conf_note = ShortNote::with_detail("这一栏列不出来。", e.to_string());
                    None
                }
            };
            if let Some(lines) = lines {
                let mut misses = Vec::new();
                for name in section_names(&lines) {
                    match build_remote_block(&name, &lines) {
                        Ok(block) => remotes.push(block),
                        // 名字与段出自同一份文件，还能对不上的只剩"同名段出现两次"那一种：
                        // 那一段跳过、点名是哪个名字，而不是让它带着"改哪一段说不清"上线。
                        Err(e) => misses.push(e.to_string()),
                    }
                }
                if !misses.is_empty() {
                    conf_note = ShortNote::with_detail("这一栏列不出来。", misses.join("\n"));
                }
            }
        }
    }

    // 每一条交出去时都带着后台算好的那枚配对键（与运行那一面同一个坐标系）：
    // 「任务」那一块按它分组显示，而写出去的仍然是这一条自己的那几项。
    let jobs = parsed.jobs.iter().map(JobDraft::from_job).collect();

    Ok(SettingsView {
        jobs_path: files.jobs.display().to_string(),
        conf_path,
        conf_note,
        defaults,
        remotes,
        jobs,
        running,
    })
}

// ---- 意图与"造新字节"：这一块卡片要改成什么样 ----------------------------------

/// 一次要写的东西。**一块卡片一个意图**：差异行、那句"有没有东西要改"、确认层里那份合并视图，
/// 全都要以块为单位才有意义（一个字段一个意图会把一次确认切成四次）。
///
/// `JobAdd` 与 `JobSet` 分开的理由不是排版：合成一条"有则改之无则加之"的通路，
/// 会让"我要改甲"在打错 id 时变成"多了一条新任务"——那是最安静的一种改错配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Intent {
    #[serde(rename_all = "camelCase")]
    Defaults {
        rclone_bin: String,
        log_dir: String,
        flags: Vec<String>,
        exclude: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    JobAdd {
        id: String,
        name: String,
        job_kind: JobKindWire,
        src: String,
        dst: String,
        open_dir: String,
        manifest: bool,
    },
    #[serde(rename_all = "camelCase")]
    JobSet {
        id: String,
        name: String,
        job_kind: JobKindWire,
        src: String,
        dst: String,
        open_dir: String,
        manifest: bool,
    },
    #[serde(rename_all = "camelCase")]
    RemoveJob { id: String },
    #[serde(rename_all = "camelCase")]
    Remote {
        remote: String,
        remote_type: String,
        provider: String,
        endpoint: String,
        region: String,
    },
}

impl Intent {
    /// `JobAdd` 与 `JobSet` 共用那份字段（两条通路的差别只在"块在不在"这一判据上）。
    fn job_fields(&self) -> Option<(&str, &str, &JobKindWire, &str, &str, &str, bool)> {
        match self {
            Self::JobAdd { id, name, job_kind, src, dst, open_dir, manifest }
            | Self::JobSet { id, name, job_kind, src, dst, open_dir, manifest } =>
                Some((id, name, job_kind, src, dst, open_dir, *manifest)),
            _ => None,
        }
    }

    /// 这次动的是哪一份文件（`Prepared.path` 与确认层正文第一行的来源）。
    fn target(&self) -> Target {
        match self {
            Self::Remote { .. } => Target::Conf,
            _ => Target::Jobs,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Jobs,
    Conf,
}

/// 一行差异的三种形状。判定在后台，界面只按这个渲染"（原来没有这一行）/（这次要拿走这一行）"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Changed,
    Added,
    Removed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    /// TOML / conf 里那一行的键名（`rclone_bin`、`endpoint`）：操作员在自己文件里认的就是它。
    pub key: String,
    /// 同一件事的人话说法，由后台给：差异行与确认层念的是这一份。
    /// 判据要说窄：字段**标题**那一族是前端自己写的（远端那四项经 `labelOf` 回来读这份，
    /// defaults 与任务那几项是前端字面量）。两处各有一套说法时，改词必须一起改 ——
    /// 2026-10-09 那次词表迁移就在这里漏过一回（`用哪种存储`、`哪些东西不进去`）。
    pub label: String,
    pub before: String,
    pub after: String,
    pub kind: ChangeKind,
}

/// 一份**待落盘**的方案：新字节 + 那一刻的原文 + 给人看的那几行。
/// `original` 是给 `apply` 比对的（磁盘被别人动过 ⇒ 不写）。
#[derive(Debug, Clone)]
pub struct Prepared {
    pub path: PathBuf,
    pub original: Vec<u8>,
    pub bytes: Vec<u8>,
    /// 确认层正文第一行（路径）之后那一句：任务是那一**对**位置，远端是远端名。
    pub subject: String,
    pub changes: Vec<Change>,
    pub notes: Vec<ShortNote>,
}

/// TOML 字符串。**按内容选写法**：含反斜杠的（Windows 路径）用字面串 `'…'`——那是这两份文件里
/// 现在就在用的写法，换一种引号就是在一行没动过的内容上制造一次改动；其余用基本串 `"…"`。
/// 两边都不是"猜"：判据是这一行的字节会不会因此变。
pub fn toml_string(s: &str) -> String {
    if s.contains('\\') && !s.contains('\'') {
        return format!("'{s}'");
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => {}
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn toml_array(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| toml_string(s)).collect::<Vec<_>>().join(", "))
}

/// sync 任务的名字由面板生成（spec §5.3）：「删」那个字是这一面唯一的危险标记，不该让人手抄。
/// 生成的这一串必须同时过面板自己那条判据（`the_generated_name_…` 钉的就是这件事）。
pub fn suggested_job_name(src: &str, dst: &str) -> String {
    // 说的是"哪一侧是全集"，所以点名**远端那一边**（`r2:img` 这种拼法照抄，是代码不是文案）。
    let far = [src, dst].into_iter().find(|s| crate::endpoint::endpoint_of(s).is_remote());
    match far {
        Some(fs) => format!("让两边对齐 {fs}：多余的那些会被删掉"),
        // 两侧都是本地：那种意图会被装载侧那句"恰好一侧是 remote"拒掉，
        // 这里仍要交得出一个名字，别让操作员读到"名字为空"这种误导性拒绝。
        None => "让两边对齐：多余的那些会被删掉".to_string(),
    }
}

/// 「名字里要带那个字」这句话的**唯一出处**：面板拒操作员时用它，拒名字那条证人（
/// `tests/config_edit.rs`）也复用它。两处各写一遍就是两处会各自漂。
/// （出厂那份不归它管：`tests/app_state.rs` 现在钉的是「出厂一条会删的都不发」，方向相反。）
pub fn delete_word_in_name_reason() -> String {
    "会删掉东西的那一条，名字里要有「删」这个字——那是这一面上唯一的危险标记。\
     不这么钉，四个长得很像的按钮里就会有一个偷偷删东西，而逐条确认恰恰抓不住它。".to_string()
}

/// 一个键在一段里的三种落点：改了、原来没有、这次要拿走。**值一模一样就不点名**
/// （那是一行没人动的字节）。
fn set_key(lines: &[Line], plan: &mut EditPlan, span: Span, key: &str,
           want: Option<String>, label: &str, out: &mut Vec<Change>) {
    let at = (span.start..span.end).find(|i| line_key(&lines[*i].text) == Some(key));
    match (at, want) {
        (Some(i), Some(w)) if lines[i].text != w => {
            out.push(Change { key: key.into(), label: label.into(),
                before: lines[i].text.clone(), after: w.clone(), kind: ChangeKind::Changed });
            plan.replaced.push((i, w));
        }
        (Some(_), Some(_)) => {}
        (Some(i), None) if line_key(&lines[i].text) == Some(key) => {
            out.push(Change { key: key.into(), label: label.into(),
                before: lines[i].text.clone(), after: String::new(), kind: ChangeKind::Removed });
            plan.deleted.push(i);
        }
        (None, Some(w)) => {
            out.push(Change { key: key.into(), label: label.into(),
                before: String::new(), after: w.clone(), kind: ChangeKind::Added });
            insert_at_span_end(lines, plan, span, w);
        }
        (Some(_), None) | (None, None) => {}
    }
}

/// 段内最后一个键行之后插（那才是"这一段的内容"的末尾；后面那些空行与注释是留给下一段的）。
fn span_end_insert_index(lines: &[Line], span: Span) -> usize {
    (span.start..span.end)
        .filter(|i| line_key(&lines[*i].text).is_some())
        .next_back()
        .map(|i| i + 1)
        .unwrap_or(span.start + 1)
}

/// 追加到段末尾。同一档位置上第二次插会**并进去**（`EditPlan.inserted` 按位置归并）。
fn insert_at_span_end(lines: &[Line], plan: &mut EditPlan, span: Span, text: String) {
    let at = span_end_insert_index(lines, span);
    if let Some((_, group)) = plan.inserted.iter_mut().find(|(i, _)| *i == at) {
        group.push(text);
        return;
    }
    plan.inserted.push((at, vec![text]));
}

/// 落地的公共一步：应用方案 → **装载侧校验** → "只改了这些行"断言。
/// 顺序不能换（spec §4）：断言说的是"我没多改"，它拦不住一个装载期就拒的文件；
/// 反过来让校验先过，界面上那句拒绝才是操作员要看的话。
fn finish(jobs: bool, path: PathBuf, original_text: String, old: Vec<Line>,
          plan: EditPlan, subject: String, changes: Vec<Change>, notes: Vec<ShortNote>)
    -> Result<Prepared, EditError> {
    let new = apply_plan(&old, &plan);
    let bytes = join_lines(&new).into_bytes();
    if jobs {
        let text = String::from_utf8(bytes.clone()).map_err(|_| EditError::Refused(
            "改完的字节不是 UTF-8：这次一个字节都不写。".into()))?;
        JobsFile::parse(&text)?;
    }
    only_these_changed(&old, &new, &plan)?;
    Ok(Prepared { path, original: original_text.into_bytes(), bytes, subject, changes, notes })
}

/// 造新字节。**不做拒绝（有任务在跑）也不落盘**——那是 `WriteRegistry` 的事。
/// 这一条通路不起进程：路径已经定好了（问 rclone 是读侧那一步的事）。
pub fn plan_bytes(intent: &Intent, files: &Files) -> Result<Prepared, EditError> {
    match intent.target() {
        Target::Jobs => plan_jobs(intent, files),
        Target::Conf => plan_conf(intent, files),
    }
}

fn plan_jobs(intent: &Intent, files: &Files) -> Result<Prepared, EditError> {
    let original_text = read_text(&files.jobs)?;
    // 装载侧那一份口径读现在的内容：界面上显示的值与这里比较的值必须是同一个来源。
    let current = JobsFile::parse(&original_text)?;
    let old = split_lines(&original_text);
    let mut plan = EditPlan::default();
    let mut changes = Vec::new();
    let mut notes = Vec::new();

    let subject = match intent {
        Intent::Defaults { rclone_bin, log_dir, flags, exclude } => {
            let span = defaults_span(&old).ok_or_else(|| EditError::Refused(
                "这份文件里没有 [defaults] 那一段，面板不知道往哪儿写。".into()))?;
            // 存在性只在这一项**被改动**时检查：不动它就不拿一个操作员没碰过的路径去拦他
            // （面板现在就是用那一条跑起来的；要拦的是"改成另一个"这一件事）。
            if rclone_bin != &current.defaults.rclone_bin && rclone_bin.trim().is_empty() {
                // 空的那一格单独一句：拿"那个位置没有可执行文件：。"去说一个空字符串，
                // 操作员读到的是一个他根本没指过的路径，而那件事真正的说法是"你还没填"。
                return Err(EditError::Refused(
                    "rclone 程序那一格是空的。面板要拿它起后台，得先有那一个位置。".into()));
            }
            if rclone_bin != &current.defaults.rclone_bin
                && !Path::new(rclone_bin).is_file() {
                return Err(EditError::Refused(format!(
                    "那个位置没有可执行文件：{rclone_bin}。面板要拿它起后台，\
                     路径写错就等于起不来——这台机器上有两份 rclone，靠顺序选是赌博。")));
            }
            // 两句连带各自只管自己那一档：改了 exclude 才说"看到的不是全部"；flags 那一句**只在新值
            // 非空时**说 —— 清空 flags 同样要重开才生效，那一档今天没有话术（已知缺口，别把这句读成
            // "清空不用重开"）。
            if flags != &current.defaults.flags && !flags.is_empty() {
                notes.push(ShortNote::plain(
                    "这些参数是后台启动时读的，要生效得重开面板。"));
            }
            if exclude != &current.defaults.exclude {
                notes.push(if exclude.is_empty() {
                    ShortNote::plain("排除规则清空后，「清空这一层」那颗按钮就点得动了，\
                                      它会把那一层里的东西一起删掉。")
                } else {
                    ShortNote::plain("有排除规则时，界面上看到的不是全部，\
                                      「清空这一层」也因此点不动。")
                });
            }
            let mut want = |key: &str, line: Option<String>, label: &str|
                set_key(&old, &mut plan, span, key, line, label, &mut changes);
            want("rclone_bin", Some(format!("rclone_bin = {}", toml_string(rclone_bin))),
                "rclone 程序在哪");
            want("log_dir", Some(format!("log_dir = {}", toml_string(log_dir))), "日志放哪");
            want("flags", Some(format!("flags = {}", toml_array(flags))), "每次都要带的参数");
            want("exclude", Some(format!("exclude = {}", toml_array(exclude))), "哪些东西不进去");
            "这台面板每次跑都要用的那几项".to_string()
        }

        Intent::JobAdd { .. } | Intent::JobSet { .. } => {
            let (id, name, job_kind, src, dst, open_dir, manifest) =
                intent.job_fields().expect("上面已经按 target 分过支");
            let span = job_span(&old, id);
            let adding = matches!(intent, Intent::JobAdd { .. });
            match (adding, span) {
                (true, Some(_)) => return Err(EditError::Refused(format!(
                    "已经有一条 id 是 {id} 的任务了。要改它就是「改这条」，\
                     新加一条得换一个 id——两个同名的任务，面板跑的时候会拿错那一个。"))),
                (false, None) => return Err(EditError::Refused(format!(
                    "找不到 id 为 {id} 的那条任务，所以这一次没有要改的东西。\
                     要新加一条请用「新加一条」。"))),
                _ => {}
            }
            // 新加一条必须带编号：编号是这一条的身份，落一个空的进去，界面上就有一条
            // 谁也指不回的任务（装载侧那句查重按 id 相等判：两条空的撞了一样报，只有一条空的没人报，
            // 所以这一面自己拦）。
            if adding && id.trim().is_empty() {
                return Err(EditError::Refused(
                    "新加一条要先给它一个编号。编号之后不改：要换编号就删了重建。".into()));
            }
            // 空名字 ⇒ 一律交那一份建议名，**不分档**：它自带「删」那个字，所以 sync 那一档恰好过得了
            // 下面那条判据；给 copy / 对账 留空名字也会拿到同一句"对齐"味的建议名 —— 那是建议，
            // 不是判定，操作员改得动。
            let name = if name.trim().is_empty() {
                suggested_job_name(src, dst)
            } else {
                name.to_string()
            };
            if *job_kind == JobKindWire::Sync && !name.contains("删") {
                return Err(EditError::Refused(delete_word_in_name_reason()));
            }
            let pair = match span {
                // 更新与删除核对的是**文件里现在**那一串位置；新加的用意图里的那一串。
                Some(s) => pairing(&old, s),
                None => format!("{} → {}", src, dst),
            };
            match span {
                Some(s) => {
                    let mut want = |key: &str, line: Option<String>, label: &str|
                        set_key(&old, &mut plan, s, key, line, label, &mut changes);
                    want("name", Some(format!("name = {}", toml_string(&name))), "这条叫什么名字");
                    want("kind", Some(format!("kind = {}", toml_string(job_kind.toml_token()))),
                        "这一条算哪类");
                    want("src", Some(format!("src = {}", toml_string(src))), "从哪里来");
                    want("dst", Some(format!("dst = {}", toml_string(dst))), "到哪里去");
                    want("open_dir", if open_dir.trim().is_empty() { None } else {
                        Some(format!("open_dir = {}", toml_string(open_dir))) },
                        "顺手打开哪个目录");
                    want("manifest", if manifest {
                        Some("manifest = { kind = \"localdir\" }".to_string())
                    } else { None }, "这一条要对账");
                }
                None => {
                    // 整块追加在文件末尾，前面补一个空行；每一行都是一条 Added 差异。
                    let lines = job_block_lines(id, &name, job_kind, src, dst, open_dir, manifest);
                    for (key, label, text) in lines {
                        changes.push(Change { key, label, before: String::new(),
                            after: text.clone(), kind: ChangeKind::Added });
                    }
                    let at = old.len();
                    let mut group = vec![String::new()];
                    group.extend(job_block_texts(id, &name, job_kind, src, dst, open_dir, manifest));
                    plan.inserted.push((at, group));
                }
            }
            if *job_kind == JobKindWire::Sync && !adding {
                notes.push(ShortNote::plain("这一条会删东西。它跑起来之前要先点「让两边对齐」看清那一串，\
                                             确认之后还要重列一遍逐条比对。"));
            }
            pair
        }

        Intent::RemoveJob { id } => {
            let span = job_span(&old, id).ok_or_else(|| EditError::Refused(format!(
                "找不到 id 为 {id} 的那条任务，所以这一次没有要删的东西。")))?;
            let pair = pairing(&old, span);
            // 删的是对账那一类 ⇒ 那个前缀从此没有发布保护（spec §5.3 尾段）。
            // 判据来自那块自己的 kind 行，不是任务名。
            if block_value(&old, span, "kind").as_deref() == Some("gate") {
                notes.push(ShortNote::plain(format!(
                    "这一条是对账，删掉它之后 {pair} 这一对位置就没有对账的闸门了——\
                     外部引用了那个前缀的东西，从此没人核对它还在不在。")));
            }
            plan = deletion_plan(&old, span);
            for i in span.start..span.end {
                if let Some(k) = line_key(&old[i].text) {
                    // 认不出的键（操作员手工加的、面板不认识的那一族）也照样列出来：
                    // 删一整块必然把它里面的一切一起删掉，差异表不能只报认识的那几行。
                    let label = job_key_label(k);
                    changes.push(Change { key: k.into(),
                        label: if label.is_empty() { k.into() } else { label.into() },
                        before: old[i].text.clone(), after: String::new(),
                        kind: ChangeKind::Removed });
                }
            }
            pair
        }
        Intent::Remote { .. } => unreachable!("远端那一支走 plan_conf"),
    };

    finish(true, files.jobs.clone(), original_text, old, plan, subject, changes, notes)
}

/// 那一块的 `(src, dst)` 原样配对，用作确认层正文：名字可以撞，配对才是身份。
fn pairing(lines: &[Line], span: Span) -> String {
    let src = block_value(lines, span, "src").unwrap_or_default();
    let dst = block_value(lines, span, "dst").unwrap_or_default();
    format!("{src} → {dst}")
}

/// `[[job]]` 块里那些键的人话说法。空串 = 面板不认识这一行（调用方拿原键名顶上，
/// 这样差异表不会漏掉任何一行真被删掉的内容）。
fn job_key_label(key: &str) -> &'static str {
    match key {
        "id" => "这条的编号",
        "name" => "这条叫什么名字",
        "kind" => "这一条算哪类",
        "src" => "从哪里来",
        "dst" => "到哪里去",
        "open_dir" => "顺手打开哪个目录",
        "manifest" => "这一条要对账",
        _ => "",
    }
}

/// 块里某个键的值（剥过引号的那一份）。
fn block_value(lines: &[Line], span: Span, key: &str) -> Option<String> {
    let i = (span.start..span.end).find(|i| line_key(&lines[*i].text) == Some(key))?;
    parse_simple_value(&lines[i].text)
}

/// 新块的那些行（`[[job]]` 头 + 每条键行）。返回 `(键名, 人话标签, 行文本)` 给差异行用。
fn job_block_lines(id: &str, name: &str, job_kind: &JobKindWire, src: &str, dst: &str,
                   open_dir: &str, manifest: bool)
    -> Vec<(String, String, String)> {
    let mut out = vec![
        ("id".into(), "这条的编号".into(), format!("id = {}", toml_string(id))),
        ("name".into(), "这条叫什么名字".into(), format!("name = {}", toml_string(name))),
        ("kind".into(), "这一条算哪类".into(), format!("kind = {}", toml_string(job_kind.toml_token()))),
        ("src".into(), "从哪里来".into(), format!("src = {}", toml_string(src))),
        ("dst".into(), "到哪里去".into(), format!("dst = {}", toml_string(dst))),
    ];
    if !open_dir.trim().is_empty() {
        out.push(("open_dir".into(), "顺手打开哪个目录".into(),
            format!("open_dir = {}", toml_string(open_dir))));
    }
    if manifest {
        out.push(("manifest".into(), "这一条要对账".into(),
            "manifest = { kind = \"localdir\" }".to_string()));
    }
    out
}

/// 同一份东西的行文本（新块落地用的那一条通路，与上面那份**同一个构造点**）。
fn job_block_texts(id: &str, name: &str, job_kind: &JobKindWire, src: &str, dst: &str,
                   open_dir: &str, manifest: bool) -> Vec<String> {
    let mut v = vec!["[[job]]".to_string()];
    v.extend(job_block_lines(id, name, job_kind, src, dst, open_dir, manifest)
        .into_iter().map(|(_, _, text)| text));
    v
}

/// conf 那一支：**只改已有段里的白名单键**，一次一行。
/// 这里没有"装载侧"可以复用的校验（`rclone.conf` 不是 TOML），所以形状判据自己钉死三条：
/// 段找得到、值里不许有换行（那是第二行，不是一行的值）、键只可能是那四个（按构造）。
fn plan_conf(intent: &Intent, files: &Files) -> Result<Prepared, EditError> {
    let Intent::Remote { remote, remote_type, provider, endpoint, region } = intent else {
        unreachable!("上面按 target 分过支：非远端意图不会走到这一支");
    };
    let path = files.conf.clone().ok_or_else(|| EditError::Refused(
        "问不出 rclone 的配置文件在哪（那要拿配置里那一份 rclone 去问它自己），\
         所以这一项没有地方可写。".into()))?;
    let original_text = read_text(&path)?;
    let old = split_lines(&original_text);
    let span = section_span(&old, remote)?;
    let mut plan = EditPlan::default();
    let mut changes = Vec::new();
    let mut notes = Vec::new();
    let wanted = [
        (RemoteKey::Type, remote_type),
        (RemoteKey::Provider, provider),
        (RemoteKey::Endpoint, endpoint),
        (RemoteKey::Region, region),
    ];
    for (key, value) in wanted {
        let value = value.trim();
        if value.chars().any(|c| c == '\n' || c == '\r') {
            return Err(EditError::Refused(format!(
                "{} 这一项的值里不能有换行：一行只有一个值。", key.conf_name())));
        }
        if key == RemoteKey::Type && value.is_empty() {
            // 没有 type 的那一段 rclone 自己也不认，所以这一项不许被清空。
            return Err(EditError::Refused(
                "「用哪种存储」不能是空的：那一段会连是什么存储都说不清。".into()));
        }
        let want = if value.is_empty() { None } else {
            Some(format!("{} = {value}", key.conf_name()))
        };
        let was_type_change = key == RemoteKey::Type
            && block_value(&old, span, key.conf_name()).as_deref() != Some(value);
        set_key(&old, &mut plan, span, key.conf_name(), want, key.label(), &mut changes);
        if was_type_change {
            notes.push(ShortNote::plain("换一种存储等于换一套说法：那几项密钥与新的存储大概对不上，\
                                         要用 rclone 自己的命令重新配一遍。"));
        }
    }
    finish(false, path, original_text, old, plan, remote.clone(), changes, notes)
}

// ---- 凭证与落盘：看过差异之后才写得出去 ------------------------------------------

/// 一份已看过、待落盘的写。`original` 与 `bytes` 都存下来：`apply` 时重新算一遍，
/// 两份都要跟这里对上，对不上就不写（spec §4 后半段）。
#[derive(Debug, Clone)]
pub struct Pending {
    pub intent: Intent,
    pub path: PathBuf,
    pub original: Vec<u8>,
    pub bytes: Vec<u8>,
    pub subject: String,
    pub changes: Vec<Change>,
    pub notes: Vec<ShortNote>,
}

/// 确认层要渲染的那一份：路径是正文第一行，`subject` 是第二行（任务那一档是那**对**位置）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    pub token: String,
    pub path: String,
    pub subject: String,
    pub changes: Vec<Change>,
    pub notes: Vec<ShortNote>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub path: String,
    /// 短槽那句是「重开面板后生效。」——不热重载这件事在这里第一次说出口，
    /// 也在 Task 8 那一格被真窗口验一次（重开之后卡上才是新值）。
    pub note: ShortNote,
}

#[derive(Default)]
/// 配置写的凭证表。与对齐/浏览器那张表（`preview::PreviewRegistry`）**同一套一次性凭证的形状**，
/// 但**不是同一张表**：那张表的 `scopes` 非空是硬断言（空 scopes 的凭证作废不掉），
/// 而一次配置写没有"关于哪两棵树"可填——拿假 Scope 骗守卫比分开两张表更坏。
/// 共用的是 `preview::new_token()` 那一份生成器（凭证算法全仓只许有一份）。
pub struct WriteRegistry {
    inner: tokio::sync::Mutex<std::collections::BTreeMap<String, Pending>>,
}

impl WriteRegistry {
    /// 要差异：①有没有事正在跑 → ②造新字节 + 装载侧校验 → ③"只改了这些行"，然后登记凭证。
    /// **差异为空就不发凭证**：界面上没有可点的保存，那一档的句子也由这里给（不另编第二份）。
    pub async fn plan(&self, intent: &Intent, files: &Files,
                      running: &[String]) -> Result<PlanView, EditError> {
        refuse_while_running(running)?;
        let p = plan_bytes(intent, files)?;
        if p.changes.is_empty() {
            return Err(EditError::Refused(
                "这一项现在就是你要的值，没有要改的东西。".into()));
        }
        let token = crate::preview::new_token();
        self.inner.lock().await.insert(token.clone(), Pending {
            intent: intent.clone(), path: p.path.clone(), original: p.original.clone(),
            bytes: p.bytes.clone(), subject: p.subject.clone(),
            changes: p.changes.clone(), notes: p.notes.clone(),
        });
        Ok(PlanView { token, path: p.path.display().to_string(),
                      subject: p.subject, changes: p.changes, notes: p.notes })
    }

    /// 凭证换落盘：**重新算一遍**，与凭证里那份逐字节比对，对不上就不写。
    /// 比对的是两件事：那一刻的原文（磁盘被别人动过）与算出来的新字节（意图现在落不出同一份东西）。
    pub async fn apply(&self, token: &str, files: &Files,
                       running: &[String]) -> Result<Applied, EditError> {
        refuse_while_running(running)?;
        let pending = self.inner.lock().await.remove(token).ok_or_else(|| EditError::Refused(
            "这份确认不作数了（用过一次，或者面板重启过）。要保存就重新看一遍要改哪几行。".into()))?;
        let now = plan_bytes(&pending.intent, files)?;
        if now.original != pending.original || now.bytes != pending.bytes {
            return Err(EditError::Refused(
                "刚才看的那份和磁盘上的现在对不上——这两份文件在你看过之后被改过\
                 （可能是你自己在编辑器里存了一份回来）。这次一个字节都没写，\
                 请重新看一遍要改哪几行。".into()));
        }
        write_atomically(&now.path, &now.bytes)?;
        Ok(Applied { path: now.path.display().to_string(),
                     note: ShortNote::with_detail("重开面板后生效。", format!(
                         "写在 {}。面板只在启动那一次读它，所以这一面显示的还是刚才那一份；\
                         重启面板就换成新的。改远端那几项还要 rclone 自己重读它的配置，\
                         它本来也是启动时读的。", now.path.display())) })
    }
}

/// ①：有任务在跑就不许改配置。
///
/// 这一档排在算差异**之前**：操作员此刻要读的是"等它跑完"，不是一份注定写不进去的差异表。
/// 热重载不做（spec §2 决定 3）——边跑边改是最坏那一档：预览清单与锁都还按旧配对算，
/// 下一次删的就是旧那两个位置。
fn refuse_while_running(running: &[String]) -> Result<(), EditError> {
    if running.is_empty() {
        return Ok(());
    }
    Err(EditError::Refused(format!(
        "现在有事正在跑：{}。这些配置写完要重开面板才生效，所以请先等它跑完再改。",
        running.join("、"))))
}

/// 同目录临时文件 + rename（Windows 上 `rename` 覆盖已存在目标是它的既定语义）。
///
/// 临时文件必须与目标同目录：跨卷的 rename 不是原子替换，而那正是"改一半"的来路。
/// 清理失败**不许静默**——那个文件名进错误话术，因为操作员得知道目录里多了一个什么。
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), EditError> {
    let tmp = path.with_file_name(format!(
        "{}.rclone-deck-tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("config")));
    let attempt = (|| -> Result<(), std::io::Error> {
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    match attempt {
        Ok(()) => Ok(()),
        Err(e) => {
            let leftover = match std::fs::remove_file(&tmp) {
                Ok(()) => String::new(),
                Err(r) => format!("；临时文件 {} 也没删掉，请手动删除（{r}）", tmp.display()),
            };
            Err(EditError::Write(format!("{}：{e}{leftover}", path.display())))
        }
    }
}
