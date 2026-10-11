//! 浏览器读侧与两道闸（spec §5.3 的前两行 + I8 + R74）。
//!
//! 这个模块住在 `runner::` 下面而不是 `src-tauri/src/` 顶层（裁定 R77）：它要用 `Runner` 的私有状态
//! （`defaults.exclude`、`rcd.client`、`jobs`），而 `runner/` 里 `checks.rs`、`tasklog.rs` 与它同级。
//! 把那些字段改 `pub(crate)` 是扩大可见性，不是解决问题。
//!
//! 读侧**不拿锁**（要拿锁的是写动作，裁定 R93），但它已经把"这一行点中的是哪棵树"定在锁用的那个
//! 坐标系上（[`browser_scope`]）：读侧说一次，写侧照它申领。两处各拼一遍"这是哪棵树"就是
//! 裁定 R90 那个洞的另一种写法。
//!
//! 本地目录那一档（左列可点、`fs` 是一个裸的本地路径）整个模块按 **F16** 的读数写：
//! 量到的只有"盘符 + 绝对路径"这一族（两种斜杠写法列出同一棵树），所以没量过的形状（UNC、
//! 相对路径、光一个盘符）不发。读数在 `tools/rc-contract-probe.mjs` §7 与
//! `docs/superpowers/evidence/build-notes.md` 的 F16 一节。

use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;

use crate::endpoint::{endpoint_of, Endpoint};
use crate::rc::types::{FsInfo, JobStatus, ListEntry, StatsView};
use crate::runner::POLL_INTERVAL;
use crate::rc::{ListError, ListShape};
use crate::rc::RcError;
use crate::runner::{Inflight, RunError, Runner};
use crate::scope::{has_dot_segment, Scope};
use crate::mutex::Busy;
use crate::preview::{PreviewIntent, StoredPreview};
use crate::runner::tasklog::{Level, TaskLog};

/// 一个可以打开的位置。
///
/// `paired_with` 是**配对的那一头**（2026-10-11 加的，联动对比视图靠它）：
/// 这一面站在某个本地目录上时，那一枚交出的是它配的远端，反过来也一样。
/// 它交的是**同一个形状的 id**（能直接喂回 `browser_list`），不是一句给人看的短名 ——
/// 短名是显示，id 是坐标，界面上拿它去列东西就必须是坐标。
/// 任务里没有这一对（临时卡、或者两朵云单独加进来的远端）时是空串。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRef {
    pub id: String,
    pub label: String,
    pub is_local: bool,
    pub paired_with: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserEntry { pub name: String, pub size: u64, pub is_dir: bool }

/// 一句人话 + 可展开的细节。定义搬到了 `crate::note`（错误话术那一层要用同一个形状），
/// 这里照旧 re-export：浏览器这一族的调用点与证人都按 `browser::ShortNote` 认它。
pub use crate::note::ShortNote;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserListing {
    pub path: String,
    pub entries: Vec<BrowserEntry>,
    /// I8：那句"这不是全部"的全部内容后台拼好（前端不许自己解释过滤器与能力位）。
    pub filtered_note: ShortNote,
    /// F6/I5：能力位翻成人话。**只解释，不禁用**。
    pub capabilities: ShortNote,
    /// R74：能不能清空整个前缀。**两道闸一次算完**（排除规则 + 站在根上），界面只读这两个字段。
    /// 给布尔而不是句子的理由不变（判定作为数据走出去），但**不能只给布尔**：只给一个"能不能"，
    /// 界面要说出"为什么不能"就只能从它反推 —— 那是把"不能只可能是因为规则"当成前提，
    /// 而根那一档同样是不能（终审抓的正是这一格：根上那颗按钮亮着，点下去才被拒）。
    pub can_clear_prefix: bool,
    /// 点不动那一句的**内容**（[`clear_gates`] 拼好；能清空时是空串）。
    pub clear_note: ShortNote,
    /// 这一份清单里那些对象的用量。**不是** `operations/size` 的读数，理由见 [`usage_of`]（F11 的
    /// "受同一批过滤器管"这一半仍然成立：F16 又量了一遍过滤后的求和与过滤后的 size 相等）。
    pub total_bytes: u64,
    pub total_count: u64,
    /// 「1517 个对象」数的是文件、行里却排着目录行 —— 那句话在骗人。这两个数由后台一起交出来，
    /// 界面说「N 个文件 · M 个文件夹」，**不许**自己数行（子项目 3 记下的缺口，⑤片收掉）。
    pub file_count: u64,
    pub dir_count: u64,
}

/// 「清空这一层」的两道闸，**一次算完**（R54 + R74 + F12）：
/// ① 排除规则非空即拒 —— `purge` 无视 `--exclude`（F11），它删的比显示的多，
///    "清单与实删相等"这回事在有空规则时根本不存在；
/// ② 站在根上即拒 —— alias→本地实测 `purge` 连目录本身一起删（F12）。
///
/// 这一档问的是**参数**，不是 Scope（裁定 R112）：`operations/purge fs=X remote=""` 清的就是
/// 整个 `X`，而 `X` 可以是 `decklocal:`（alias→本地）、可以是 `r2:img`（整个桶）、
/// 也可以是 `D:/vault/img`（一整块本地目录）。换成 Scope 的"根"判据会漏掉后两种：桶名与本地
/// 路径都被算进 `prefix`，那个谓词只看得到 `decklocal:` 这一种（同一个洞 R97 刚在锁键上填过）。
/// 所以这里的判据是"`prefix` 是不是全空"，而空段/`.`/`..` 在上一道就被拒了。
///
/// 交回 `(能不能, 不能时那一句)`：界面上那颗按钮的灰与 Tooltip 里那句都从这里来，
/// 不再由"能不能"反推"为什么"。
pub fn clear_gates(exclude: &[String], fs: &str, prefix: &str) -> (bool, ShortNote) {
    if !exclude.is_empty() {
        return (false, ShortNote {
            short: format!("有 {} 条排除规则，不能清空整层", exclude.len()),
            detail: browser_clear_refused_by_rules_refusal(exclude.len()),
        });
    }
    // `.` 与空段一样是"就是这一层自己"，purge 打上去等于打在整个 `fs` 上。点段在两个调用点
    // 都被更早的一道挡住，这里再算一次是为了**这个函数自己不说谎**：折叠之后它就是唯一的判据，
    // 前提条件不该只写在注释里等调用方自觉。
    if prefix.split('/').all(|s| s.is_empty() || s == ".") {
        // "远端 / 本地目录"这个说法由根的**形状**决定，不由调用方传一个词进来：传错了那句拒绝
        // 就指着另一侧说假话。判据是冒号前那一段像不像盘符（单个字母）—— `r2:img` 这种"远端名
        // 后面还带路径"的写法整桶也是远端，而 `D:/vault` 冒号前那个字母就是盘符。
        let head = fs.split(':').next().unwrap_or("");
        let which = if head.chars().count() == 1 { "本地目录" } else { "远端" };
        return (false, ShortNote {
            short: format!("不能清空整个{which}"),
            detail: browser_prefix_root_refusal(which),
        });
    }
    (true, ShortNote::none())
}

pub fn filtered_note(rules: &[String]) -> ShortNote {
    if rules.is_empty() { return ShortNote::none(); }
    ShortNote {
        short: format!("有 {} 条排除规则 · 看到的不是全部", rules.len()),
        detail: "被排除的对象不在列表里，而清空整个前缀会把它们一起删掉（所以那一项现在点不动）。这里能做的动作都按你点中的名字执行。".to_string(),
    }
}

/// 能力位 → 一句人话。**不许**用它禁用任何按钮（F6：`Copy=false` 而 copyfile 成功；
/// F14：`Purge=false` 而 purge 真删了整棵树）。
pub fn feature_note(info: &FsInfo) -> ShortNote {
    let mut parts: Vec<String> = Vec::new();
    if info.features.get("Copy") == Some(&false) {
        parts.push("这个后端没报服务端复制的能力，复制会走下载再上传".to_string());
    }
    if info.features.get("Purge") == Some(&false) {
        parts.push("这个后端没报整棵清空的能力（能不能清空由面板按实测形状判断）".to_string());
    }
    if info.features.get("FilterAware") == Some(&true) {
        parts.push("这个后端会按排除规则过滤列表".to_string());
    }
    if parts.is_empty() { return ShortNote::none(); }
    ShortNote {
        short: "这个后端报的能力不全，面板按实测判断".to_string(),
        detail: parts.join("；"),
    }
}

/// 同一批清单里**文件与目录各几个**。与 [`usage_of`] 一样只数已经列到的那一份，不另起一次列举。
pub fn counts_of(entries: &[BrowserEntry]) -> (u64, u64) {
    let files = entries.iter().filter(|e| !e.is_dir).count() as u64;
    (files, entries.len() as u64 - files)
}

