//! 盘点顺手落的那一份**名单快照**：给操作员隔两周 diff 一下用的，不参与任何判定。
//!
//! 为什么是"顺手落"而不是一颗按钮：数据就在 `run_inventory` 手上（两半名字都是刚列完算出来的），
//! 再加一条前端交名字的通路只会多一个不受控的写入口；而"按一次才留一份"意味着忘了按就没有历史。
//!
//! 为什么不推翻 `runner/mod.rs` 里那条"待传那一半只记数不记名"：那条守的是**任务日志**别被
//! 六千行灌爆（日志是每次动作的证据，不是数据集）。名字进这份单独的文件，日志照旧只记数。
//!
//! 形状的全部意义是可 diff：**时刻与来源一律进文件名，内容里只有名字与两行固定节头**。
//! 内容里写一行"几点算的"，两周的两份文件 diff 起来就永远整片红。

use std::path::{Path, PathBuf};

use crate::prune::PruneReport;
use crate::rcd::redact_credentials;

/// 与那份任务日志**同名**（只换扩展名），住在日志旁边的 `lists/` 里：
/// 拿着日志能找到名单，反过来也一样。同一秒内撞名由 `TaskLog::create` 的 `-1` 后缀解决，
/// 这里不必再造一套。
pub fn path_for(log: &Path) -> PathBuf {
    let stem = log.file_stem().map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "list".to_string());
    log.parent().unwrap_or(log).join("lists").join(format!("{stem}.txt"))
}

/// 两节、一行一个名字、节头是固定字。`secrets` 与任务日志同一份：对象名理论上可以
/// 把凭据当名字用，那份值不许顺着名单落到盘上（"密钥的值从来不上屏"这条不分出口）。
pub fn body(report: &PruneReport, secrets: &[&str]) -> String {
    let mut out = String::from("# 本地有、云端没有\n");
    for name in &report.pending_upload {
        out.push_str(&redact_credentials(name, secrets));
        out.push('\n');
    }
    out.push_str("\n# 云端有、本地没有\n");
    for name in &report.orphans {
        out.push_str(&redact_credentials(name, secrets));
        out.push('\n');
    }
    out
}

/// 写下去，返回落点。错误交给调用方**说出去**（日志里那一行"名单没存成"）——
/// 不许 `let _ =`：这份文件是答应过的东西，没落成必须看得见它没落成。
pub fn write(log: &Path, report: &PruneReport, secrets: &[&str]) -> std::io::Result<PathBuf> {
    let path = path_for(log);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, body(report, secrets))?;
    Ok(path)
}
