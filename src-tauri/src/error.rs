use thiserror::Error;

use crate::note::{note_for_text, ShortNote};
use crate::rcd::RcdError;
use crate::runner::RunError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("面板没能进入可用状态：{0}")]
    Startup(String),
    #[error(transparent)]
    Run(#[from] RunError),
    #[error(transparent)]
    Rcd(#[from] RcdError),
    #[error("读写文件失败：{0}")]
    Io(String),
    /// 设置面那三条命令的错误：拒绝话术本来就是一句人话（`{0}` 直接上屏），
    /// 不套"面板没能进入可用状态"那种外壳。
    #[error(transparent)]
    Config(#[from] crate::config::edit::EditError),
}

impl AppError {
    /// 起不来那一条**存进 `AppState` 的那句原因本身** —— 不带"面板没能进入可用状态"那层外壳。
    ///
    /// 为什么不用 `to_string()` 存：`AppState::runner()` 每一次被问到都会把存着的那句
    /// 再包进 `Startup` 一次，存进来的那份如果已经带过外壳，屏幕上就是两遍
    /// （实测撞到过：`面板没能进入可用状态：面板没能进入可用状态：jobs.toml 解析失败…`）。
    /// 外壳归呈现那一层加，一处一次。
    pub fn reason(&self) -> String {
        match self {
            AppError::Startup(s) => s.clone(),
            other => other.to_string(),
        }
    }

    /// 面向操作员的**两段**话：短的那句给人看，长的那份是排障原文（一个字不少）。
    ///
    /// 判据在 `crate::note`：我们自己拼的那一族本来就是人话（整句就是摘要）；
    /// 带 rclone 原话 / 内部方法名 / HTTP 状态码的那一族按认得出的形状给一句人话，
    /// 原文进 `detail`；认不出的**不编**，摘要写"这一步没做成"。
    pub fn note(&self) -> ShortNote {
        note_for_text(&self.to_string())
    }
}

/// 交给界面的错误是**两个字段**，不是一根字符串（裁定：话术也是判定，判定作为数据走出去）。
///
/// 以前这里 `serialize_str(&self.to_string())`：界面上于是出现
/// `rc 调用 operations/list 失败：error in ListJSON: … AccessDenied …` 这种句子 ——
/// 前端那条"界面不许出现内部名词"的证人扫的是 TSX 字面量，运行时拼进来的这一族它一条也扫不到
/// （操作员报的就是它）。改成两段之后，界面只渲染 `short`，`detail` 挂在「为什么？」后面。
impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.note().serialize(s)
    }
}

impl From<crate::config::ConfigError> for AppError {
    fn from(e: crate::config::ConfigError) -> Self {
        AppError::Startup(e.to_string())
    }
}