/// 对象名 → 它所在的目录（`""` 表示就在根的下一层）。见 tests 里那条证人。
///
/// 它的生产读者是写侧那几条申领锁的路（`browser_lock_scopes` 与建目录那一处）。
/// 坐标系只有一处定义：读侧那句 `path` 回显用 [`browser_scope`]，写侧申领的那个键也必须是
/// `browser_scope(fs, parent_of(remote))` 算出来的 —— 写成 `Scope::remote(fs, parent_of(..))`
/// 会得到第二套坐标系（`r2:img#…` vs 对齐任务交出的 `r2:#img/…`，裁定 R84），两把谁也不挡谁。
/// 锁的粒度因此就是**所在目录**：每一条浏览器写动作申领的都是这一格（父子相斥由
/// `mutex::conflicts_with` 判，证人 `a_held_tree_blocks_the_tree_inside_it_but_not_a_neighbour`）。
pub fn parent_of(remote: &str) -> String {
    match remote.rsplit_once('/') { Some((p, _)) => p.to_string(), None => String::new() }
}

/// 本地目录 → 发进 rc 的那个 `fs`。**这是一次转换，不是 `to_string_lossy()` 的原样**（F16）：
/// 探针量到 `<盘符>:\…` 与 `<盘符>:/…` 两种写法列出的都是同一棵树、同一批 `Path`，
/// 而**开头带斜杠**的那一种（`/D:/x`）答的是"list 与 size 都 404、fsinfo 却回 200 并且介绍的
/// 是插进了一个别的盘符的根"——那种回答不能当读数用，所以这里只放行量过的那一族。
/// 交正斜杠那一档：与 `scope::norm` 的仓库内部规范同向，也让 `describe()` 那句回显不混分隔符。
/// `None` = 这个形状没量过（UNC `\\srv\share`、相对路径、`D:tree` 这种盘符相对写法、
/// 只有一个盘符的整块盘）⇒ 左列不发这一行。**这里不说"rclone 会拒绝它们"**：没量过就不说。
/// 浏览器那一栏读一张本地图，交成 `data:` URL（2026-10-11）。
///
/// 与 [`super::local_image_data_url`] 分成两个命令，是因为**根不一样**：
/// 那一处是"这条任务的本地根"，而这里一栏能自由导航（`prefix` 随面包屑变），
/// 且 `fs` 是 `local_fs_id` 编出来的那种字符串（`D:/pics`）。所以这里要把 `fs`
/// **反解**回 `Path` 再拼 `prefix` —— 复用同一个 `local_fs_id` 既是唯一口径，
/// 也顺带保证了"解不出来的形状（UNC 等）根本不会发到界面上"。
///
/// 三道安全闸与那一处逐条相同（理由见那儿）：相对名、规范化后仍在根下、扩展名白名单。
/// **根是这一栏当前所在的位置**——它由操作员导航到哪就封到哪，不是任务配置里那个。
pub fn preview_pane_image(fs: &str, prefix: &str, name: &str) -> Result<String, RunError> {
    let Some(root) = local_fs_id_to_path(fs) else {
        return Err(RunError::Invariant(
            "这一栏的位置是界面画不出来的形状，看不了图。".into()));
    };
    // prefix 是这一栏的导航状态，它的每一段都来自清单行的 `name`（`BrowserPanel` 的
    // `under()` 是唯一一处拼接）。但它仍然来自**界面**，所以与 `name` 一道过闸。
    let mut rel = String::new();
    if !prefix.is_empty() { rel.push_str(prefix); rel.push('/'); }
    rel.push_str(name);
    if !super::safe_relative_name(&rel) {
        return Err(RunError::Invariant(format!("「{rel}」不是一个可以读的图片名。")));
    }
    super::read_local_image_data_url(&root, &rel)
}

/// `local_fs_id` 的反函数：`D:/pics` → `Path("D:\\pics")`。**只放行那一个量过的形状**，
/// 解不出来交 `None`（UNC、盘符相对写法、整块盘——`local_fs_id` 本来就不发它们）。
fn local_fs_id_to_path(id: &str) -> Option<std::path::PathBuf> {
    let flat = id.replace('\\', "/");
    let (head, tail) = flat.split_once(':')?;
    if head.len() != 1 || !head.as_bytes()[0].is_ascii_alphabetic() || !tail.starts_with('/') {
        return None;
    }
    let rest: Vec<&str> = tail.split('/').filter(|s| !s.is_empty()).collect();
    if rest.is_empty() { return None; }
    Some(std::path::PathBuf::from(format!("{head}:\\{}", rest.join("\\"))))
}

/// 远端一栏读一张图的前若干字节（2026-10-11）——**云端预览**。
///
/// 走 `operations/cat` 并让**取内容的那一侧就截断**（`count` 被翻成 Range 头），
/// 所以一张 20 MB 的原图只下前若干 KB，不是整张拉回来再裁。
///
/// **为什么这一条与本地那两条不是同一个函数**：本地的根在磁盘上，能用 `canonicalize`
/// 规范化之后判"仍在根之下"；远端没有"规范化"这一步，能做的只有——
/// - `safe_relative_name`（点段 / 盘符 / 前导斜杠，与本地同一份判据）；
/// - 名字必须落在**这一栏当前的前缀**之下（远端那一栏也在导航，prefix 随面包屑变）；
/// - 先 `operations/stat` 问大小，**超上限连内容都不问**（省一次整文件下载）。
///
/// 少了第一条，那一条命令就是一个"能把任意远端对象读成 base64 交上屏"的出口。
pub async fn preview_remote_thumb(
    client: &crate::rc::RcClient, fs: &str, prefix: &str, name: &str, max_bytes: u64,
) -> Result<String, RunError> {
    let Some(root) = fs.strip_suffix(':') else {
        return Err(RunError::Invariant("这个位置不是远端，看不了图。".into()));
    };
    if !super::safe_relative_name(name) {
        return Err(RunError::Invariant(format!("「{name}」不是一个可以读的图片名。")));
    }
    // 这一栏当前前缀之下：远端那一栏也能自由导航，交一个跳出前缀的名字就是在读别处。
    if !prefix.is_empty() && !name.starts_with(&format!("{prefix}/")) {
        return Err(RunError::Invariant(format!("{name} 不在这一层里，不读。")));
    }
    let full = if root.is_empty() { name.to_string() } else { format!("{root}/{name}") };
    // 先问大小：超上限就**不问内容**。少这一次整文件下载是这一条最实际的省。
    if let Ok(Some(meta)) = client.operations_stat(fs, &full).await {
        if meta.size > max_bytes {
            return Err(RunError::Invariant(format!(
                "这张图 {:.1} MB，缩略图不取这么大的。",
                meta.size as f64 / 1_048_576.0)));
        }
    }
    let bytes = client.operations_cat_prefix(fs, &full, max_bytes).await
        .map_err(|e| RunError::List(e))?;
    let mime = match name.to_ascii_lowercase().rsplit('.').next().unwrap_or_default() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        _ => "application/octet-stream",
    };
    Ok(format!("data:{mime};base64,{}", super::base64_encode(&bytes)))
}

pub fn local_fs_id(path: &Path) -> Option<String> {
    let flat = path.to_string_lossy().replace('\\', "/");
    let (head, tail) = flat.split_once(':')?;
    if head.len() != 1 || !head.as_bytes()[0].is_ascii_alphabetic() || !tail.starts_with('/') {
        return None;
    }
    // 中间与尾部的空段一起压掉：同一个目录在左列只能有一行（去重见 `browser_remotes`）。
    let rest = tail.split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/");
    (!rest.is_empty()).then(|| format!("{head}:/{rest}"))
}

/// 一次递归列表 → 界面上的条目。名字用 `Path`（fs 根内的相对名）而不是 `Name`：
/// F16 的控件实测同一棵树下 `pick.webp` 与 `x/pick.webp` 的 `Name` **都是** `pick.webp`，
/// 而写动作的执行单位就是名字（I2c）——撞在一起的两个名字里，操作员点的是哪一个？
/// `Path` 同时就是写动作要发的那个 `remote`，所以这里不需要再造第二个坐标。
/// 目录在前、按名字排；大小写不敏感（`CaseInsensitive=true` 的后端上大小写交错最难读，
/// 本地那一侧就是这种）。这一条**不是** `fsinfo` 的读数：探针 §7 只量了 `Copy` / `Purge` /
/// `FilterAware` / `IsLocal` 四枚位，没量 `CaseInsensitive` —— 判据是排序行为本身，别引探针。
/// 大小写折叠后**相等**的那一对（`a.webp` 与 `A.webp`）再按名字本身比一次：只折叠就收尾的话，
/// 两者的先后退成 rclone 回来的顺序，同一份清单在两次刷新之间可以换序，操作员看到的行序就不是判据。
pub fn browser_entries(raw: Vec<ListEntry>) -> Vec<BrowserEntry> {
    let mut entries: Vec<BrowserEntry> = raw.into_iter()
        .map(|e| BrowserEntry { name: e.path, size: e.size, is_dir: e.is_dir })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir)
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name)));
    entries
}

/// 用量按**已经列到的那一份**算，不按 `operations/size`（F16）：问 `remote="sub"`、
/// 问一个根本不存在的前缀，反斜杠 / 正斜杠 / 具名 alias 三种本地 fs 形状回的都是**整棵树**的
/// 字节数与个数，而且都是 HTTP 200——没有一个错误可接，把那个数放在"这一层"的位置上就是说谎。
/// 同一份探针量到根上"递归列表求和 == `operations/size` 的读数"（13 B / 4 个文件，目录条目
/// 不进数），所以这一档在根上与那个方法是同一个量，只是往前缀里也照样说得通。
/// 云端前缀上 `operations/size` 看不看前缀**没有量过**（真桶的 dry-run 许可没批，云端零请求），
/// 所以这里不问它。
pub fn usage_of(entries: &[BrowserEntry]) -> (u64, u64) {
    entries.iter().filter(|e| !e.is_dir)
        .fold((0u64, 0u64), |(bytes, count), e| (bytes + e.size, count + 1))
}

