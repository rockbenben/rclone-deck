use std::path::Path;

use serde::{Deserialize, Serialize};

/// 认领孤儿靠这几项一起对：PID 会被系统复用，光有 PID 不够。
///
/// 写入时机是这条记录的一部分（裁定 R42 item 1）：监管器在子进程一起来就把这条记录落盘，
/// **早于**健康检查与 `core/version`。任何在那之后才发生的早退都是"端口还绑着、
/// `Child` 句柄已丢弃"的出口，而记录要是还没写，那个 daemon 就再也收不回来——
/// 下一次启动只会得到 `PortBusyByForeignProcess`，面板从此自己变砖。
/// 反过来，`!healthy` 那条出口会把它删掉：那里的子进程已经被终结，记录留着只是给将来
/// 复用同一个 PID 的无关进程递一条假线索。
///
/// `panel_pid` 是写下这条记录的那个**面板进程**（`std::process::id()`，裁定 R42 item 3）：
/// `pid_path` 和端口（现挑的那个号在同一台机器上是确定的，理由见 `rcd::reclaim` 头上那段）对同一个
/// 用户是**常量**，dev 构建与 release 构建共用它们，而 Task 6
/// 的互斥锁是进程内的（`AtomicU64`），挡不住两个面板同时跑。其余三项只能证明"端口上那个是
/// **我们这类** rclone"，证明不了"写记录的那个面板已经不在了"；少了 `panel_pid`，
/// B 面板会在 A 面板传到一半时把 A 的 rcd 终结掉。
/// `#[serde(default)]` = 0 表示这条记录还没有这个字段（升级期间留下的孤儿）：那种情况下
/// 退回三重身份判定，别让"读不出记录"变成一种新的不可回收残局。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PidRecord {
    pub pid: u32,
    pub start_time: u64,
    pub port: u16,
    #[serde(default)]
    pub panel_pid: u32,
}

pub fn read(path: &Path) -> Option<PidRecord> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write(path: &Path, rec: PidRecord) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string(&rec).expect("PidRecord 可序列化"))
}

/// 删掉那条记录，并把失败交回调用方（裁定 R56 item 3）。
///
/// 为什么它**不许**是 `let _ =`：本项目自己定的规矩是"清理失败不许静默"，而这条记录留在磁盘上
/// 不是一堆无害的字节——PID 会被系统复用，下一个恰好复用该 PID 的进程会带着记录里的 `port` 与
/// `start_time` 被误认成"我们的 rcd"，也就是**一条假线索**。记录过期本身是 fail-safe 的
/// （`reclaim` 还要重查 `start_time` + 完整 exe 路径，且只在不匹配时只报告不杀），
/// 所以这里的后果有界；但有界的后果也要说出来，理由同 `wipe_credential_echo`（裁定 R42 item 2）。
/// 怎么个说法由调用方决定：健康检查没过那条出口把它拼进原有的 `RcdError` 话术，
/// 原始失败先说、收尾失败跟在后面，不为此新增错误变体。
pub fn remove(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(path)
}
