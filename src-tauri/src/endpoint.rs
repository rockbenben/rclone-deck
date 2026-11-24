//! 任务的本地或远端根。对账报告和 Scope 仍可从同一个端点读取桶坐标。
use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// 一侧的端点：本地目录，或 `scheme:桶[/路径]`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Local(PathBuf),
    /// 完整的 rclone 根，保留配置中的路径拼写（包括远端名之后的前导斜杠）。
    Remote { root: String },
}

/// Windows 盘符也长成 `X:`，靠"冒号前不止一个字符"区分：`r2:img/x` 是 remote，`D:\vault\x` 是本地。
/// 这是**唯一**一份"什么算 remote"的判据：前端要知道哪一侧是本地时读 `JobView` 上的 `src_is_local`/`dst_is_local`
/// （由 `endpoint_of` 填），不在 TypeScript 里再正则认一遍（终审 Important 2 → 裁定 R69）。
/// 同一个冒号规则在 `scope.rs` 的 `is_drive_root()` 里还判了一遍（它问的是已经切好的根：恰好一个字符加
/// 冒号是盘符，更长的是远端名）；改这里的规则必须同步改那里，否则同一个端点会被一边当远端名、另一边当盘符。
pub fn remote_split(raw: &str) -> Option<(&str, &str)> {
    // POSIX local absolute paths may contain a colon in a directory or file name.
    if raw.starts_with('/') { return None; }
    let (scheme, rest) = raw.split_once(':')?;
    (scheme.len() > 1).then_some((scheme, rest))
}

pub fn endpoint_of(raw: &str) -> Endpoint {
    if remote_split(raw).is_some() {
        Endpoint::Remote { root: raw.to_string() }
    } else {
        Endpoint::Local(PathBuf::from(raw))
    }
}

impl Endpoint {
    pub fn is_remote(&self) -> bool { matches!(self, Endpoint::Remote { .. }) }

    pub fn as_local(&self) -> Option<&Path> {
        match self { Endpoint::Local(p) => Some(p), Endpoint::Remote { .. } => None }
    }

    /// 任务读写共用这个根。rclone 的锚定过滤规则从这个根开始匹配。
    pub fn rclone_root(&self) -> Cow<'_, str> {
        match self {
            Self::Remote { root } => Cow::Borrowed(root),
            Self::Local(path) => path.to_string_lossy(),
        }
    }

    /// 旧的桶坐标，供 Scope 及对账报告的键名使用。桶内前缀不带首尾斜杠。
    pub fn remote_target(&self) -> Option<(&str, &str)> {
        match self {
            Endpoint::Remote { root } => {
                let (_, path) = remote_split(root)?;
                match path.split_once('/') {
                    Some((bucket, prefix)) => {
                        let bucket_end = root.len() - path.len() + bucket.len();
                        Some((&root[..bucket_end], prefix.trim_matches('/')))
                    }
                    None => Some((root, "")),
                }
            }
            Endpoint::Local(_) => None,
        }
    }

    /// remote 端点里"冒号之后"混进的反斜杠，原样交出让调用方**拒绝**（不改写）。
    /// 本地路径永远 `None`：那里的 `\` 是分隔符，不是分隔符问题。
    ///
    /// 返回第一个反斜杠之后的原文，桶名中出现的反斜杠也会被检测到。
    pub fn stray_backslash(&self) -> Option<&str> {
        let Endpoint::Remote { root } = self else { return None };
        root.split_once('\\').map(|(_, tail)| tail)
    }
}