/// 点中的那个位置是**哪一棵树**——读侧的回显与写侧的申领共用它（裁定 R93/R90）。
/// 判据只从 `Scope::of_endpoint` 那一处出：写成 `Scope::remote(fs, prefix)` 会把 `r2:img`
/// 整串当根，键就成 `r2:img#2026`，而对齐任务交出的是 `r2:#img/2026`（桶名在 `prefix` 里，
/// 裁定 R84），根不相等 ⇒ 谁也不挡谁。本地那一支落进 `Scope::local`，与任务行那句回显同一个写法。
pub fn browser_scope(fs: &str, prefix: &str) -> Scope {
    match endpoint_of(fs) {
        // `fs` 自己可能已经带了一段路径（`r2:img/deep`：配置里那种写法是合法的），
        // 那一段和点进来的前缀是同一段路径，先接起来再交给端点坐标系。
        Endpoint::Remote { fs: root, remote } => {
            let joined = if remote.is_empty() { prefix.to_string() } else { format!("{remote}/{prefix}") };
            Scope::of_endpoint(&Endpoint::Remote { fs: root, remote: joined })
        }
        Endpoint::Local(p) => Scope::of_endpoint(&Endpoint::Local(p.join(prefix))),
    }
}

/// 一次动作的一端的**回显**。坐标只有一个来源（[`browser_scope`]，裁定 R97），显示的形状只有一个出口。
///
/// 为什么不能写 `format!("{fs}{remote}")`：本地那一份 `fs` 是 [`local_fs_id`] 产出的 `D:/x/y`，
/// **结尾没有分隔符**，直接粘会把目录和名字接成 `D:/x/ya.webp` —— 一条界面上读得出、实际上不存在的
/// 路径，而 I1 要给的正是"操作员看得出自己动的是哪儿"。远端那一支（`r2:` + `img`）反过来不能插斜杠。
/// 这两种拼法 `Scope::describe()` 已经分过一次（三种根形各有证人），所以这里不再判第二次。
pub fn browser_target_label(fs: &str, remote: &str) -> String {
    browser_scope(fs, remote).describe()
}

/// 拒收点段时给操作员的那句话：说清是哪儿、为什么点不动。
/// 这句里不许出现内部名（`fs`、`prefix`、`Scope` 这类），也不许猜 rclone 会怎么处理它。
/// 公开只为证人能读它——`browser_list` 里那条"不发请求就拒"的通路要 `Runner`，本任务够不着。
/// 判据本体在 `scope::has_dot_segment`（全仓只那一处）：配置侧的 parse 与这里用的是同一条，
/// 一个名字在"能不能进键"上不许有两个答案。
pub fn dot_segment_refusal(fs: &str, prefix: &str) -> String {
    let place = if prefix.is_empty() { fs.to_string() } else { format!("{fs} {prefix}") };
    format!("这个位置的名字里有一段就是一个点（.）或是两个点（..），面板不猜它指的是哪儿：{place}")
}

/// 写动作的名字边界（裁定 R98 + R110）：每一端的 `fs` 与 `remote` 都不许含点段。
///
/// 为什么**每个动手入口**都要问一遍，而不是靠 `browser_list` 那道：读侧那道挡的是"浏览到哪儿"，
/// 而动手时交的 `remote` 是界面在一次动作里交回来的自由串（清单里那些名字只是默认值，
/// 不是唯一来源）。配置侧的 parse 也管不到它——那条管的是 `jobs.toml` 里写死的两侧。
/// 判据仍是 `scope::has_dot_segment` 那一个函数：一条名字在"能不能进键"上全仓只许有一个答案。
fn refuse_dot_segments(ends: &[(&str, &str)]) -> Result<(), RunError> {
    for (fs, remote) in ends {
        if has_dot_segment(fs) || has_dot_segment(remote) {
            return Err(RunError::Invariant(dot_segment_refusal(fs, remote)));
        }
    }
    Ok(())
}

/// `done` 这个数**只从 rclone 的 stats 取**。清单条数是"我们以为的"，两个数分开放才有核对意义（I7）。
///
/// **为什么是 `pub`**：这一档的活证人只能钉在纯函数上——动手前的重列比对保证了
/// "我们的清单"与"rclone 报的 deletes"在成功路径上**必然相等**，所以"取错了来源"这个错误
/// 在端到端用例里观察不到（`the_purge_count_comes_from_rclone_not_from_our_list` 钉的是取值）。
/// 两个函数都是对公开类型的公开取值器，不扩大任何能力。
pub fn purge_done_count(stats: &StatsView) -> usize { stats.deletes as usize }
pub fn purge_deleted_dirs(stats: &StatsView) -> usize { stats.deleted_dirs as usize }

impl Runner {
    /// 左列"你的远端"问 rclone 自己（F3），不解析 `rclone.conf`；
    /// 再加上**任务里出现过的位置**——本地目录与带桶段的远端位置都算（spec §3/§4：不新增配置）。
    ///
    /// 远端那半边不能只交远端名：站在 `r2:` 上那个动作在 rclone 里是**列桶**（ListBuckets），
    /// 而按桶发的钥匙没有这个权限。在操作员的真配置上量过（只读、各一次）：
    /// `fs="r2:"` → 500 `AccessDenied … ListBuckets`，同一把钥匙 `fs="r2:img"` → 200（1.3 s，2 条）。
    /// 也就是说"列桶根"这一档对这种钥匙根本不存在，而它是当时左列里**唯一**能选的远端行。
    /// 本地那半边一直在交任务里出现过的目录，这里把远端补上，两半才对称。
    pub async fn browser_remotes(&self) -> Result<Vec<RemoteRef>, RunError> {
        let names = self.rcd.client.config_listremotes().await?;
        // 实测：rc 回来的名字**不带冒号**，而每个 `fs` 参数都要带。这一句就是那条事实的落点。
        let mut out: Vec<RemoteRef> = names.iter()
            .map(|n| RemoteRef { id: format!("{n}:"), label: n.clone(), is_local: false, paired_with: String::new() })
            .collect();
        // 去重的键用 `Scope` 那一份判据（`browser_scope` 与锁用的是同一个）：两行指向同一棵树时
        // 只留一行，不在这里再写一份"位置怎么算相同"（大小写归一来自裁定 R84）。
        // 根那几行先入册：任务的 dst 恰好就是 `r2:` 时，留下的是 `label = r2` 那一行，
        // 而不是派生出来那条 `label = r2:`（同一个 id 两种写法，取短的那个）。
        let mut seen: BTreeSet<String> = out.iter()
            .map(|r| browser_scope(&r.id, "").key())
            .collect();
        for job in self.jobs() {
            // 先算出这一条任务**两端**的 id：联动视图要的就是"这一头 ↔ 那一头"，
            // 所以两头一次算齐，而不是两轮循环里各算各的（那样配对关系就丢了）。
            // `(是不是本地, id, 配对的那一头)`。**只有前两格是真值**，第三格在两端都算出来之后才填。
            //
            // 用 `push` 而不是 `ends[n] = …`：上面那个 `continue` 会跳过某一端（没量过的本地形状），
            // 于是**下标与长度就错开了** —— 那一端没被 push，下一次 `ends[n]` 要往更后的位置写，
            // 越界或者写到错的格上（实测 panic：index out of bounds, len 0）。`push` 自己管长度。
            let mut ends: Vec<(bool, String)> = Vec::with_capacity(2);
            for ep in [job.endpoint_src(), job.endpoint_dst()] {
                match ep {
                    Endpoint::Remote { fs, remote } => {
                        ends.push((false, if remote.is_empty() { fs.clone() } else { format!("{fs}/{remote}") }));
                    }
                    Endpoint::Local(p) => {
                        // 没量过的本地形状不进候选（`local_fs_id` 的 F16 那一段）。
                        if let Some(id) = local_fs_id(&p) { ends.push((true, id)); }
                    }
                }
            }
            // 少于两端就**不成对**，不动对面（临时卡、两侧同型那些）。硬凑一个配对是编的。
            if ends.len() != 2 { continue; }
            // 两端互相记住对方（这是"配对"这一件事的唯一算法，两处不许各写一遍）。
            let mate0 = ends[1].1.clone();
            let mate1 = ends[0].1.clone();
            for (is_local, id) in ends.into_iter() {
                let mate = if is_local { mate0.clone() } else { mate1.clone() };
                // 带点段的那一种不必在这里再筛一次：`jobs.toml` 的两侧在 parse 就被同一条判据拒了
                // （`config/jobs.rs`，裁定 R98），走到这里的 `p` 结构上已经不可能含点段。
                let key = if is_local { Scope::local(std::path::Path::new(&id)).key() }
                          else { browser_scope(&id, "").key() };
                if !seen.insert(key) { continue; }
                let label = if is_local {
                    // 本地那半边给目录名（短）；远端那半边原样就是那个 id（配置里的写法）。
                    std::path::Path::new(&id).file_name()
                        .map(|f| f.to_string_lossy().to_string())
                        .unwrap_or_else(|| id.clone())
                } else { id.clone() };
                out.push(RemoteRef { id, label, is_local, paired_with: mate });
            }
        }
        out.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
        Ok(out)
    }

