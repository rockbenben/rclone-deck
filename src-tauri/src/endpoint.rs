//! 端点模型（裁定 R60）。
//!
//! 为什么不继续用 `Job` 上那四个 helper：它们都从 **`self.dst`** 找 remote，因为它们唯一的消费者
//! 是 gate 与盘点，而那两类的 dst 恒为远端。对齐的**下载方向**把 remote 放到了 `src`——前提一反，
//! 那些函数不会报错，只安静地交出错的那一侧，而"错的那一侧"在下游就是一份删除清单。
//! 于是"怎么解析一个端点字符串"收成这里一处；`Job` 上的旧方法改成对它薄封装（语义一字不动，
//! 只把知识的归属挪对）。
use std::path::{Path, PathBuf};

/// 一侧的端点：本地目录，或 `scheme:桶[/路径]`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Local(PathBuf),
    /// `fs` 到桶名为止（`r2:img`），`remote` 是桶内路径、**不含结尾斜杠**、可以为空。
    Remote { fs: String, remote: String },
}

/// Windows 盘符也长成 `X:`，靠"冒号前不止一个字符"区分：`r2:img/x` 是 remote，`D:\vault\x` 是本地。
/// 这是**唯一**一份"什么算 remote"的判据：前端要知道哪一侧是本地时读 `JobView` 上的 `src_is_local`/`dst_is_local`
/// （由 `endpoint_of` 填），不在 TypeScript 里再正则认一遍（终审 Important 2 → 裁定 R69）。
/// 同一个冒号规则在 `scope.rs` 的 `is_drive_root()` 里还判了一遍（它问的是已经切好的根：恰好一个字符加
/// 冒号是盘符，更长的是远端名）；改这里的规则必须同步改那里，否则同一个端点会被一边当远端名、另一边当盘符。
pub fn remote_split(raw: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = raw.split_once(':')?;
    (scheme.len() > 1).then_some((scheme, rest))
}

pub fn endpoint_of(raw: &str) -> Endpoint {
    match remote_split(raw) {
        Some((scheme, rest)) => match rest.split_once('/') {
            Some((bucket, path)) => Endpoint::Remote {
                fs: format!("{scheme}:{bucket}"),
                // 结尾斜杠在这里剥：实测 `remote="imgprompt/"` 返回 200 + 空列表，
                // 于是那一侧"什么都没有"，对齐的 doomed 就成了对端的全部。
                remote: path.trim_matches('/').to_string(),
            },
            None => Endpoint::Remote { fs: format!("{scheme}:{rest}"), remote: String::new() },
        },
        None => Endpoint::Local(PathBuf::from(raw)),
    }
}

impl Endpoint {
    pub fn is_remote(&self) -> bool { matches!(self, Endpoint::Remote { .. }) }

    pub fn as_local(&self) -> Option<&Path> {
        match self { Endpoint::Local(p) => Some(p), Endpoint::Remote { .. } => None }
    }

    /// 列举用的 `(fs, remote)`。**本地端点返回 `None`——这就是"不许把本地路径当 remote"那道闸**：
    /// 拿本地路径去 `operations/list` 是真能列出本地盘的（实测），于是"远端清单"退化成
    /// "再扫一遍本地"，那是本项目要消灭的假绿（§5.1）。
    pub fn remote_target(&self) -> Option<(&str, &str)> {
        match self {
            Endpoint::Remote { fs, remote } => Some((fs, remote)),
            Endpoint::Local(_) => None,
        }
    }

    /// remote 端点里"冒号之后"混进的反斜杠，原样交出让调用方**拒绝**（不改写）。
    /// 本地路径永远 `None`：那里的 `\` 是分隔符，不是分隔符问题。
    ///
    /// 为什么不只看 `remote`：桶名与路径的切分**只认** `/`，所以 `r2:img\a\b` 里根本没有 `/`，
    /// 整串都落在 `fs` 那一段、`remote` 是空的——只查 `remote` 会让最常见的那种手滑（把本地路径
    /// 粘到冒号后面）悄悄过去，而 fs 里带反斜杠的 remote 是 rclone 根本不认识的 remote 名。
    /// 取"第一个反斜杠之后的原样文本"：宁可交出的尾巴短一截，也不改写任何一个字符。
    pub fn stray_backslash(&self) -> Option<&str> {
        match self {
            Endpoint::Remote { fs, remote } => match fs.find('\\') {
                Some(i) => Some(&fs[i + 1..]),
                None => remote.find('\\').map(|i| &remote[i + 1..]),
            },
            Endpoint::Local(_) => None,
        }
    }
}