    /// 列一个前缀。**只读，所以不拿锁**（写动作的锁是 Task 9 那一档，裁定 R93）。
    pub async fn browser_list(&self, fs: &str, prefix: &str) -> Result<BrowserListing, RunError> {
        // 边界先拒点段（`scope::has_dot_segment`）：一个请求都不发。`prefix` 是界面交回来的自由串
        // （配置侧的 parse 管不到它）：放行下去的话 `..` 可能被解析成另一棵树，而回显那句键写的
        // 还是这一棵——两边各自都不算假话，合起来说的不是同一件事。
        refuse_dot_segments(&[(fs, prefix)])?;
        // **单层**列举：这一面是两栏文件浏览器，进一层就该只拿一层。
        // 递归那档是集合差与盘点要的（`rc::listing`），两边口径不同是刻意的：
        // 在操作员自己的 vault 根上量过：同一层递归回 6,110 条、单层回 2 条 ——
        // 摊成"这一层"既慢（撞客户端超时），也让「删掉这一层看到的 N 个」和它实际动的范围不是一回事。
        let raw = match self.rcd.client.operations_list(fs, prefix, ListShape::Level).await {
            // F9/R64：列举的 404 = **这一侧没有东西**（一个还没上传过的子目录是正常.empty；
            // F16 量到本地 fs 上同一档也是 404）。
            Err(ListError::NotFound) => Vec::new(),
            // 500（远端名不存在）与解析失败原样抛出：把打错的桶名读成"云端什么都没有"
            // 是这一族最贵的错。
            Err(e) => return Err(e.into()),
            Ok(v) => v,
        };
        let entries = browser_entries(raw);
        // 用量按这份清单算（F16：`operations/size` 不看前缀，见 `usage_of` 的文档）。
        let (total_bytes, total_count) = usage_of(&entries);
        // 能力位只是解释，所以它**失败不阻塞列表**：取不到就留空串（F6/I5）。
        // 这一处**故意**用 `_`：能力位取不到就是不说，这里没有需要分档的回答。
        // （F16 也量到本地形状上 fsinfo 造不出"单独失败"那一档：目录不存在它回 200，
        // fs 是文件时它和 list 一起 500。所以这一臂的运行时证人只能在桩那一层，见 tests。）
        let capabilities = match self.rcd.client.operations_fsinfo(fs).await {
            Ok(info) => feature_note(&info),
            Err(_) => ShortNote::none(),
        };
        let (can_clear_prefix, clear_note) = clear_gates(&self.defaults.exclude, fs, prefix);
        let (file_count, dir_count) = counts_of(&entries);
        Ok(BrowserListing {
            path: browser_scope(fs, prefix).describe(),
            entries,
            filtered_note: filtered_note(&self.defaults.exclude),
            capabilities,
            can_clear_prefix,
            clear_note,
            total_bytes,
            total_count,
            file_count,
            dir_count,
        })
    }

    /// 每个动作一份日志：`{动作}-{时间}.log`（I7）。`TaskLog::create` 的第二个参数是文件名前缀，
    /// 所以动作名传在那里；脱敏表用 rcd 的凭据（与任务日志同一来源）。
    /// `pub` 是为了证人够得着（与 `Runner::force_release` 同一个口径：runner 内部与测试的口子），
    /// 生产侧的读者是 Task 10-13 的每一个动作。
    pub async fn browser_open_log(&self, action: &str) -> Result<TaskLog, RunError> {
        TaskLog::create(std::path::Path::new(&self.log_dir()), action, self.rcd.secrets())
            .map_err(|e| RunError::Io(e.to_string()))
    }

    /// I2c 的预检。**这不是原子的**：stat 与写入之间不是同一笔事务——`Scope` 锁挡住的是面板内的并发，
    /// 挡不住面板外的写者。这里不假装已经关死。
    /// 非 2xx 一律当错误抛出（`?` 那条路）：**"不知道有没有同名对象"绝不能放行成"没有"**
    /// （F10：rclone 对一个撞名的写入回的是 200，所以"它会不会拒绝"根本没有那一档）。
    /// `pub` 是为了证人够得着；生产侧的读者是 Task 10 的复制与 Task 13 的移动。
    pub async fn refuse_if_target_exists(&self, fs: &str, remote: &str) -> Result<(), RunError> {
        let target = browser_target_label(fs, remote);
        match self.rcd.client.operations_stat(fs, remote).await? {
            Some(obj) => Err(RunError::Invariant(browser_target_exists_refusal(
                &target, obj.size, obj.mod_time.as_deref().unwrap_or_default(),
            ))),
            None => Ok(()),
        }
    }

    /// 一次动作的各端 → **要申领的那几棵树**（裁定 R99：交 `Vec<Scope>`，不交键串）。
    /// 判据 = 点中的那个对象**所在的目录**，不是对象本身：同目录里的并发写才会互相踩，
    /// 而对象级的键挡不住"一个改名撞进来"。坐标系只有一处定义（[`browser_scope`]，裁定 R97）。
    /// 为什么不是 `Vec<String>`：锁的入口是 `acquire_scopes`（裁定 R90/R93），键串入口
    /// `acquire_all` 登记的 `scope = None` 只按等号命中，父子那一格又空了 —— 交出去的那一刻
    /// 就已经把这条锁降级。键要等真正登记/回显时再逐条 `.key()`。
    /// `pub` 是为了证人够得着；生产侧的读者是 Task 10-13 的每一次申领。
    pub fn browser_lock_scopes(&self, ends: &[(&str, &str)]) -> Vec<Scope> {
        ends.iter().map(|(fs, remote)| browser_scope(fs, parent_of(remote).as_str())).collect()
    }

    /// 下载 / 另存为 / 上传：一条 `operations/copyfile`，写之前先做存在性预检（I2c）。
    ///
    /// 目标路径**必须由调用方拼到对象级**（当前前缀 + 文件名）。把目录名交给 `dst_remote`
    /// 会被 rc 拒绝（F10：500 `is a directory not a file`），所以这里不接目录形式的目标、
    /// 也不替操作员猜名字——猜错就是往另一个对象上写。
    ///
    /// 锁交的是**两端所在的目录**（裁定 R90/R93/R99）：只登记目标端时，
    /// 一次浏览器读取改掉对齐预览的源，那份清单还会照样执行（源变了删除候选就变了）。
    pub async fn browser_copy(&self, src_fs: &str, src_remote: &str,
                              dst_fs: &str, dst_remote: &str) -> Result<RunReport, RunError> {
        // 名字边界先问（裁定 R110）：两端都是界面在一次动作里交回来的自由串。
        refuse_dot_segments(&[(src_fs, src_remote), (dst_fs, dst_remote)])?;
        let scopes = self.browser_lock_scopes(&[(src_fs, src_remote), (dst_fs, dst_remote)]);
        // 日志排在申领之前：一次被拒的动作也要留得下"谁在什么时候试过什么"（I7）。
        let log = self.browser_open_log("复制").await?;
        // 先拿锁再动数据。这一档没有凭证（copyfile 不是破坏动作），所以不存在
        // "一次失败的申领吃掉了凭证"那条路（对齐那边有，见 `run_mirror`）。
        let guard = self.mutex.acquire_scopes(&scopes, "浏览器复制").await
            .map_err(|Busy { owner }| RunError::Busy { owner })?;
        log.line(Level::Info, &format!("复制 {} -> {}", browser_target_label(src_fs, src_remote), browser_target_label(dst_fs, dst_remote)))
            .map_err(|e| RunError::Io(e.to_string()))?;
        self.refuse_if_target_exists(dst_fs, dst_remote).await?;
        self.rcd.client.operations_copyfile((src_fs, src_remote), (dst_fs, dst_remote)).await?;
        log.line(Level::Info, "已完成：1 个对象（源保留）").map_err(|e| RunError::Io(e.to_string()))?;
        // 写完作废：任何把这棵树当**任一侧**的预览都不再作数（I3 第 3 条）。
        // 作废用的就是申领的那一组树 —— 同一个坐标系、同一个来源，不在这里再拼一遍"这次动了哪些位置"。
        self.invalidate_writes(&scopes).await;
        drop(guard);
        Ok(RunReport {
            action: "复制".into(),
            target: browser_target_label(dst_fs, dst_remote),
            // 复制**不发凭证、没有清单**：那两个数与"一致"这半句都不该出现（`has_list` 说这件事）。
            // 早先这里报 `confirmed: 1`，界面上就成了「清单上 1 个，报做完 1 个，两个数一致」——
            // 而这一档从头到尾没有一份被核对过的清单。
            confirmed: 0, done: 1, agrees: true, has_list: false,
            note: "源保留，写了 1 个对象".into(),
            log_path: log.path().to_string_lossy().to_string(),
        })
    }

    /// 建一个目录。**幂等**（F14 实测：对已存在的目录再建一次还是 200），所以这里不问"在不在"，
    /// 第二次点下去也不报错。
    ///
    /// 登记的树是**它的上一层**（裁定 R111）：新建改变的是"父层里看得见什么"。子层不用另登记一条——
    /// `conflicts()` 判的是同根 + 任一方向祖先，谁要动这个新目录，撞的就是父层那一条。
    pub async fn browser_mkdir(&self, fs: &str, remote: &str) -> Result<RunReport, RunError> {
        refuse_dot_segments(&[(fs, remote)])?;
        let scopes = vec![browser_scope(fs, parent_of(remote).as_str())];
        let log = self.browser_open_log("建目录").await?;
        let guard = self.mutex.acquire_scopes(&scopes, "浏览器建目录").await
            .map_err(|Busy { owner }| RunError::Busy { owner })?;
        self.rcd.client.operations_mkdir(fs, remote).await?;
        log.line(Level::Info, &format!("建了 {}", browser_target_label(fs, remote))).map_err(|e| RunError::Io(e.to_string()))?;
        self.invalidate_writes(&scopes).await;
        drop(guard);
        Ok(RunReport {
            action: "建目录".into(), target: browser_scope(fs, remote).describe(),
            // 没有清单可核对：`has_list` 交 false、`confirmed` 留 0，界面据此不显示那半句
            // （判据是 `has_list`，不是 `confirmed == 0`，见 `RunReport` 头上那段）。
            confirmed: 0, done: 0, agrees: true, has_list: false,
            note: "目录已建好（原来就有也算成功）".into(),
            log_path: log.path().to_string_lossy().to_string(),
        })
    }

    /// 删一个**空**目录的预览。这一档以前是直接动的（点一下就删），而它是当时界面上
    /// 唯一一条不经过清单与凭证的删除路径 —— R78 那句"没有第二条直接删的路"在它身上是假的。
    /// 现在它也走同一条：预览 → 一次性凭证 → `browser_run`。
    ///
    /// 空不空由后台在这里判（递归列一遍，只看文件），**不等 rclone 动手后报 500**：
    /// 判在发凭证之前，操作员就不会拿到一张注定作废的确认。
    ///
    /// 目录不存在时**不许**读成"它是空的"：列举的 404 在别处按"这一侧没有东西"处理（F9/R64），
    /// 那是读侧的口径；写侧把"没看见"读成"是空的"就是本仓最忌的那一族假绿，所以这里响亮拒。
    pub async fn browser_preview_rmdir(&self, fs: &str, remote: &str) -> Result<BrowserPreview, RunError> {
        refuse_dot_segments(&[(fs, remote)])?;
        let inside = match self.rcd.client.operations_list(fs, remote, ListShape::Tree).await {
            Ok(v) => v,
            Err(ListError::NotFound) => return Err(RunError::Invariant(
                format!("{} 已经不在了（或者本来就没有），没什么可删的。刷新一下这一层。",
                    browser_target_label(fs, remote)))),
            Err(e) => return Err(e.into()),
        };
        let files: Vec<String> = inside.iter().filter(|e| !e.is_dir).map(|e| e.path.clone()).collect();
        if !files.is_empty() {
            return Err(RunError::Invariant(rmdir_not_empty_refusal(files.len())));
        }
        let scope = browser_scope(fs, remote);
        let scopes = vec![scope];
        self.previews.supersede(&scopes).await;
        // 清单上就一个名字：要被删掉的是**这个目录本身**。它不是装饰——确认层靠它说清
        // "你批准的是删掉哪一个"。动手前那一步的坐标来自凭证里那对 `fs` / `remote`（重列这一层
        // 判它还在不在、还空不空），**不是**拿这份名单逐条比对 —— 别说成后者。
        let doomed = vec![remote.to_string()];
        let token = self.previews.issue(StoredPreview {
            intent: PreviewIntent::Rmdir { fs: fs.to_string(), remote: remote.to_string() },
            doomed: Some(doomed.iter().cloned().collect()),
            scopes,
        }).await;
        Ok(BrowserPreview {
            token,
            action: "删空目录".into(),
            target: browser_target_label(fs, remote),
            doomed_count: 1,
            doomed,
            total_in_prefix: 1,
            doomed_ratio: "1/1".into(),
            filtered_note: filtered_note(&self.defaults.exclude),
        })
    }

    /// 动手那一步（只从 [`Self::browser_run`] 进来，凭证是唯一入口）。
    /// 拿锁之后**再判一次空**：预览与确认之间可能有别的进程往里写了东西，
    /// 而那一次要说的是"一个都没动"，不是删掉半个目录。
    async fn run_rmdir(&self, fs: &str, remote: &str) -> Result<RunReport, RunError> {
        let scopes = vec![browser_scope(fs, remote)];
        let log = self.browser_open_log("删空目录").await?;
        let guard = self.mutex.acquire_scopes(&scopes, "浏览器删空目录").await
            .map_err(|Busy { owner }| RunError::Busy { owner })?;
        match self.rcd.client.operations_list(fs, remote, ListShape::Tree).await {
            Ok(inside) => {
                let n = inside.iter().filter(|e| !e.is_dir).count();
                if n > 0 {
                    // 锁还持着就返回：guard 在这里 Drop，键放开（没有东西被动过，不存在"还在删"）。
                    return Err(RunError::Invariant(format!(
                        "{} 里面有 {} 个文件了（刚才给你看的时候是空的），一个都没动。刷新一下再决定。",
                        browser_target_label(fs, remote), n)));
                }
            }
            Err(ListError::NotFound) => return Err(RunError::Invariant(
                format!("{} 已经不在了，一个都没动。刷新一下这一层。", browser_target_label(fs, remote)))),
            Err(e) => return Err(e.into()),
        }
        // 只剩空子目录那一档 rclone 自己会拒（F14）：那句原文（带操作员自己的绝对路径）原样交出去，
        // 不翻译成猜测。
        self.rcd.client.operations_rmdir(fs, remote).await.map_err(rmdir_refusal)?;
        log.line(Level::Info, &format!("删了空目录 {}", browser_target_label(fs, remote))).map_err(|e| RunError::Io(e.to_string()))?;
        self.invalidate_writes(&scopes).await;
        drop(guard);
        Ok(RunReport {
            action: "删空目录".into(), target: browser_target_label(fs, remote),
            // 这一档现在**有**清单了（就那一个目录名），所以两个数都报，界面那句核对也出现。
            confirmed: 1, done: 1, agrees: true, has_list: true, note: "只删了那一个空目录".into(),
            log_path: log.path().to_string_lossy().to_string(),
        })
    }

    /// 清空整个前缀的**预览**。两道闸排在一切之前：排除规则非空即拒（R74/I8，F11 说 purge 不受
    /// 过滤器管——它删的比显示的多），**前缀**站在根上即拒（F12：alias→本地时 purge 连目录本身一起删；
    /// 判据只看 `prefix` 是不是全空/全是点段，见 [`clear_gates`]）。
    ///
    /// 这里**没有**比例守卫（I2b）：清单与实删等价是 `deletefile` 那条通路的事，purge 做不到，
    /// 所以它的核对靠"动手前重列一遍"，而比例那一档从来不是它的判据。
    pub async fn browser_preview_purge(&self, fs: &str, prefix: &str) -> Result<BrowserPreview, RunError> {
        refuse_dot_segments(&[(fs, prefix)])?;
        // 两道闸与列表上那一对字段**同一个来源**（[`clear_gates`]）：界面上灰着的那颗按钮与这里
        // 拒的那一次，说的是同一句话。判据的推导只写在那一个函数里，这里不复述一遍。
        let (may_clear, why_not) = clear_gates(&self.defaults.exclude, fs, prefix);
        if !may_clear {
            // 界面那侧摆的是 `short`（栏上那句点不动），这里给的是拒绝的正文（整句解释）。
            return Err(RunError::Invariant(why_not.detail));
        }
        let scope = browser_scope(fs, prefix);
        let doomed = self.prefix_files(fs, prefix).await?;
        let scopes = vec![scope];
        self.previews.supersede(&scopes).await;
        let shown = doomed.len();
        let token = self.previews.issue(StoredPreview {
            intent: PreviewIntent::Purge { fs: fs.to_string(), prefix: prefix.to_string() },
            doomed: Some(doomed.iter().cloned().collect()),
            scopes,
        }).await;
        Ok(BrowserPreview {
            token,
            action: format!("清空这一层（连里面的一起，{shown} 个文件）"),
            target: browser_scope(fs, prefix).describe(),
            doomed_count: shown,
            doomed,
            total_in_prefix: shown,
            // 照实报 100%。这道拒绝**不**看它（I2b），看的是规则数与是不是根。
            doomed_ratio: format!("{shown}/{shown}"),
            filtered_note: filtered_note(&self.defaults.exclude),
        })
    }

    /// 异步提交 + 轮询。**锁的寿命照 `transfer` 那套来**（R33）：guard 必须在 job 结束前不死，
    /// 而"读不到状态"那一路要把锁挂进 `abandoned` 而不是就地放开——
    /// 否则 rclone 还在删，浏览器已经允许对同一棵树再发起一次写。
    /// 反过来同样成立：`job/status` 确证说完之后**就没有东西还在动这棵树**，那一刻就解除持有；
    /// 下面那几步只是画数字与写日志，失败不许把键扣到重启为止（那是把一次抖动变成一次事故）。
    async fn run_purge(&self, fs: &str, prefix: &str, doomed: BTreeSet<String>)
        -> Result<RunReport, RunError> {
        let scopes = vec![browser_scope(fs, prefix)];
        let log = self.browser_open_log("清空前缀").await?;
        let guard = self.mutex.acquire_scopes(&scopes, "浏览器清空前缀").await
            .map_err(|Busy { owner }| RunError::Busy { owner })?;
        // 动手前重新列一遍，逐条比对（I2）。规则数为 0 时"看到的"就是"会删的"（F11 的反面）。
        let fresh: BTreeSet<String> = self.prefix_files(fs, prefix).await?.into_iter().collect();
        if fresh != doomed {
            return Err(RunError::Invariant(browser_list_changed_refusal("这一层", doomed.len(), fresh.len())));
        }
        let ack = self.rcd.client.operations_purge_async(fs, prefix).await?;
        log.line(Level::Info, &format!("已提交清空，jobid={}", ack.jobid))
            .map_err(|e| RunError::Io(e.to_string()))?;
        // 持锁的整批键交给 `Inflight`：它 Drop 时如果还没 disarm，就把这批键挂进 `abandoned`。
        let mut flight = Inflight::new(guard, &self.abandoned);
        let status = self.wait_for_job(ack.jobid).await?;
        // **唯一**的解除点，跟 `transfer` 同一格语义（R48 item 1）：job 说完了。
        // 绑成有名字的局部量让它活到本函数结束 —— 裸 `flight.disarm();` 是当场落地，
        // 而 `Inflight::disarm` 头上那条规矩说的正是这一种形状。
        let _released = flight.disarm();
        let stats = self.rcd.client.core_stats(&format!("job/{}", ack.jobid)).await?.view();
        let confirmed = doomed.len();
        // `done` 只从 rclone 的 stats 取（见 `purge_done_count`）：拿清单条数填这里，
        // I7 那句"是否一致"就恒真了。
        let done = purge_done_count(&stats);
        log.line(Level::Info, &format!("清单 {confirmed} 个，rclone 报删除 {done} 个（含 {} 个目录），一致={}",
                 purge_deleted_dirs(&stats), done == confirmed))
            .map_err(|e| RunError::Io(e.to_string()))?;
        if !status.success {
            return Err(RunError::Invariant(format!("清空没有成功收尾：{}", status.error)));
        }
        self.invalidate_writes(&scopes).await;
        Ok(RunReport {
            action: "清空这一层".into(), target: browser_scope(fs, prefix).describe(),
            confirmed, done, agrees: done == confirmed, has_list: true, note: String::new(),
            log_path: log.path().to_string_lossy().to_string(),
        })
    }

    /// 等到这个 job **说完**为止，再把它的答复交出去（裁定 R113）。
    ///
    /// 为什么不能只读一次：`job/status` 立刻就答，跑完之前答的是 `finished:false` +
    /// `success:false`（rclone 在 job 跑完之前就把 `success` 答成 false；那个字段**没有**
    /// `#[serde(default)]`，缺字段是解析失败不是 false）。照 `success` 判的话，
    /// "还没跑完"就被读成"跑失败了"——那与本项目咬过两次的那格同族
    ///（把"没读到"当"没有"）。`transfer` 那圈循环同一个道理，只是它还要画进度。
    ///
    /// 读不到（`poll_status` 自己已经重试过一轮）就 `?` 抛出：调用方手里的 `Inflight`
    /// 随栈 drop 把整批键挂进 `abandoned`（R33），绝不就地放开——rclone 那边可能还在删。
    async fn wait_for_job(&self, jobid: i64) -> Result<JobStatus, RunError> {
        loop {
            let status = self.poll_status(jobid).await?;
            if status.finished {
                return Ok(status);
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    /// 改名 / 移动到目录 / 跨远端移动的**预览**。破坏性（源会没），所以走凭证；
    /// 而撞名在**领凭证之前**就拒（I2c）——这一步不是保险而是必须：
    /// F10 那一档里 move 会删源而目标一字不改（同大小同 modtime，实测 10/10）。
    pub async fn browser_preview_move(&self, src_fs: &str, src_remote: &str,
                                       dst_fs: &str, dst_remote: &str) -> Result<BrowserPreview, RunError> {
        refuse_dot_segments(&[(src_fs, src_remote), (dst_fs, dst_remote)])?;
        self.refuse_if_target_exists(dst_fs, dst_remote).await?;
        // 凭证覆盖**两端**所在的目录（裁定 R90/R99）：只登记目标端的话，源那一侧被人写过一次，
        // 操作员确认的还是"这个文件从这里搬走"——而它可能已经不在了，或者内容已经不是那份。
        let scopes = self.browser_lock_scopes(&[(src_fs, src_remote), (dst_fs, dst_remote)]);
        self.previews.supersede(&scopes).await;
        let token = self.previews.issue(StoredPreview {
            intent: PreviewIntent::Move {
                src: (src_fs.to_string(), src_remote.to_string()),
                dst: (dst_fs.to_string(), dst_remote.to_string()),
            },
            // 清单是"要被搬走的那一个名字"，与对齐那份一样是 BTreeSet<String>。
            doomed: Some([src_remote.to_string()].into_iter().collect()),
            scopes,
        }).await;
        Ok(BrowserPreview {
            token,
            action: "移动这一个".into(),
            // I1：两端都写全。只写目标端的话，"从哪搬走"就靠操作员记忆了。
            target: format!("{} → {}", browser_target_label(src_fs, src_remote), browser_target_label(dst_fs, dst_remote)),
            doomed_count: 1,
            doomed: vec![src_remote.to_string()],
            total_in_prefix: 1,
            doomed_ratio: "1/1".into(),
            filtered_note: filtered_note(&self.defaults.exclude),
        })
    }

    /// 顺序照 `run_delete_visible`：拿锁 → 核对 → 动手 → 作废两端 → 放开锁。
    /// 核对的是那份凭证承诺过的**两件事**：源还在，而且目标还空着
    ///（目标存在性必须在动手前再问一次——预览与执行之间隔着操作员的一次点击与可能的刷新，
    /// 而 `move` 撞名会删源，F10 那一档）。
    async fn run_move(&self, src: (String, String), dst: (String, String), doomed: BTreeSet<String>)
        -> Result<RunReport, RunError> {
        let ends = [(src.0.as_str(), src.1.as_str()), (dst.0.as_str(), dst.1.as_str())];
        let scopes = self.browser_lock_scopes(&ends);
        let log = self.browser_open_log("移动").await?;
        let guard = self.mutex.acquire_scopes(&scopes, "浏览器移动").await
            .map_err(|Busy { owner }| RunError::Busy { owner })?;
        let (src_fs, src_remote) = (src.0.as_str(), src.1.as_str());
        let (dst_fs, dst_remote) = (dst.0.as_str(), dst.1.as_str());
        if !doomed.iter().any(|d| d == src_remote) {
            return Err(RunError::Invariant("这份凭证与当前要移动的名字不一致，所以一个都没动".to_string()));
        }
        if self.rcd.client.operations_stat(src_fs, src_remote).await?.is_none() {
            return Err(RunError::Invariant(browser_list_changed_refusal("它", doomed.len(), 0)));
        }
        self.refuse_if_target_exists(dst_fs, dst_remote).await?;
        log.line(Level::Info, &format!("移动 {} -> {}", browser_target_label(src_fs, src_remote), browser_target_label(dst_fs, dst_remote)))
            .map_err(|e| RunError::Io(e.to_string()))?;
        self.rcd.client.operations_movefile((src_fs, src_remote), (dst_fs, dst_remote)).await?;
        log.line(Level::Info, "已完成：1 个对象换了名字/位置（源不再存在）")
            .map_err(|e| RunError::Io(e.to_string()))?;
        // 作废的就是申领的那两端（同一个来源，不在这里重算一遍位置集合）。
        self.invalidate_writes(&scopes).await;
        drop(guard);
        let confirmed = doomed.len();
        Ok(RunReport {
            action: "移动".into(), target: format!("{} → {}", browser_target_label(src_fs, src_remote), browser_target_label(dst_fs, dst_remote)),
            confirmed, done: 1, agrees: confirmed == 1, has_list: true, note: String::new(),
            log_path: log.path().to_string_lossy().to_string(),
        })
    }

    /// 列文件名的公共出口（目录不算删除对象：`deletefile` 打在目录名上是 500，F10/F14）。
    /// 名字用 `Path`，它是 `operations/list` 相对前缀返回的形状（子项目 1 §3.2 量过：不能再拼一次）。
    ///
    /// 形状（`ListShape`）**不由调用点挑**，由下面两个具名包装各自定死：挑错就是"给人看的范围"与
    /// "实际动的范围"不是一回事，而这个面上最贵的一种说错话正是这一种。
    async fn listed_files(&self, fs: &str, prefix: &str, shape: ListShape) -> Result<Vec<String>, RunError> {
        let entries = match self.rcd.client.operations_list(fs, prefix, shape).await {
            Err(ListError::NotFound) => Vec::new(),   // F9/R64：这一侧没有东西
            Err(e) => return Err(e.into()),           // 500 原样响亮
            Ok(v) => v,
        };
        let mut names: Vec<String> = entries.into_iter().filter(|e| !e.is_dir).map(|e| e.path).collect();
        // 排一次：清单要能逐条核对，也要能稳定显示给人看。rc 回来的顺序不是判据的一部分，
        // 而 `BrowserPreview.doomed` 与动手前重列的那一份比的正是集合，顺序只影响读感。
        names.sort();
        Ok(names)
    }

    /// **这一层**直接看得见的文件（不含子目录里的）。「删掉这一层看到的 N 个」那两条通路用它，
    /// 浏览器显示（`browser_list`）走的是同一档 `ListShape::Level`（它自己调 `operations_list`，
    /// 没经过这个出口）—— 两边必须同档：显示用单层、比对用递归的话，
    /// 后台每次都会说"清单变了"，那是我们自己造的假拒绝，而它会教会操作员忽略拒绝。
    pub(crate) async fn layer_files(&self, fs: &str, prefix: &str) -> Result<Vec<String>, RunError> {
        self.listed_files(fs, prefix, ListShape::Level).await
    }

    /// **整个前缀**下的文件（含子目录里的）。「清空这一层」用它：`operations/purge` 删的就是整棵，
    /// 拿单层那一份当清单会把"会删多少"报小 —— 报小的那一次是真删了整棵。
    pub(crate) async fn prefix_files(&self, fs: &str, prefix: &str) -> Result<Vec<String>, RunError> {
        self.listed_files(fs, prefix, ListShape::Tree).await
    }

    /// "删掉这一层看到的 N 个"（R75）与"删这一个"。名单为空 = 当前前缀下看到的全部。
    ///
    /// 这条通路的**清单必然等于实删**：执行单位是名字（F13：按名的调用不受过滤器影响），
    /// 所以它是浏览器里唯一能做到等价核对的删除。`purge` 做不到（F11）。
    pub async fn browser_preview_delete(&self, fs: &str, prefix: &str, names: Vec<String>)
        -> Result<BrowserPreview, RunError> {
        // 名单里每一个名字都问一遍（裁定 R110）：名字是界面交回来的，不是后台生成的那一份清单。
        refuse_dot_segments(&[(fs, prefix)])?;
        refuse_dot_segments(&names.iter().map(|n| (fs, n.as_str())).collect::<Vec<_>>())?;
        let all = self.layer_files(fs, prefix).await?;
        let doomed: Vec<String> = if names.is_empty() {
            all.clone()
        } else {
            // 名单必须**全部**在看到的集合里。有人点了三个、界面上只剩两个（中间被删了一个），
            // 那就不是"照清单删"，而是"按名字造一份清单"——这一步不放行。
            for n in &names {
                if !all.iter().any(|a| a == n) {
                    return Err(RunError::Invariant(format!("清单里的 {n} 不在这次看到的东西里，所以这次不作数")));
                }
            }
            let mut picked = names.clone();
            picked.sort();
            picked
        };
        let scopes = vec![browser_scope(fs, prefix)];
        self.previews.supersede(&scopes).await;
        let shown = doomed.len();
        let total = all.len();
        let token = self.previews.issue(StoredPreview {
            intent: if names.is_empty() {
                PreviewIntent::DeleteVisible { fs: fs.to_string(), prefix: prefix.to_string() }
            } else {
                PreviewIntent::DeleteNames { fs: fs.to_string(), names: names.clone() }
            },
            doomed: Some(doomed.iter().cloned().collect()),
            scopes,
        }).await;
        Ok(BrowserPreview {
            token,
            action: format!("删掉这 {shown} 个"),
            target: browser_scope(fs, prefix).describe(),
            doomed_count: shown,
            doomed,
            total_in_prefix: total,
            doomed_ratio: format!("{shown}/{total}"),
            filtered_note: filtered_note(&self.defaults.exclude),
        })
    }

    /// **唯一的动手入口**（R78）。顺序与对齐那条完全一致：
    /// 消费凭证 → 拿锁 → 重新核对 → 执行 → 作废别人。
    /// "先消费再拿锁"是有意的（`a_run_that_cannot_get_the_lock_has_already_consumed_the_confirmation`
    /// 钉的就是它）：拿不到锁的那一次不许把授权留在桌上。
    pub async fn browser_run(&self, token: &str) -> Result<RunReport, RunError> {
        let Some(stored) = self.previews.consume(token).await else {
            return Err(RunError::Invariant("这份确认不作数了（已经用过，或者被别的动作顶掉了）".to_string()));
        };
        // 把意图**拆出来 owned** 再分发：`stored` 要整个交给动手那一步做逐条比对，
        // 而借它的字段去调用同一个 `stored` 是借还未完就移走（编译器拦得对，不是啰嗦）。
        let StoredPreview { intent, doomed, .. } = stored;
        // 浏览器这五路的删除范围**就是**那份名单，所以它必须有名单。`None` 是对齐那一路
        // 「批准的是这一趟、没人看过名单」的凭证——它发到这里就是发错了入口。
        // 这里必须**拒绝**，不许"当成空集合下发一遍"：那等于把"没人看过"读成"看过且没东西可删"，
        // 而后者是一份合法的凭证形状（`an_unchanged_pair_still_runs_when_there_is_nothing_to_delete`）。
        let Some(doomed) = doomed else {
            return Err(RunError::Invariant(
                "这份确认没带清单，而浏览器这一族要删的整份范围就是那份清单；这次不动手".to_string()));
        };
        match intent {
            PreviewIntent::DeleteVisible { fs, prefix } => self.run_delete_visible(&fs, &prefix, doomed).await,
            PreviewIntent::DeleteNames { fs, names } => self.run_delete_names(&fs, &names, doomed).await,
            PreviewIntent::Purge { fs, prefix } => self.run_purge(&fs, &prefix, doomed).await,
            PreviewIntent::Rmdir { fs, remote } => self.run_rmdir(&fs, &remote).await,
            PreviewIntent::Move { src, dst } => self.run_move(src, dst, doomed).await,
            // 到这里五种意图都有了实现者；将来加第六种时，编译器会因为这条 catch-all 不再穷尽而**不**提醒，
            // 所以下面这条兜底臂要说得出"没接上"，而不是悄悄返回一个空报告。
            other => Err(RunError::Invariant(format!("这份确认对应的动作还没接上：{other:?}"))),
        }
    }

    async fn run_delete_visible(&self, fs: &str, prefix: &str, doomed: BTreeSet<String>)
        -> Result<RunReport, RunError> {
        let scopes = vec![browser_scope(fs, prefix)];
        let log = self.browser_open_log("删除").await?;
        let guard = self.mutex.acquire_scopes(&scopes, "浏览器删除").await
            .map_err(|Busy { owner }| RunError::Busy { owner })?;
        // 动手前重新列一遍，逐条比对（I2）。这里**没有**比例守卫：按名删除的清单等于实删，
        // 100% 那一档是"删掉这一层看到的 N 个"的合法形状（I2b），照对齐那条接过来
        // 就做出一个永远点不动的按钮（`a_full_prefix_delete_by_name_is_still_executable…` 钉的就是它）。
        let fresh: BTreeSet<String> = self.layer_files(fs, prefix).await?.into_iter().collect();
        if fresh != doomed {
            return Err(RunError::Invariant(
                browser_list_changed_refusal("这个前缀", doomed.len(), fresh.len())));
        }
        let confirmed = doomed.len();
        let mut done = 0usize;
        for name in &doomed {
            self.rcd.client.operations_deletefile(fs, name).await?;
            done += 1;
        }
        log.line(Level::Info, &format!("清单 {confirmed} 个，实际删除 {done} 个，一致={}", confirmed == done))
            .map_err(|e| RunError::Io(e.to_string()))?;
        // 作废用的就是申领的那一组树（与 `browser_copy` 同一个口径，不在这里重算位置集合）。
        self.invalidate_writes(&scopes).await;
        drop(guard);
        Ok(RunReport {
            action: "删除".into(), target: browser_scope(fs, prefix).describe(),
            confirmed, done, agrees: confirmed == done, has_list: true, note: String::new(),
            log_path: log.path().to_string_lossy().to_string(),
        })
    }

    async fn run_delete_names(&self, fs: &str, names: &[String], doomed: BTreeSet<String>)
        -> Result<RunReport, RunError> {
        // 按名字删的这一档：核对的是"这些名字还在不在"（stat），不是重列整个前缀——
        // 一个对象的删除与它邻居的变化无关，重列会让一次无关的写入把授权作废。
        let ends: Vec<(&str, &str)> = names.iter().map(|n| (fs, n.as_str())).collect();
        let scopes = self.browser_lock_scopes(&ends);
        let log = self.browser_open_log("删除").await?;
        let guard = self.mutex.acquire_scopes(&scopes, "浏览器删除").await
            .map_err(|Busy { owner }| RunError::Busy { owner })?;
        for n in names {
            if !doomed.iter().any(|d| d == n) {
                return Err(RunError::Invariant("这份确认与当前名单不一致，所以一个都没动".to_string()));
            }
            if self.rcd.client.operations_stat(fs, n).await?.is_none() {
                return Err(RunError::Invariant(browser_list_changed_refusal("它", names.len(), names.len() - 1)));
            }
        }
        let mut done = 0usize;
        for n in names { self.rcd.client.operations_deletefile(fs, n).await?; done += 1; }
        log.line(Level::Info, &format!("清单 {} 个，实际删除 {done} 个，一致={}", names.len(), done == names.len()))
            .map_err(|e| RunError::Io(e.to_string()))?;
        self.invalidate_writes(&scopes).await;
        drop(guard);
        Ok(RunReport {
            // 回显走同一处坐标系：`names` 是相对那一侧根的名字，光把它们列出来读不出"在哪儿"。
            action: "删除".into(),
            target: names.iter().map(|n| browser_target_label(fs, n)).collect::<Vec<_>>().join("、"),
            confirmed: names.len(), done, agrees: done == names.len(), has_list: true,
            note: String::new(), log_path: log.path().to_string_lossy().to_string(),
        })
    }
}

/// I2c 的话术。归因要说对：这是面板在提交之前拦下的，不是 rclone 报的错
/// （它对一个撞名的写入回 200，然后按"大小 + modtime"决定跳过还是盖掉——F10）。
///
/// 第一句是人话摘要，细节跟在后面（spec §4.1-1）：时间只到分钟、字节走人类读法 ——
/// 旧版这里印的是 `2026-10-02T21:05:09.6642251+08:00` 与 `30 字节`，
/// 秒与纳秒是日志的事（DESIGN.md 的时间那条口径）。
pub fn browser_target_exists_refusal(target: &str, size: u64, mod_time: &str) -> String {
    // `mod_time` 是空串时**不说**"改于 。"——那是这一句最难看的一种残缺，而它出现的时刻
    // 恰好是信息最少的那一刻（后端没给 ModTime 那一键）。空串在这里的含义是"没拿到"，
    // 于是那一段整个不出现，而不是留下一个空的顿号。
    let when = if mod_time.trim().is_empty() {
        String::new()
    } else {
        format!(" · 改于 {}", human_time(mod_time))
    };
    format!("那里已经有一个同名文件，一个字节都没写。{target} · {}{when}。换个名字再来一次。",
        human_bytes(size))
}

/// 字节数的人类读法。口径与前端 `format.ts` 一致（1024 进制），但**这里到 GiB 为止**：
/// 前端那一份还有一档 TiB，所以 1 TiB 以上的同一堆东西，界面上会出现 `1.00 TiB` 与
/// `1024.0 GiB` 两种读法（面板量的都是"一桶封面图"那一档，够不到这条线；真够到了就一起加一档）。
/// 之所以两处各有一份：这几句话是**后台拼好**交上来的（I5），拼句子的人手里就得有单位转换器。
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    if n < 1024 { return format!("{n} B"); }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 { v /= 1024.0; i += 1; }
    // 一位小数只给大数；小数点后两位给不到 10 的那档 —— 与前端那份保持同一个取舍。
    format!("{} {}", if v < 10.0 { format!("{v:.2}") } else { format!("{v:.1}") }, UNITS[i])
}

/// rclone 那个 RFC3339 时间 → `月-日 时:分`。认不出来就**原样交出去**：
/// 编一个看不出的时间比给一串难读的真时间更坏。
pub fn human_time(iso: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(iso) {
        Ok(t) => t.format("%m-%d %H:%M").to_string(),
        Err(_) => iso.to_string(),
    }
}

/// R74/I8 的话术：唯一的判断依据是"这里显示的不是全部"，所以条数与那句"能用的通路"都要在里面。
pub fn browser_clear_refused_by_rules_refusal(rules: usize) -> String {
    format!("有 {rules} 条排除规则时不能清空整层：清空会把没显示出来的一起删掉。请先去掉排除规则，或改用「删掉这一层看到的」那颗按钮，它只动列出来的那些。")
}

/// F12 的话术：alias→本地实测 `purge` 连目录本身一起删，所以两侧的根一律拒绝。
pub fn browser_prefix_root_refusal(which: &str) -> String {
    format!("不能清空整个{which}。根上一次误点就是整个{which}。先进到要清的那一层。")
}

/// 「删空目录」在**发凭证之前**就判出非空时说的话。
/// 与 [`rmdir_refusal`] 分两句是有意的：这一句是我们自己数出来的（有几条列在这里），
/// 那一句是 rclone 动手之后回的原话。数出来的那次不给原文，因为根本没有原文。
pub fn rmdir_not_empty_refusal(files: usize) -> String {
    format!("这个目录不是空的：里面有 {files} 个文件，所以删不掉。要连里面的一起清掉，用「清空这一层（连里面的一起）」。")
}

/// F14 那一档的话术：**先把"这个目录不是空的"说明白，rclone 的原文附在后面**。
/// 原文里那个绝对路径是操作员自己的路径，可以给他看；`operations/rmdir` 这个名字是我们自己贴在
/// 方法上的内部标签，不跟着上界面（计划的全局面口径）。
///
/// 认不出"非空"的失败（目录已经不在了、连不上 rcd）原样交出去 —— 那种时候编一句"不是空的"
/// 才是说假话。这也是为什么这里认的是**载荷里的短语**而不是"rmdir 失败"这件事本身：
/// F14 实测非空与不存在都是 500，只有原文分得开它们。
pub fn rmdir_refusal(e: ListError) -> RunError {
    // 先把**我们自己贴的那层包装**剥掉：`rc 调用 operations/rmdir 失败：…` 里那半句是内部标签，
    // 认不认得出失败原因都不该上界面。剥完剩下的才是 rclone 自己的话（带操作员自己的路径）。
    let raw = match &e {
        ListError::Other(RcError::Rc { message, .. }) => message.clone(),
        ListError::Other(RcError::Http { body, .. }) => body.clone(),
        other => other.to_string(),
    };
    if raw.contains("not empty") {
        return RunError::Invariant(format!("这个目录不是空的，所以删不掉。rclone 原话：{raw}"));
    }
    RunError::Invariant(raw)
}

/// 动手前重列比对不过的话术（I2）：两个数都要说，而且**一个都没动**——
/// 说成"删了一部分"是把一次拒绝写成一次半完成的操作。
pub fn browser_list_changed_refusal(label: &str, confirmed: usize, now: usize) -> String {
    format!("{label}变了，一个都没动。刚才给你看的是 {confirmed} 个，现在是 {now} 个。要按新的清单再来一次，点「重新预览」。")
}

/// 一次动作的预览（确认层读的那一份）。判定都在后台做完，界面只读这些字段（I5）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPreview {
    pub token: String,
    /// 后台拼好的动作名（这一档交的是 `"删掉这 3 个"`；清空那一档是 `"清空这一层（连里面的一起，N 个文件）"`），
    /// 确认层的标题。屏上那颗按钮写的是「删掉这一层看到的 N 个」，两句不是一句。
    pub action: String,
    /// I1：范围必须是显式数据——完整路径，不是"当前目录"。
    pub target: String,
    /// 这份清单有几个。**由后台数**：界面拿 `doomed.length` 当计数用的是"数行数"那一族，
    /// 而这一份数组哪天为了传输截断着发，界面上那句"删这 N 个"就会悄悄变成前 20 个
    /// （确认层的显示预算正是按条截的，见 `LayerConfirm` 那个 `LIST_BUDGET`）。
    pub doomed_count: usize,
    pub doomed: Vec<String>,
    pub total_in_prefix: usize,
    /// 照实报的比例。**不拿它拒绝**（I2b）。
    pub doomed_ratio: String,
    /// 同一份过滤说明（确认层里也要说一次：人是在这里点"删"的）。
    pub filtered_note: ShortNote,
}

/// 一次动作跑完之后交出去的那一份（I7：说得出做了什么、日志在哪儿）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub action: String,
    pub target: String,
    /// **这一档有没有一份被核对过的清单**（从预览走的那几个动作有，删空目录现在也在里面；建目录与一次复制没有）。
    /// 界面按它决定「清单上 N 个 / 两个数一致」那半句显不显 —— 而不是按 `confirmed == 0` 猜：
    /// 空清单（`confirmed == 0` 且 `has_list` 为真）是**真核对过、结果是零**，那半句要说。
    pub has_list: bool,
    /// 清单上的条数（没有清单那一档恒为 0，因为那份清单不存在）。
    pub confirmed: usize,
    /// 实际完成的条数（purge 用 rclone 自己报的 deletes，见 `purge_done_count`）。
    pub done: usize,
    /// I7 要的那一句：两个数是否一致。**判定在这里做完**，界面只读这个布尔（I5）。
    ///
    /// 没有清单那一档（[`RunReport::has_list`] 为假）时它**没有意义**（没有第二个数可核对）——所以界面在那一档不许显示
    /// "N/N 一致"这半句，否则就是把"没核对过"演成"核对通过"。这一条写在这里而不是只写在前端，
    /// 是因为判定的口径归后台；渲染那一侧只读 `has_list` 决定显不显这句（不许在 TS 里比那两个数），
    /// 并由 `frontend_browser.rs` 的证人钉住。
    pub agrees: bool,
    pub note: String,
    pub log_path: String,
}
