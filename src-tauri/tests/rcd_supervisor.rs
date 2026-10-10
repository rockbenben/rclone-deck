#![cfg(windows)]

// 这个文件的规则（裁定 R45，由 R47 收口）：**凡能留下活 daemon 的用例都必须持有守卫**。
// 两档口径（pid 记录档 + 端口档）写在下面 `StrayGuard` 的文档注释里，Task 9 往这里加起 daemon 的
// 用例时照它抄。守卫那句"是不是我们的 rclone"**只认构造点交给它的那份 spec 的 `rclone_bin`**
// （裁定 R47 item 1），两档都不许再读下面那个文件级 helper。

use std::path::{Path, PathBuf};
use std::time::Duration;

use rclone_deck_lib::rcd::{
    pidfile, random_credentials, rclone_env_is_scrubbed, redact_credentials, RcdError, RcdHandle,
    RcdSpec, RcdSupervisor, LOG_ROTATE_BYTES,
};

/// 生产端口。可用 `RCLONE_DECK_PORT` 覆盖（与 `config::deck_port` 同一个出口）。
///
/// 2026-10-09 实测：本机的**动态排除端口区**会漂，有一轮它正好盖住 5460–5659，把当时的默认
/// 5599 一起吃了 —— 那不是"端口被占"，是"这个端口不许绑"，`rclone rcd` 于是报
/// `bind: … forbidden by its access permissions`，**整个 target 整组红**，而病因与被测代码
/// 一行都无关。默认值后来抬到 6400（理由见 `AppConfig::PORT`），这里跟生产走同一个出口，
/// 不再自己抄一份数字 —— 抄一份的版本在默认值挪位时会静默留下一条"与生产不是同一个端口"的证人。
/// 守卫（`Drop`）也读它：它要终结的 daemon 正是这个端口上那个。
/// 2026-10-10 补：上头的"抬默认值"已经被这台机器证伪了一次 —— 排除区当天就漂到
/// **6383–6482**，把抬上去的 6400 一起吃了，于是这个 target 又整组红
/// （`bind: … forbidden by its access permissions`，11 条），病因仍然与被测代码无关。
/// 抬数字是**挪雷**，所以 fallback 改成"挑一个当下绑得上的"。
///
/// 为什么"试一下能不能绑"就够：被占 bind 失败，**落在排除区里 bind 同样失败**
/// （WSAEACCES）—— 两种病因在判据上是同一件事，不必去解析 `netsh` 那两张表。
/// 显式设了 `RCLONE_DECK_PORT` 仍然照旧优先（那是人要的那个号，不猜也不改）。
fn default_port() -> u16 {
    match std::env::var("RCLONE_DECK_PORT") {
        Ok(s) => s.trim().parse().expect("RCLONE_DECK_PORT 不是一个 u16 端口号"),
        // 候选窗口躲开生产的 6400：这个 target 守的就是"生产那个号上发生了什么"，
        // 悄悄换成一个别的号会让它测的不再是生产那条路。真要测生产那条路就设
        // `RCLONE_DECK_PORT=6400`。
        Err(_) => {
            for cand in 15000..=49000u16 {
                if std::net::TcpListener::bind(("127.0.0.1", cand)).is_ok() {
                    return cand;
                }
            }
            panic!("15000–49000 里没有一个绑得上的号");
        }
    }
}

/// 读法收成这一个函数。写死 `const PORT: u16` 读不到运行时环境；
/// 而它在这个文件里的两种用法都得成立 —— `p()` 进要 `u16` 的调用，
/// `{p}` 进那十几句 format 串（隐式捕获只认**标识符**，`{p()}` 编译不过），
/// 所以每处 format 串旁边都有一句 `let p = p();`。值在第一次读时取环境变量并缓存。
static PORT_VALUE: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
fn p() -> u16 {
    *PORT_VALUE.get_or_init(default_port)
}

/// 只是 `spec()` 那份夹具的默认 rclone 路径（可用 `RCLONE_BIN` 覆盖）。
/// 裁定 R47 item 1：守卫**不读它**——Task 9 的 `setup_with_conf` 那份 spec 的 `rclone_bin` 是从
/// 配置派生的，跟这里的环境变量默认值没有必然关系；守卫要是继续读它，那时候两档会同时瞎掉。
fn rclone_bin() -> String {
    std::env::var("RCLONE_BIN").unwrap_or_else(|_| {
        "C:\\Users\\Administrator\\AppData\\Local\\Microsoft\\WinGet\\Links\\rclone.exe".into()
    })
}

/// 目录里的文件名，排过序。轮转那条用它来看"归档到底造出来没有"。
fn list_dir(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn spec(dir: &Path) -> RcdSpec {
    let p = p();    RcdSpec {
        rclone_bin: Path::new(&rclone_bin()).to_path_buf(),
        port: p,
        flags: vec!["--fast-list".into()],
        exclude: vec!["Thumbs.db".into()],
        // 本任务的测试不碰任何 remote（只验 spawn/凭据/窗口/pid 回收），所以用 rclone 的默认配置位置。
        // 需要具名 remote 的是 Task 9（`setup_with_conf`），不是这里。
        rclone_config: None,
        log_file: Some(dir.join("rcd.log")),
        pid_path: dir.join("rcd.pid"),
        log_level_debug: true,
    }
}

async fn port_free(port: u16) -> bool {
    let _ = port;
    tokio::time::timeout(Duration::from_millis(300),
        tokio::net::TcpStream::connect(("127.0.0.1", p()))).await.is_err()
}

/// 路径比较的统一口径：剥掉两侧的引号（Windows 的命令行参数带空格时会被引号包住），
/// 大小写不敏感，`/` 与 `\` 等价。守卫的两条归属判据（`is_our_rcd` 的完整 exe 路径、
/// `cmd_carries_my_fixture_dir` 的 fixture 目录）都走它，避免同一个字符串被两处按不同口径比。
///
/// **不做 canonicalize**：这两条比较的两侧都是**本用例自己**交出去的同一段字符串
/// （那份 spec 的 `rclone_bin`、`dir.join("rcd.log")` 与 `dir.join("rcd.pid")` 同一个 `dir`），
/// 值得折进来的差异只有引号/大小写/分隔符。canonicalize 反而会引入混合状态：目录一定存在、
/// 那个日志文件可能还没被 rclone 建出来，一侧拿到 `\\?\` verbatim 前缀一侧拿不到，
/// 判据就从"能对上"变成"永远对不上"（端口档因此静默不干活，比松一点更糟）。
fn norm_path(p: &str) -> String {
    p.trim_matches('"').to_lowercase().replace('/', "\\")
}

/// 裁定 R47 item 3：`RcdError` 的每个变体都携着**生产写进去的字符串**（`Io(msg)`、
/// `SpawnFailed(msg)`、`NotHealthy(_, detail)` 的 detail 是从 rclone 的 stderr 读来的、
/// `OrphanNotReapable{reason}`、`PortBusyByForeignProcess{name}`），所以
/// `panic!("期望 …，实际 {other:?}")` 这种话术安不安全，取决于生产那一侧此刻往这些字段里
/// 放了什么——这正是 `7fc1725` 关掉的那类口子。这些 `panic!` 从此只报**变体名**：
/// 够定位"跑错到哪个分支了"，不含任何内容。别往这个返回值上再 `format!` 那个值。
fn err_variant(e: &RcdError) -> &'static str {
    match e {
        RcdError::PortBusyByForeignProcess { .. } => "PortBusyByForeignProcess",
        RcdError::OrphanNotReapable { .. } => "OrphanNotReapable",
        RcdError::SpawnFailed(_) => "SpawnFailed",
        RcdError::NotHealthy(..) => "NotHealthy",
        RcdError::Io(_) => "Io",
    }
}

/// 一个"绝不会是活进程"的 PID：本机的 PID 都在 6 万上下，400 万这个号从来没被发出去过，
/// 所以 `pidfile` 里写它 = "写这条记录的那个面板已经不在了"（裁定 R42 item 3 的活性判定据此放行）。
const DEAD_PANEL_PID: u32 = 4_000_000;

/// **规则（裁定 R45，R47 item 2 收口）：这个文件里凡能留下活 daemon 的用例，
/// 都必须持有一个守卫，覆盖"daemon 被拉起 → 测试自己 `shutdown()` 收掉"之间的每一段。**
/// 不是某条用例的特殊照顾——Task 9 还会往这里加起 daemon 的用例，下面两档就是给它们照抄的口径：
///
/// 1. `start()` 返回 `Ok` 句柄的 → 句柄一拿到就 `StrayGuard::new(h, spec.rclone_bin, pid_path, env_keys)`，
///    正常路径 `release()` 把句柄还给测试自己 `shutdown()`，守卫就不插手；
/// 2. spawn 之后返回 `Err`、手上没有句柄的 → `StrayGuard::orphan_sweep(spec.rclone_bin, pid_path, env_keys)`，
///    在 `start()` **之前**建好。它**两档都走**（裁定 R47 item 2）：先按 pid 记录找（裁定 R42 item 1
///    把记录提前落盘，正是为了让这条路有依据），记录不存在/读不到时退到端口档——
///    于是"spawn 之后、`pidfile::write` 之前"这个窗口里 panic 也有人收尾（复审 F-4 点名的就是这段）。
///    同一条路上"故意把 daemon 丢成孤儿"的那两条（`drop(a)` 模拟崩溃）也靠它，句柄形态收不了。
///
/// 端口档自裁定 R46 起要过**两条**闸才动手（`panicking()` + 候选命令行带着本用例自己的 fixture
/// 目录），裁定 R47 item 2 把这个开关对**所有**守卫点打开：原来那四条记录档用例写死
/// `port_leg = false`，理由只是"端口档没有可靠的归属判据"，而 R46 已经把判据造出来了。
///
/// 不起进程的用例（纯函数、以及端口被本进程自己的 listener 占住的那条）**不要**套守卫：
/// 守卫的收尾是"按 PID 终结"，套错了就是拿它去撞别人的进程（R45 的"不许做的事"）。
///
/// 为什么非要有守卫：`RcdHandle::shutdown()` 需要 await，而 panic 展开时那个 future 已经没有
/// runtime 可等，于是真 daemon 搁浅在生产那个端口上、`RCLONE_COPY_LINKS` 也留在进程环境里；
/// 套件按 `--test-threads=1` 跑，一次搁浅把后面每一条 rcd 用例连锁带红（连锁红掩盖病因，
/// 比多一个进程更难查）。守卫做的三件事全都不需要 runtime：
/// 1. 终结 daemon——有句柄走 `kill_now()`（`start_kill`，不等 await，所以 `Drop` 里能调），
///    没句柄走 `kill_pid()`，且**先按三重身份确认那个 PID 确实是本用例的 rclone rcd**；
/// 2. 阻塞轮询到那个 PID 真的从进程表里消失——否则端口还绑着，下一条用例的 `start` 会撞上
///    "端口被占 + 记录里的 panel_pid 还活着（就是本进程）"的窗口；
/// 3. 恢复环境变量（`remove_var`，展开路径上原来那句直接被跳过）。
///
/// `Drop` 里不许 `unwrap`/`expect`：守卫自己 panic 会把展开变成中止。
struct StrayGuard {
    handle: Option<RcdHandle>,
    /// **起这个 daemon 的那份 spec** 的 `rclone_bin`（裁定 R47 item 1）。两档的"是不是我们的
    /// rclone"都以它为准，不读文件级 helper `rclone_bin()`：Task 9 的 spec 从配置派生，
    /// 到那时候 helper 和真值会不是同一个东西，继续读它等于两档同时瞎掉（daemon 搁浅 + 连锁红）。
    expected_exe: PathBuf,
    pid_path: PathBuf,
    env_keys: &'static [&'static str],
}

impl StrayGuard {
    /// 句柄形态（裁定 R45 item 1 点名的那几条）：`start()` 已经交出 `Ok` 句柄。
    /// `expected_exe` 取起这个 daemon 的那份 `spec.rclone_bin`（裁定 R47 item 1）。
    fn new(
        handle: RcdHandle,
        expected_exe: PathBuf,
        pid_path: PathBuf,
        env_keys: &'static [&'static str],
    ) -> Self {
        Self { handle: Some(handle), expected_exe, pid_path, env_keys }
    }

    /// 无句柄形态（裁定 R45 item 2 + R47 item 2）：走 spawn 之后返回 `Err` 那条路的用例手上
    /// 根本没有 `RcdHandle`（监管器在健康窗口耗尽 / pid 文件写不下去的出口自己终结子进程，
    /// 而**用例的 future 在那之前被打断**时，子进程正绑着端口、`Child` 已经没了）。
    /// 于是收尾有两档依据：先读 pid 记录（裁定 R42 item 1 把记录写在任何可能早退的动作之前，
    /// 承诺的就是"这条 panic 路径认得出该收谁"），记录不存在时再退到端口档——R46 把
    /// "这个 daemon 是不是本用例起的"的凭据（命令行带着本用例的 TempDir）造出来之后，
    /// 这一档已经没有理由再对哪条用例关闭。`expected_exe` 同样取自那份 spec。
    fn orphan_sweep(
        expected_exe: PathBuf,
        pid_path: PathBuf,
        env_keys: &'static [&'static str],
    ) -> Self {
        Self { handle: None, expected_exe, pid_path, env_keys }
    }

    /// 借出内部的 `RcClient`，让测试正常路径不用先把句柄从守卫里拆出来。
    fn client(&self) -> &rclone_deck_lib::rc::RcClient {
        &self.handle.as_ref().expect("守卫的句柄已经被取走，测试写法有误").client
    }

    fn release(&mut self) -> RcdHandle {
        self.handle.take().expect("守卫的句柄被取了两次")
    }

    /// 三重身份判定（裁定 R45 item 2 说的"复用已有的 helper，不要新写一套认进程的逻辑"）：
    /// 读进程靠现成的 `command_line_of`（本文件已经拿它钉"凭据不上 argv"、也拿它判活），
    /// 判定要同时命中三件事——① 映像就是我们那份 spec 的 `rclone_bin`、② 子命令是 `rcd`、
    /// ③ `--rc-addr` 指着我们这个端口。生产 `reclaim` 对的是 PID + start_time + exe + 端口，
    /// 守卫手上没有 `Child` 也没导出 `inspect`，命令行里的 exe 与 rc-addr 就是那几项的对应物。
    ///
    /// 裁定 R46 item 3：第①件从"文件名"收成**完整路径**——本机恰好装着两个不同构建的
    /// `rclone.exe`（v1.75.1 与 v1.72.1 不是同一个东西），只比可执行文件名那一段等于把它们当成
    /// 同一个，端口档因此会把另一个构建的 rcd 也认成"我们的"。比较走 `norm_path`
    /// （剥引号、大小写不敏感、`/` 与 `\` 等价），比的是"命令行第一段 == 期望 exe"这一整条
    /// 路径。期望 exe 由调用方给（裁定 R47 item 1：守卫传 `self.expected_exe`，也就是
    /// **起这个 daemon 的那份 spec** 的 `rclone_bin`），这里不再读文件级 `rclone_bin()`。
    /// 严格意义上的 PID + `start_time` + 完整 exe 对账在记录档那一侧由
    /// `record_identity_matches` 补上（就是 `reclaim` 用的那两行）。
    /// 注意这三件只证明"是一个 rclone rcd 在听我们的端口"，**不**证明"是本用例起的"——
    /// 那一条归 `cmd_carries_my_fixture_dir`（裁定 R46 item 1），端口档两条都要过。
    /// 判定完绝不把命令行打印出去：按不变量它不含凭据，但摊到终端只是给未来的回归留门。
    fn is_our_rcd(pid: u32, expected_exe: &Path) -> bool {
        Self::cmd_is_our_rcd(&rclone_deck_lib::rcd::command_line_of(pid), expected_exe)
    }

    /// `is_our_rcd` 的判定本体，收 `&[String]` 是为了让端口档读一次命令行就能把两条判据
    /// （身份 + fixture 路径）都判完。
    fn cmd_is_our_rcd(cmd: &[String], expected_exe: &Path) -> bool {
    let p = p();
        let Some(first) = cmd.first() else { return false }; // 进程已不在（或这个 PID 从未被发出）
        if norm_path(first) != norm_path(&expected_exe.display().to_string()) {
            return false; // 映像不是起这个 daemon 的那一个 `rclone.exe`
        }
        let joined = cmd.join(" ").to_lowercase();
        cmd.iter().any(|a| a.eq_ignore_ascii_case("rcd"))
            && joined.contains(&format!("127.0.0.1:{p}"))
    }

    /// 端口档的**归属**判据（裁定 R46 item 1）：候选进程的命令行里必须出现**本守卫自己的
    /// fixture 目录**（从 `self.pid_path.parent()` 取，不硬编码字符串）。本用例的 `pid_path`
    /// 与 `log_file` 都在自己那个 `TempDir` 里，我们起的那个 daemon 的命令行因此必然带着
    /// `--log-file <该目录>\rcd.log`；别的进程——另一个面板（它的 log/pid 在它自己的目录里）、
    /// 另一个用例的 TempDir——都不可能带着**这个**目录。这是唯一一条**不依赖 pid 记录**的
    /// "端口上那个是不是我起的"的判据，正是端口档缺的那一半（复审量到端口档会截断活着的面板
    /// 同行：`normal-drop alive = true; after-panic alive = false`）。判不出来就什么都不做，
    /// 让下一条用例的 `reclaim` 去报 `PortBusy…`：留一个进程的代价远小于截断同行的传输。
    fn cmd_carries_my_fixture_dir(&self, cmd: &[String]) -> bool {
        // 拿不到父目录、或父目录不是绝对路径（那就不再是一个具体的 TempDir，而是一条会把任何
        // 命令行都命中的空/宽判据）时返回 false：判据不成立就是"什么都不做"，不是"全都算"。
        let Some(dir) = self.pid_path.parent().filter(|d| d.is_absolute()) else { return false };
        let dir_norm = norm_path(&dir.display().to_string());
        !dir_norm.is_empty() && cmd.iter().any(|a| {
            // 按**路径分量**比（不是子串）：兄弟 TempDir 的名字可能是彼此的前缀
            // （`.tmpabcd` 与 `.tmpabcd1234`），子串匹配会把别的用例也认成自己。
            let arg = norm_path(a);
            Path::new(&arg).starts_with(Path::new(&dir_norm))
        })
    }

    /// 写下这条记录的面板是不是"另一个还活着的面板"（裁定 R42 item 3 的同一条不变量，
    /// 判活复用 `command_line_of`：不存在的 PID 返回空表）。`0` = 还没有这个字段的旧记录，
    /// 本进程 = 上一轮的自己，两种都放行。
    fn panel_is_live_sibling(panel_pid: u32) -> bool {
        panel_pid != 0 && panel_pid != std::process::id()
            && !rclone_deck_lib::rcd::command_line_of(panel_pid).is_empty()
    }

    /// 按 PID 终结 + 阻塞轮询到它真消失。`Drop` 里，所以一条 `unwrap` 都不能有。
    fn terminate_and_wait(&self, pid: u32) {
    let p = p();
        if !rclone_deck_lib::rcd::kill_pid(pid) {
            // 说出来的那句就是"这条用例即将给自己那个号留一个 daemon"——后面的连锁红因此可解释。
            eprintln!("rcd 守卫无法终结 PID {pid}：{p} 上可能留着一个 daemon");
            return;
        }
        for _ in 0..200 {
            if rclone_deck_lib::rcd::command_line_of(pid).is_empty() {
                // 守卫在 `Drop` 里、一条 `unwrap` 都不能有，但清理失败照样不许静默（裁定 R56 item 3）：
                // 说出来——那条记录留在原地，就是给下一个复用该 PID 的进程递一条假线索。
                if let Err(e) = pidfile::remove(&self.pid_path) {
                    eprintln!("rcd 守卫删不掉 pid 记录 {}：{e}", self.pid_path.display());
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!("rcd 守卫终结 PID {pid} 之后它仍在进程表里：{p} 可能还被占着");
    }

    /// 无句柄形态的收尾（裁定 R45 item 2，R47 item 2 补全）：先读 pid 记录，读不到/不认再退到
    /// 端口档。两档都过，不再有"这条用例只走记录档"的开关。
    fn sweep_without_handle(&self) {
    let p = p();
        if let Some(rec) = pidfile::read(&self.pid_path) {
            // 裁定 R46 item 3：记录档除了命令行那三条身份，还要比 `rec.start_time` 与**完整
            // exe 路径**——复用 `reclaim` 的那条判据（`record_identity_matches` 就是那两行的
            // 导出），不新写第二套。PID 会被系统复用，光看"现在这个 PID 是某个 rclone rcd"
            // 不够：那可能已经是一个跟我们无关、恰好接着这个号跑的进程。
            // 裁定 R47 item 1：这两处期望 exe 都是 `self.expected_exe`，也就是起这个 daemon
            // 的那份 spec 的 `rclone_bin`（不是文件级 helper）。
            if rec.port == p
                && Self::is_our_rcd(rec.pid, &self.expected_exe)
                && rclone_deck_lib::rcd::record_identity_matches(
                    rec.pid, rec.start_time, &self.expected_exe)
            {
                if Self::panel_is_live_sibling(rec.panel_pid) {
                    eprintln!(
                        "rcd 守卫不动 PID {}：记录是另一个还活着的面板（PID {}）写的——宁可留一个进程也不截断同行的传输",
                        rec.pid, rec.panel_pid
                    );
                    return;
                }
                self.terminate_and_wait(rec.pid);
            }
            // 记录在、但对不上（PID 已被复用 / start_time 对不上 / 端口不是我们的 / 不是我们的
            // rclone）：按它杀就是撞别人，什么都不做，让下一条用例的 `reclaim` 去报 `PortBusy…`。
            return;
        }
        // 端口档：记录不在（还没写下、写不下去、或已被出口收走）时唯一的线索，
        // 裁定 R47 item 2 起它对**所有**守卫点开放。而且**两条**判据都要过。
        // ① `std::thread::panicking()`（裁定 R46 item 2：留着，它必要不充分——正常结束时端口上
        //    若正好挂着另一个面板的真 daemon，去碰它就是 R42 item 3 明令禁止的那种截断，
        //    但"我在 panic"推不出"端口上那个是我起的"）；
        // ② 候选进程的命令行里带着本守卫自己的 fixture 目录（裁定 R46 item 1，
        //    见 `cmd_carries_my_fixture_dir`）——这条才是"是不是**我们这个**用例起的"的凭据，
        //    三重身份判定只认得出"一个 rclone rcd 在听某个号"，认不出它属于哪条用例/哪个面板。
        // 任一条不成立就什么都不做，让下一条用例的 `reclaim` 去报 `PortBusy…`。
        if !std::thread::panicking() {
            return;
        }
        if let Some(pid) = rclone_deck_lib::rcd::port_listener_pid(p) {
            let cmd = rclone_deck_lib::rcd::command_line_of(pid);
            if Self::cmd_is_our_rcd(&cmd, &self.expected_exe) && self.cmd_carries_my_fixture_dir(&cmd) {
                self.terminate_and_wait(pid);
            }
        }
    }
}

impl Drop for StrayGuard {
    fn drop(&mut self) {
    let p = p();
        for key in self.env_keys {
            std::env::remove_var(key);
        }
        if let Some(mut h) = self.handle.take() {
            let pid = h.pid;
            if let Err(e) = h.kill_now() {
                // 说出来的那句就是"这条用例即将给自己那个号留一个 daemon"——后面的连锁红因此可解释。
                eprintln!("rcd 守卫无法终结 PID {pid}：{e}——{p} 上可能留着一个 daemon");
                return;
            }
            for _ in 0..200 {
                if rclone_deck_lib::rcd::command_line_of(pid).is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            if let Err(e) = pidfile::remove(&self.pid_path) {
                // 同上（裁定 R56 item 3）：`Drop` 里不许 panic，但也不许静默。
                eprintln!("rcd 守卫删不掉 pid 记录 {}：{e}", self.pid_path.display());
            }
            return;
        }
        // `release()` 之后句柄也是空的：无句柄档只在"这条用例从没拿到句柄"时才可能留东西，
        // 而它两档判定都要求先确认那个进程是我们的 rclone rcd，所以正常收尾（已 shutdown、
        // 记录已删、端口已空）在这里就是一条都不命中、什么都不做的路径。
        self.sweep_without_handle();
    }
}

#[tokio::test]
async fn start_then_shutdown_roundtrip() {
    let p = p();
    assert!(port_free(p).await, "开跑前 {p} 就该是空的");
    let dir = tempfile::tempdir().unwrap();
    let s = spec(dir.path());
    let h = RcdSupervisor::start(s.clone()).await.unwrap();
    // 裁定 R45 item 1：句柄一进门就交给守卫，下面任何一条断言 panic 都不会把 daemon 留给下一条用例。
    let (pid, version) = (h.pid, h.rclone_version.clone());
    let mut guard = StrayGuard::new(h, s.rclone_bin.clone(), dir.path().join("rcd.pid"), &[]);
    // 裁定 R47 item 1：期望 exe 取自**起这个 daemon 的那份 spec**（`s.rclone_bin`），
    // 不是文件级 helper `rclone_bin()`——Task 9 的 spec 从配置派生，届时两者会不是一回事。

    assert!(pid > 0);
    assert!(!version.is_empty(), "core/version 没读回来");
    assert!(port_free(p).await == false);

    let rec = pidfile::read(&dir.path().join("rcd.pid")).expect("pid 文件应该已写好");
    assert_eq!(rec.pid, pid);
    assert_eq!(rec.port, p);
    assert!(rec.start_time > 0, "start_time 为 0 的话孤儿认领永远不会命中");
    // 裁定 R42 item 3：记录里还得留下"是哪个面板写的它"，否则收尸程序分不清
    // 上一个面板已经崩溃、还是另一个面板正在用它。
    assert_eq!(rec.panel_pid, std::process::id(), "panel_pid 必须是当前进程");

    // 正常路径：句柄还给自己 `shutdown()`（它会等退出 + 确认端口释放），守卫从此不插手。
    guard.release().shutdown().await.unwrap();
    assert!(port_free(p).await, "shutdown 之后端口应已释放");
    assert!(pidfile::read(&dir.path().join("rcd.pid")).is_none(), "shutdown 应删掉 pid 文件");
}

/// 裁定 R42 item 3 的证人：**四件事全对，也不许收一个活着的面板同行的 daemon**。
/// `pid_path` 与端口（本进程由 `p()` 现挑一次，见 `default_port`）对这一批用例是常量
/// （dev 构建与 release 构建共用），Task 6 的互斥锁
/// 又只在进程内，所以"两个面板同时跑"是真实可达状态——而 B 的 `start` 一旦按 PID 号杀过去，
/// 截断的是 A 传了一半的 `rclone sync`：本项目的 founding incident 就是两次反向 sync 互相删文件。
///
/// 夹具形状（同一个测试进程造不出第二个面板进程，所以只能伪造记录，且必须伪造得**只差那一条判据**）：
/// 先真起一个 daemon A，把它自己写下的记录原样读回来，只把 `panel_pid` 改成**另一个真实活着、
/// 但不是本进程的 PID**（一条 `ping -n 30` 的子进程，够活过这次启动尝试），再试 `start`。
/// 其余三条身份（PID / start_time / exe）仍然逐字匹配，所以这条用例变红只可能是因为
/// 活性检查没了，不可能是因为夹具本来就不像孤儿。
/// 断言分两头：报的是 `PortBusyByForeignProcess`；**并且 A 还活着**（凭据还在它内存里，
/// `core/version` 仍然答得出来）——这才是"没杀错进程"的证据，而不只是"报错了"。
#[tokio::test]
async fn a_live_sibling_panels_daemon_is_never_reaped() {
    use std::os::windows::process::CommandExt;

    let dir = tempfile::tempdir().unwrap();
    let s = spec(dir.path());
    let pid_path = dir.path().join("rcd.pid");

    // 顶替"另一个还活着的面板"：cmd 里跑 30 次 ping，用例结束时自己收掉。
    let mut peer = std::process::Command::new("cmd")
        .arg("/C").arg("ping -n 30 127.0.0.1 >nul")
        .creation_flags(rclone_deck_lib::rcd::CREATE_NO_WINDOW)
        .spawn().expect("造一个活着的面板替身");
    assert!(peer.id() > 0 && peer.id() != std::process::id(),
        "替身进程必须和自己是两个 PID，否则这条用例验的是别的东西");

    let a = RcdSupervisor::start(s.clone()).await.unwrap();
    // 裁定 R45 item 1：A 是这条用例起的真 daemon，句柄一进门就交给守卫——
    // 下面任何一条断言（包括"同行的 daemon 被杀了"那条）panic 时，A 都不该留给下一条用例。
    let mut guard = StrayGuard::new(a, s.rclone_bin.clone(), pid_path.clone(), &[]);
    let mut rec = pidfile::read(&pid_path).expect("A 启动后应该有 pid 记录");
    rec.panel_pid = peer.id(); // 一个活着的面板（不是本进程）
    pidfile::write(&pid_path, rec).unwrap();

    // 前提得当场成立：替身此刻真的活着（`command_line_of` 对不存在的 PID 返回空表）。
    assert!(!rclone_deck_lib::rcd::command_line_of(peer.id()).is_empty(),
        "替身进程没起来 = 活性判定当时对的是'已死'，这条用例空转");

    let started = RcdSupervisor::start(s).await;

    // 替身收干净：它是被我们杀的那个，不该留在机器上（`ping` 还要跑 30 秒）。
    let cleanup = peer.kill().and_then(|()| peer.wait().map(|_| ()));
    assert!(cleanup.is_ok(), "替身进程收尾失败：{cleanup:?}");

    match started {
        Err(RcdError::PortBusyByForeignProcess { port, ref name, .. }) => {
            assert_eq!(port, p());
            assert!(name.contains("面板"), "话术要让操作员认出这是同行，不是陌生进程：{name}");
        }
        Err(other) => panic!("期望 PortBusyByForeignProcess（不许抢同行的 daemon），实际是变体 {}", err_variant(&other)),
        Ok(h) => {
            h.shutdown().await.ok();
            guard.release().shutdown().await.ok();
            panic!("把活着的面板同行的 rcd 收尸了——A 正传到的那一半就这么被打断");
        }
    }

    // 关键的一半：A 的 daemon 得**还活着**（终结过它的话这条必然失败）。
    assert!(guard.client().core_version().await.is_ok(), "同行正在用的 daemon 被杀了");
    guard.release().shutdown().await.unwrap(); // 收尾：这条用例起的 daemon 自己带走，别搁浅给下一条
}

/// 裁定 R42 item 3 的**反方向**：活性检查只管"还活着的面板"，不能顺手把真正的孤儿也挡掉，
/// 否则 item 1 修好的"可回收"又被堵回去（端口上留着一个永远收不回的 daemon = 面板变砖）。
/// 两种"记录的主人已经不在"都要放行：`panel_pid = 0`（升级前写的旧记录，没有这个字段）
/// 和一个从未被发出的 PID（面板进程真的没了）。
#[tokio::test]
async fn a_dead_panels_record_is_still_reaped() {
    let p = p();
    let dir = tempfile::tempdir().unwrap();
    let s = spec(dir.path());
    let pid_path = dir.path().join("rcd.pid");

    for legacy in [0u32, DEAD_PANEL_PID] {
        // 裁定 R45 item 1 + item 2 的交界：这条用例**故意**要留下 A 那个 daemon（`drop(a)`
        // 就是"面板崩溃"的模拟手法），所以 A 不能交给句柄形态——守卫得用无句柄档：
        // 它先按 pid 记录收尾，A 的记录（含下面改成的 `legacy` panel_pid）正是它认人的依据；
        // 裁定 R47 item 2 之后记录读不到时还有端口档兜着。
        let _sweep = StrayGuard::orphan_sweep(s.rclone_bin.clone(), pid_path.clone(), &[]);

        let a = RcdSupervisor::start(s.clone()).await.unwrap();
        let a_pid = a.pid;
        drop(a); // 不调 shutdown：模拟面板崩溃，rcd 留在原地
        assert!(!port_free(p).await, "孤儿 rcd 应该还占着端口，否则这条空转");

        let mut rec = pidfile::read(&pid_path).expect("A 启动后应该有 pid 记录");
        rec.panel_pid = legacy;
        pidfile::write(&pid_path, rec).unwrap();

        let b = RcdSupervisor::start(s.clone()).await.unwrap_or_else(|e| {
            // 裁定 R47 item 3：只报错值，不报内容（`RcdError` 的变体里带着生产写进去的字符串）。
            panic!("panel_pid={legacy} 的记录挡不住收尸才对，实际是变体 {}", err_variant(&e));
        });
        // B 是这条用例正常拿到句柄的那一环，归句柄形态管（句柄守卫后建、先 Drop，
        // 它收完时记录已经删了，`_sweep` 再 Drop 就是一条都不命中、什么都不做）。
        let b_pid = b.pid;
        let mut guard = StrayGuard::new(b, s.rclone_bin.clone(), pid_path.clone(), &[]);
        assert_ne!(b_pid, a_pid, "认领后复用了旧 daemon（它的凭据我们根本没有）");
        guard.release().shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn credentials_never_reach_the_command_line_or_the_debug_log() {
    let dir = tempfile::tempdir().unwrap();
    let (user, pass) = random_credentials();
    let s = spec(dir.path());
    let log_path = s.log_file.clone().unwrap();

    let h = RcdSupervisor::start_with_credentials(s.clone(), &user, &pass).await.unwrap();
    // 裁定 R45 item 1：这条用例读的是**真 daemon** 的命令行，读不到（子进程先没了）就是
    // 空转，而它下面的断言一条比一条容易 panic——句柄一拿到就交给守卫。
    let (pid, secrets) = (h.pid, h.secrets().to_vec());
    let mut guard = StrayGuard::new(h, s.rclone_bin.clone(), dir.path().join("rcd.pid"), &[]);
    // 判定不变（必须是这两条本轮凭据、顺序也不能错），但**用 `assert!` 而不是 `assert_eq!`**：
    // `assert_eq!` 变红时把左右两边整段打印出来，而"右边"就是本轮的真用户名和真口令
    // （复审在 :361 上量到 `left: ["deck-…","<32位hex>"]`）。话术里只许有长度。
    assert!(secrets.as_slice() == [user.as_str(), pass.as_str()],
        "secrets() 不是本轮这两条凭据（口令 Task 8 的脱敏全靠它；期望长度 {}/{}，实到 {} 项）",
        user.chars().count(), pass.chars().count(), secrets.len());
    let argv = rclone_deck_lib::rcd::command_line_of(pid).join(" ");
    assert!(!argv.is_empty(), "读不到子进程命令行，这条断言就是空转");
    assert!(!argv.contains(&pass), "密码出现在命令行里（argv 不打印：这条变红就等于把口令贴进测试输出）");
    assert!(!argv.contains(&user), "用户名出现在命令行里（argv 不打印，同上）");
    guard.release().shutdown().await.unwrap();

    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(!log.is_empty(), "DEBUG 日志没落盘，这条断言就是空转");
    assert!(!log.contains(&pass), "密码进了 rcd 日志");

    // brief 只验了 `--log-file` 这一个落点。本机实测还有第二个：rclone 在
    // `--log-level DEBUG` 下会把环境里的凭据原样打到 **stderr**（`rcd-stderr.log`）上——
    //   DEBUG : Setting --rc-pass "<pass>" from environment variable RCLONE_RC_PASS="<pass>"
    // 同一个进程的 `--log-file` 里却一行都没有（raw 抓取见 task-7-report.md）。
    // "凭据不上磁盘"是不变量，所以第二个落点一样要钉住：supervisor 在确认健康之后
    // 会把那份启动回显清空（失败路径则先读进错误详情再清）。
    let stderr_log = std::fs::read_to_string(dir.path().join("rcd-stderr.log")).unwrap_or_default();
    assert!(!stderr_log.contains(&pass), "密码进了 rcd 的 stderr 落盘文件");
    assert!(!stderr_log.contains(&user), "用户名进了 rcd 的 stderr 落盘文件");
}

/// 裁定 R37：失败启动交出去的错误详情里不许带凭据。
/// `redact_credentials` 是唯一那个把 stderr 文本拼进 `NotHealthy` 之前跑一遍的闸门；
/// 三条 `SpawnFailed` 从不读 stderr，本就带不出口令（见 `rcd/mod.rs` 里的审计注释），
/// 所以这条对它们是 no-op——把脱敏放在唯一读 stderr 的那条上，同时保证这条不变量
/// **不依赖具体哪个分支**。这里喂一段与真 rclone 同形的 debug 回显，钉住口令与用户名都被抹掉。
#[test]
fn failed_start_details_have_the_credentials_redacted() {
    let (user, pass) = random_credentials();
    let echo = format!(
        "DEBUG : Setting rc_pass=\"{pass}\" from environment variable RCLONE_RC_PASS\n\
         DEBUG : Setting --rc-pass \"{pass}\" from environment variable RCLONE_RC_PASS=\"{pass}\"\n\
         ERROR : rc user {user} rejected"
    );
    let red = redact_credentials(&echo, &[&user, &pass]);
    // 失败消息里**不许**有 `{red}`：这条断言变红就意味着脱敏失败，把待断言的原文插进消息
    // 等于把口令写进测试输出（而测试输出会被贴进验收表）。只报形状，不报内容。
    assert!(!red.contains(&pass), "口令没被脱敏：脱敏结果 {} 字符里仍含口令（原文不打印，见本注释）", red.chars().count());
    assert!(!red.contains(&user), "用户名没被脱敏：脱敏结果 {} 字符里仍含用户名（原文不打印）", red.chars().count());
    // R39：掩码写法与 Task 8 的落盘日志统一成 `[REDACTED]`（两边原来各写各的）。
    // 裁定 R47 item 3：这句原来插着 `{red}`，它不泄密**只是因为排在上面两条之后**——顺序一挪
    // 就把 `7fc1725` 关掉的那个口子重新开出来。判定不动（还是"含不含占位串"），话术里只报长度。
    assert!(red.contains("[REDACTED]"),
        "脱敏后应留下占位串：{} 字符里一处 `[REDACTED]` 都没有（原文不打印）", red.chars().count());

    // 空口令绝不能把整段文本吃掉：`replace("", x)` 会命中每一个字符之间，所以 helper 要跳过空串。
    assert_eq!(redact_credentials("plain line", &[""]), "plain line");
    // 裁定 R47 item 4：正典的 `>= 4` 下限在 rcd 这一侧也必须看得见。上面那条（`&[""]`）在
    // `>= 1` 下照样绿，本文件里也没有别的用例递短值——把下限改成 `>= 1` 只红 `tests/tasklog.rs`
    // 那一条（红在另一个 target）。这一条钉住"两个字符的值不擦"，rcd 这个 target 因此守着同一个数。
    assert_eq!(redact_credentials("ab", &["ab"]), "ab");
}

#[tokio::test]
async fn a_foreign_listener_is_reported_with_its_owner_not_worked_around() {
    let p = p();
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", p)).await.unwrap();

    let err = RcdSupervisor::start(spec(dir.path())).await.unwrap_err();
    match err {
        RcdError::PortBusyByForeignProcess { port, pid, ref name } => {
            assert_eq!(port, p);
            assert!(pid > 0);
            assert!(!name.is_empty(), "只报 PID 不报进程名，用户没法自己处理");
        }
        other => panic!("期望 PortBusyByForeignProcess，实际是变体 {}", err_variant(&other)),
    }
    drop(listener);
    assert!(port_free(p).await);
}

#[tokio::test]
async fn a_stale_pid_file_does_not_block_a_fresh_start() {
    let p = p();
    let dir = tempfile::tempdir().unwrap();
    // 一个 PID 都不存在的记录（连"面板进程"也是死的）——端口又没人听，所以这条走的不是
    // `reclaim` 而是"记录被新启动覆盖"，正是标题说的 stale 不挡路。
    pidfile::write(&dir.path().join("rcd.pid"),
        pidfile::PidRecord { pid: 4_000_000, start_time: 1, port: p,
            panel_pid: DEAD_PANEL_PID }).unwrap();

    let s = spec(dir.path());
    let h = RcdSupervisor::start(s.clone()).await.unwrap();
    // 裁定 R45 item 1：真起了 daemon，句柄立刻归守卫。
    let pid = h.pid;
    let mut guard = StrayGuard::new(h, s.rclone_bin.clone(), dir.path().join("rcd.pid"), &[]);
    let rec = pidfile::read(&dir.path().join("rcd.pid")).unwrap();
    assert_eq!(rec.pid, pid);
    assert_ne!(rec.pid, 4_000_000);
    guard.release().shutdown().await.unwrap();
}

/// 裁定 R42 item 1 的证人：**pid 记录必须写在任何可能早退的动作之前**，而早退要把它收回去。
/// 形状：先在 `pid_path` 上放一条**哨兵记录**，再走一条必定 `NotHealthy` 的启动（假 flag →
/// rclone 非零退出，健康循环按截止时间耗尽）。
/// - 实现正确时：监管器在 spawn 之后、健康循环之前就把哨兵**覆盖**成自己的记录，
///   然后在 `!healthy` 出口把它 `remove` → 这里读到 `None`。
/// - 把 `pidfile::write` 挪回健康检查之后（破坏一次 #1）：那条出口根本不会写、也就不会删，
///   哨兵原样留在盘上 → 本条变红。留着的哨兵正是那个缺陷的形状：一次早退之后，
///   `pid_path` 上的内容和真正在跑的 daemon 对不上，而真正的 daemon **一条记录都没有**，
///   下一次启动只能报 `PortBusyByForeignProcess`（面板自己变砖）。
/// 顺带钉住"早退不搁浅 daemon"：出口把子进程终结、端口释放。
#[tokio::test]
async fn a_start_that_early_exits_leaves_no_pid_record_and_no_daemon() {
    let p = p();
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("rcd.pid");
    let mut s = spec(dir.path());
    s.flags.push("--this-flag-does-not-exist".into());
    // 裁定 R45 item 2：这条走的是"spawn 之后返回 Err"的路，手上永远不会有句柄，
    // 所以用无句柄档的守卫，而且要在 `start()` **之前**建好——future 在健康窗口里被打断时，
    // 子进程还活着、端口还绑着。裁定 R42 item 1 提前落盘的那条 pid 记录是头一档依据；
    // 裁定 R47 item 2 之后，记录还没写下（或压根没写成）时端口档兜底。
    // 正常路径上这条是空转的：出口已经把记录和 daemon 一起收掉了，两档都找不到东西。
    // 它必须活到函数最后（不能提前 `drop`）：下面两条断言验的正是"没留下记录/daemon"，
    // 守卫先跑等于替生产的缺陷把现场擦干净。
    let _sweep = StrayGuard::orphan_sweep(s.rclone_bin.clone(), pid_path.clone(), &[]);
    pidfile::write(&pid_path,
        pidfile::PidRecord { pid: 4_000_000, start_time: 1, port: p,
            panel_pid: DEAD_PANEL_PID }).unwrap();

    let exe = s.rclone_bin.clone(); // 下面 Ok 分支那个守卫也要认这份 spec 的 exe（R47 item 1）
    match RcdSupervisor::start(s).await {
        Err(RcdError::NotHealthy(_, detail)) => {
            assert!(!detail.is_empty(), "早退还得到 `rclone 自己的报错`，否则操作员没法判断");
        }
        Err(other) => panic!("期望 NotHealthy，实际是变体 {}", err_variant(&other)),
        Ok(h) => {
            // 真起来了 = 生产坏了；把句柄交给守卫再 panic，别让这条用例顺手留一个 daemon。
            let _h = StrayGuard::new(h, exe, pid_path.clone(), &[]);
            panic!("非法 flag 居然起来了");
        }
    }

    assert!(
        pidfile::read(&pid_path).is_none(),
        "早退的出口留下了 pid 记录：它指的是一个并不存在的 daemon，而真留在端口上的那个没有记录"
    );
    assert!(port_free(p).await, "!healthy 之后端口应已释放（不许搁浅 daemon）");
}

/// 打不开 stderr 日志时必须**说得出是哪个文件**，而且不许把自己打死。
///
/// 这条**钉得住的**：错误话术以前只有 OS 原因（`Access is denied. (os error 5)`），操作员看不出该去
/// 修哪个路径；现在路径在消息里。破坏一次：把 `.map_err` 那句里的 `{}` 去掉 → 本条红（实测过）。
/// 这条**钉不住的**，照实写：`build_command` 原来在 `fs::write` 之后**再打开一次**同一路径，
/// 第二次失败走 `.expect(...)`——release 是 GUI 子系统，panic 没有窗口也没有消息。但那个形状
/// 造不出确定性夹具（两次打开的失败条件相同，夹具永远先撞第一次），所以它不是被这条证人挡住的，
/// 是**被构造消掉的**：现在整条流程只开一次句柄，直接把活口交给子进程，没有"第二次"可失败。
/// 夹具是在 stderr 日志的位置放一个目录，打开必然失败；不会 spawn，所以没有守卫要登记的残骸
/// （裁定 R45 的守卫管"能留下活 daemon 的用例"，这条留不下）。
#[tokio::test]
async fn an_unwritable_stderr_log_names_the_path_and_does_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let s = spec(dir.path());
    // stderr 日志是 `log_file` 的同名兄弟：`rcd-stderr.log`。在原位置放一个目录。
    let stderr_path = s.log_file.as_ref().unwrap().with_file_name("rcd-stderr.log");
    std::fs::create_dir(&stderr_path).unwrap();

    match RcdSupervisor::start(s).await {
        Err(RcdError::Io(msg)) => {
            assert!(msg.contains("rcd-stderr.log"),
                "话术里要点名打不开的是哪个文件，光有 OS 原因修不动：{msg}");
            assert!(!msg.is_empty(), "Io 得带上原因：{msg}");
        }
        Err(other) => panic!("期望 Io（stderr 日志打不开），实际是变体 {}", err_variant(&other)),
        Ok(h) => {
            let _ = h.shutdown().await;
            panic!("stderr 日志位置是个目录，启动居然成功了");
        }
    }
}

/// 轮转不是"一个通过了单测的函数"，它得**接在启动通路上**——这条钉的就是接线。
///
/// 夹具：把 `rcd.log` 写成刚好超过阈值，然后用一个不存在的 flag 让启动在 spawn 之后失败
/// （轮转排在 spawn **之前**，所以启动失败不影响它已经发生）。断言两件事：
/// ① 旧的超尺寸那份变成了带时间戳的归档；② 当前那份的**第一行**写着上一份去哪了——
/// 因为操作员接着翻开的就是这一份，那里没有这句话，归档就等于凭空多出来的一堆文件。
#[tokio::test]
async fn an_oversized_rcd_log_is_rotated_on_the_way_in_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("rcd.log");
    std::fs::write(&log, vec![b'x'; (LOG_ROTATE_BYTES as usize) + 1]).unwrap();
    let mut s = spec(dir.path());
    s.flags.push("--this-flag-does-not-exist".into());
    let exe = s.rclone_bin.clone();
    let _sweep = StrayGuard::orphan_sweep(exe.clone(), s.pid_path.clone(), &[]);

    // 启动本身该失败（假 flag），但那不是这条的断言对象——它只保证不会留下活 daemon。
    let _ = RcdSupervisor::start(s).await;

    let left = list_dir(dir.path());
    assert!(left.iter().any(|n| n.starts_with("rcd.") && n.ends_with(".log")),
        "没有归档被造出来，说明启动通路上根本没调用轮转：{left:?}");
    let fresh = std::fs::read_to_string(&log).expect("当前那份 rcd.log 该在（rclone 自己也会写）");
    // 话术里只摘**第一行**：这一份没轮转时它装的是 rclone 写满的几 MB 传输记录，
    // 整份塞进 panic 消息就等于把测试输出撑爆（实测过一次 4 MB 的失败信息）。
    let first_line = fresh.lines().next().unwrap_or("").to_string();
    assert!(fresh.contains("已转存为"),
        "新日志的第一行没交代上一份去哪了：{first_line}");
    assert!(fresh.contains(&format!("{} 字节", LOG_ROTATE_BYTES + 1)),
        "那句交代得报出挪走了多大一份：{first_line}");
}

/// 裁定 R42 item 1 的顺序证人：pid 记录写在**健康循环之前**，所以"pid 文件根本写不下去"
/// 这类故障立刻以 `RcdError::Io`（操作员能懂、能修的话术）报出来，而不是先花掉 20s 健康窗口、
/// 再把它误报成"rcd 在 20s 内没有响应 rc 请求"。
/// 夹具：`pid_path` 占成一个**目录**（写它必然失败），同时用假 flag 让 rcd 永远不健康。
/// 破坏一次 #1（把 `pidfile::write` 挪回健康检查之后）：这里拿到的是 `NotHealthy`，本条变红。
#[tokio::test]
async fn an_unwritable_pid_file_is_reported_as_io_before_the_health_window() {
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("rcd.pid");
    std::fs::create_dir(&pid_path).unwrap();
    let mut s = spec(dir.path());
    s.pid_path = pid_path.clone();
    s.flags.push("--this-flag-does-not-exist".into());
    let exe = s.rclone_bin.clone(); // 两个守卫点都认这份 spec 的 exe（裁定 R47 item 1）
    // 裁定 R45 item 2 + R47 item 2：**pid 文件写不下去 = 档一（读记录）必然落空**，
    // 这条用例只能靠端口那一档收尾——而端口档现在对所有守卫点都开着，不再有"这条只走记录档"
    // 的开关。它不许悄悄退回"没得收"：端口档找不到东西时守卫就是空转，这正是期望的结果
    // （生产出口本来就该把子进程带走）。
    let _sweep = StrayGuard::orphan_sweep(exe.clone(), pid_path, &[]);

    match RcdSupervisor::start(s).await {
        Err(RcdError::Io(msg)) => {
            assert!(!msg.is_empty(), "Io 得带上 OS 的原因，否则操作员不知道去修什么");
        }
        Err(other) => panic!("期望 Io（pid 文件写不下去），实际是变体 {}", err_variant(&other)),
        Ok(h) => {
            let _h = StrayGuard::new(h, exe, dir.path().join("rcd.pid"), &[]);
            panic!("pid 路径是个目录，启动居然成功了");
        }
    }
}

/// 裁定 R56 item 3 的证人：`pidfile::remove` 是全仓最后一处 `let _ =` 收尾，现在它把
/// `std::io::Error` 交回调用方，而"健康检查没过"那条出口必须**把它说出来**。
///
/// 夹具是上面那条的镜像：上面那条让记录**写不下去**，这条让记录**先正常落盘**（写得了），
/// 等它落了盘、监管器还没走到 `!healthy` 出口上那次删除之前，把它改名挪走并在原位置放一个
/// **目录**——`remove_file` 打不动目录。（std 在 Windows 没有稳定的"设只读位"API，
/// 改名 + 目录是唯一纯 std 的确定性注入，与 `a_credential_wipe_that_cannot_finish_is_reported_not_swallowed`
/// 用的是同一招；20s 健康窗口就是注入的时间窗。）
/// - 实现正确：拿到 `NotHealthy`，话术里既有原始失败、也看得到"pid 记录没能删掉"那一句，
///   且那句排在原始失败**之后**（收尾失败不许顶掉真正的原因），也不带凭据。
/// - 破坏一次（把 `src/rcd/pidfile.rs` 恢复成 `let _ = std::fs::remove_file(path); Ok(())`）：
///   这条 `Err` 里再也看不见收尾失败 → 本条变红，而那条假线索就这么静默留在了磁盘上。
#[tokio::test]
async fn a_pidfile_removal_that_cannot_finish_shows_up_in_the_not_healthy_error() {
    let dir = tempfile::tempdir().unwrap();
    let (user, pass) = random_credentials();
    let pid_path = dir.path().join("rcd.pid");
    let moved_path = dir.path().join("rcd.pid.moved");
    let mut s = spec(dir.path());
    s.pid_path = pid_path.clone();
    s.flags.push("--this-flag-does-not-exist".into());
    let exe = s.rclone_bin.clone(); // 两个守卫点都认这份 spec 的 exe（裁定 R47 item 1）
    // 这条走的是"spawn 之后返回 Err"的路，手上永远不会有句柄，所以用无句柄档的守卫，
    // 而且要在 `start()` 之前建好（裁定 R45 item 2 / R47 item 2）；正常路径两档都不命中。
    let _sweep = StrayGuard::orphan_sweep(exe.clone(), pid_path.clone(), &[]);

    let join = tokio::spawn({
        // 任务要 'static，所以这一对克隆进去；下面断言用的是外面那一对（值相同）。
        let (task_user, task_pass) = (user.clone(), pass.clone());
        async move { RcdSupervisor::start_with_credentials(s, &task_user, &task_pass).await }
    });

    // 注入前先确认那条记录**真的落盘了**：否则"删不掉"是没发生的事故，断言就成了空转。
    let mut landed = false;
    for _ in 0..150 {
        if pidfile::read(&pid_path).is_some() {
            landed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(landed, "rcd.pid 上没落下记录，注入就没有对象，下面那两条断言是空转");
    std::fs::rename(&pid_path, &moved_path).expect("注入：把 pid 记录改名挪走");
    std::fs::create_dir(&pid_path).expect("注入：在原位置放一个目录，让那次删除必然失败");

    let result = join.await.expect("启动任务被 panic 打断");

    match result {
        Err(RcdError::NotHealthy(_, detail)) => {
            // 只报命中与否和长度，不把这个字符串贴进终端（裁定 R47 item 3 的口径）：
            // 生产哪天往这段里放了别的东西，`panic!` 自己就成了泄漏面。
            let cleanup_at = detail.find("此外 pid 记录").unwrap_or_else(|| panic!(
                "NotHealthy 的话术里看不见收尾失败那句话（详情 {} 字符，不打印）",
                detail.chars().count()));
            // 原始失败必须先说：收尾那句前面要是空无一物，就是它把真正的原因顶掉了。
            // `message` 起手是健康循环最后那个错误（`last` 从"一直没有响应"起步，只会被
            // 更具体的错误替换、不会变成空串），所以这一条断言说的就是"原因还在前面"。
            assert!(!detail[..cleanup_at].trim().is_empty(),
                "原始失败被收尾失败顶掉了：Err 里只剩收尾那一句");
            // 操作员拿到这句话得能动手：说的是哪条记录、该怎么办，两句都得在里面。
            assert!(detail.contains("没能删掉"), "话术没说要紧事（详情 {} 字符，不打印）", detail.chars().count());
            assert!(detail.contains("rcd.pid"), "话术没指出是哪条记录（详情 {} 字符，不打印）", detail.chars().count());
            // 这句话术会进面板日志/TaskLog，和 `detail` 同一条泄漏面（裁定 R37 的口径）。
            assert!(!detail.contains(&pass), "NotHealthy 话术把口令交了出去");
            assert!(!detail.contains(&user), "NotHealthy 话术把用户名交了出去");
        }
        Err(other) => panic!(
            "期望 NotHealthy（收尾失败要拼进它的话术），实际是变体 {}", err_variant(&other)),
        Ok(h) => {
            let _h = StrayGuard::new(h, exe, pid_path.clone(), &[]);
            panic!("非法 flag 居然起来了");
        }
    }

    // 反空转：注入得还在原地。原位置要是被生产自己清掉/重建成了文件，上面那条"看不见"
    // 的断言说的就是"删除本来就成功了"，这条证人等于没跑。
    assert!(
        std::fs::metadata(&pid_path).map(|m| m.is_dir()).unwrap_or(false),
        "注入没生效（原位置不是那个目录了）= 上面的断言是空转"
    );

    // 夹具收尾：注入用的目录和改名出去的那份记录都当场删掉，不靠 TempDir 顺手删。
    std::fs::remove_dir(&pid_path).expect("夹具收尾：删掉注入用的目录");
    std::fs::remove_file(&moved_path).expect("夹具收尾：删掉改名出去的 pid 记录");
    assert!(!pid_path.exists() && !moved_path.exists());
}

/// 同一条出口的**残局**证人：pid 记录写不下去时，监管器手里还握着那个刚拉起的子进程，
/// 就必须当场把它终结掉——否则端口上留下一个"没有记录、下次永远收不回来"的 daemon，
/// 正是 item 1 描述的面板自变砖状态（下一次启动只会得到 `PortBusyByForeignProcess`）。
/// 这条用的是**合法 flag**（rcd 真的会健康），所以终结与否是可观察的：端口必须空。
#[tokio::test]
async fn an_unwritable_pid_file_does_not_strand_a_daemon_on_the_port() {
    let p = p();
    let dir = tempfile::tempdir().unwrap();
    let (user, pass) = random_credentials();
    let pid_path = dir.path().join("rcd.pid");
    std::fs::create_dir(&pid_path).unwrap();
    // 裁定 R45 item 2 + R47 item 2：**这条是"pid 记录根本写不下去"的那条，档一（读记录）注定
    // 落空，守卫只剩端口那一档**——而端口档对所有守卫点都开着，不再需要单独点亮它的构造器。
    // 用例用的是合法 flag，所以端口上真会短暂出现一个 daemon：future 在 `start()` 里被打断时，
    // 按 PID 终结它的依据是"netstat 找到那个号上的监听者 + 三重身份判定（期望 exe 取自这份 spec）
    // + 命令行带着本用例的 TempDir"。
    // 同样不许提前 drop：下面那条"端口应已空"的断言才是这条用例要验的东西。
    let mut s = spec(dir.path());
    let exe = s.rclone_bin.clone();
    let _sweep = StrayGuard::orphan_sweep(exe.clone(), pid_path.clone(), &[]);
    s.pid_path = pid_path;

    match RcdSupervisor::start_with_credentials(s, &user, &pass).await {
        Err(RcdError::Io(msg)) => {
            // 新增的这条出口也归"交出去的错误里不许有凭据"这条不变量管（裁定 R37 的口径）。
            // 裁定 R47 item 3：话术里不许插 `msg`——它是生产交出去的文本，插进去等于给未来的
            // 回归留一条把凭据贴进终端的路。判定不动，只报形状。
            assert!(!msg.contains(&pass), "pid 文件的 Io 话术把口令交了出去（详情不打印）");
            assert!(!msg.contains(&user), "pid 文件的 Io 话术把用户名交了出去（同上）");
        }
        Err(other) => panic!("期望 Io，实际是变体 {}", err_variant(&other)),
        Ok(h) => {
            let _h = StrayGuard::new(h, exe, dir.path().join("rcd.pid"), &[]);
            panic!("pid 路径是个目录，启动居然成功了");
        }
    }

    assert!(port_free(p).await, "pid 记录没写成，却把 daemon 留在了 {p} 上——它从此不可回收");
}

/// 裁定 R46 item 4 的证人（照复审的探针形状来）：那条探针量到的是
/// `normal-drop alive = true; after-panic alive = false`——端口档在展开路径上把自己人建的
/// 那道闸（R42 item 3）绕过去了，截断了一个**还活着的面板同行**的 daemon。这条把它钉回去：
/// 真起一个 daemon、它的 pid 记录写在**另一个**目录里（也就是本用例的守卫读不到记录的那种布局，
/// 只剩端口档可走），然后让本用例 panic 展开，断言那个 daemon **还活着**。
///
/// 它必须能**单独**变红：把 R46 item 1 的 fixture 路径判据删掉（端口档退回只有 `panicking()`
/// 一条闸），这条就红。反空转由上面那条 `is_our_rcd` 断言守着——那个 daemon 在"同一个 exe、
/// 子命令 rcd、同一个端口"三条上全部对得上（期望 exe 取的就是起它那份 spec 的 `rclone_bin`，
/// 裁定 R47 item 1），所以拦住守卫的**只有**"它不带本用例的目录"这一条。
/// 收尾归测试自己：那个 daemon 到底还是本用例起的（记录里 `panel_pid` = 本进程），
/// 所以由记录档守卫按 foreign 目录里那条记录收走，不许搁浅给下一条用例。
#[test]
fn a_port_tier_unwind_never_reaps_a_sibling_panels_daemon() {
    let p = p();
    // `foreign` = "另一个面板"的目录（它的 log 与 pid 记录都在里面）；`mine` = 本用例的目录，
    // 守卫只在 `mine` 里找证据。两个都是本地 TempDir，零云端。
    let foreign = tempfile::tempdir().unwrap();
    let mine = tempfile::tempdir().unwrap();
    let foreign_pid_path = foreign.path().join("rcd.pid");
    let my_pid_path = mine.path().join("rcd.pid");
    assert_ne!(foreign.path(), mine.path(), "两个目录必须是两个 TempDir，否则这条验的是别的东西");
    // 起那个 daemon 的 spec：端口档的期望 exe 必须来自**它**（裁定 R47 item 1），
    // 且两个守卫点都传同一个值，否则这条就分不清"归属判据挡住了"和"exe 判据挡住了"。
    let s = spec(foreign.path());

    // 外面的 runtime 只有这一个（`#[test]` 里没有 ambient runtime，`RcdSupervisor::start`
    // 要 tokio 的 process/IO driver）；它在下面那次展开**之后**才 Drop，展开路径上不动它。
    let rt = tokio::runtime::Builder::new_current_thread().enable_all()
        .build().expect("夹具 runtime");
    let daemon = rt.block_on(async {
        // 合法 flag：daemon 真的会健康。句柄随 async 块一起丢（`RcdHandle` 故意没有 Drop），
        // 于是它继续绑着那个号，记录留在 `foreign` 里——这就是"另一个面板在用它"的状态。
        let h = RcdSupervisor::start(s.clone()).await.unwrap();
        h.pid
    });

    // 夹具前提要当场成立，否则断言是空转。
    let rec = pidfile::read(&foreign_pid_path).expect("夹具：记录应写在另一个目录里");
    assert_eq!(rec.pid, daemon, "夹具：记录指的就是端口上那个 daemon");
    assert_eq!(rclone_deck_lib::rcd::port_listener_pid(p), Some(daemon),
        "夹具：{p} 上听的就是那个 daemon，否则端口档根本没被走到");
    assert!(StrayGuard::is_our_rcd(daemon, &s.rclone_bin),
        "夹具：那个 daemon 连三重身份都过不了 = 它活着跟判据没关系，这条空转");
    assert!(pidfile::read(&my_pid_path).is_none(),
        "夹具：本用例的目录里不该有 pid 记录（档一必须注定落空，才只剩端口档）");

    // 这条用例自己的收尾守卫（记录档，读得到 foreign 那条记录）：下面任何一条断言 panic，
    // 都不给下一条用例留下搁浅的 daemon。
    let reap = StrayGuard::orphan_sweep(s.rclone_bin.clone(), foreign_pid_path.clone(), &[]);

    // 复审的探针形状：本用例带着端口档展开（裁定 R47 item 2 之后端口档对所有守卫点都开着）。
    // `catch_unwind` 把"展开时守卫做什么"
    // 和"展开后看结果"这对矛盾拧开——panic 在展开路径上把 `_sweep::drop` 跑完（那一刻
    // `std::thread::panicking()` 仍为真），控制权再交回下面的断言。
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _sweep = StrayGuard::orphan_sweep(s.rclone_bin.clone(), my_pid_path.clone(), &[]);
        panic!("故意的展开：这条用例要验的就是守卫在这次 Drop 里动不动手");
    }));
    assert!(outcome.is_err(), "夹具没有 panic = 端口档的展开路径根本没被走到，这条空转");

    // 关键的一半：那个 daemon 还活着（被截断过的话这条必然失败）。
    assert!(!rclone_deck_lib::rcd::command_line_of(daemon).is_empty(),
        "端口档在展开路径上截断了另一个面板还在用的 daemon——归属判据（fixture 路径）没了");
    assert_eq!(rclone_deck_lib::rcd::port_listener_pid(p), Some(daemon),
        "同上：它还该占着 {p}，端口都没放掉就是被杀了");

    // 测试自己收尾：记录档按 foreign 目录里那条记录把它带走（`panel_pid` = 本进程，不是同行）。
    drop(reap);
    assert!(rclone_deck_lib::rcd::command_line_of(daemon).is_empty(),
        "收尾没把本用例起的这个 daemon 带走——它不该留给下一条用例");
    assert!(pidfile::read(&foreign_pid_path).is_none(), "收尾应连自己那份记录一起删掉");
}

/// 裁定 R46 item 4 的**反向证人**：端口上挂的是**本用例自己**起的、记录又**确实读不到**时，
/// 展开必须仍能收掉它——否则收紧归属判据的同时，也就悄悄把 R45 item 2 的初衷删了
/// （"pid 文件写不下去/读不到的那条路只剩端口档"）。两条证人都得有，只有一条等于没钉住。
///
/// 裁定 R47 item 2 之后这条同时是"端口档对所有守卫点都开着"的证据：守卫是用**记录档那个构造器**
/// 建的（R45 时期它写死 `port_leg = false`，这条会直接红），谁再把端口档对哪条用例关掉，这条红。
///
/// 夹具：真起一个 daemon（`log_file` 与 `pid_path` 都在本用例自己的 TempDir 里），然后把那条
/// pid 记录删掉——守卫的档一因此注定落空，端口档才是它唯一的依据。
/// 这条同时是 fixture 判据的**正向**证据：判据写歪一点（取错父目录、或按子串而不是按分量比），
/// 这里就收不到东西，`{p}` 上留下 daemon → 本条变红。裁定 R47 item 1 的证人也在这一条上：
/// 把守卫的期望 exe 换成一个不存在的路径，端口档就不许再认领它 → 本条变红。
#[test]
fn a_port_tier_unwind_still_reaps_the_daemon_this_case_started() {
    let p = p();
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("rcd.pid");
    let s = spec(dir.path());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all()
        .build().expect("夹具 runtime");
    let daemon = rt.block_on(async {
        let h = RcdSupervisor::start(s.clone()).await.unwrap();
        h.pid // 句柄在这里丢掉：daemon 留在端口上，正是"用例被打断"的残局
    });

    // 夹具前提：daemon 活着、占着端口、记录被抹掉（= 档一落空），而它带着本用例的目录。
    assert!(!rclone_deck_lib::rcd::command_line_of(daemon).is_empty(), "夹具：daemon 没起来，这条空转");
    assert_eq!(rclone_deck_lib::rcd::port_listener_pid(p), Some(daemon), "夹具：它正占着 {p}");
    std::fs::remove_file(&pid_path).expect("夹具：把 pid 记录删掉，让档一注定落空");
    assert!(pidfile::read(&pid_path).is_none(), "夹具：记录应已读不到");

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _sweep = StrayGuard::orphan_sweep(s.rclone_bin.clone(), pid_path.clone(), &[]);
        panic!("故意的展开：这条用例要验的就是守卫在这次 Drop 里收不收自己那个 daemon");
    }));
    assert!(outcome.is_err(), "夹具没有 panic = 端口档的展开路径根本没被走到，这条空转");

    // 展开之后那个 daemon 必须已经没了。先兜底再断言：这条正在变红的形状里不许顺手给
    // 下一条用例搁浅一个 daemon（本文件开头的规则）。
    let still_alive = !rclone_deck_lib::rcd::command_line_of(daemon).is_empty();
    if still_alive {
        assert!(rclone_deck_lib::rcd::kill_pid(daemon), "收尾兜底也没能终结 PID {daemon}");
    }
    assert!(!still_alive,
        "端口档把**本用例自己**起的 daemon 放走了——R46 item 1 的归属判据把它也挡掉了，\
         等于把 R45 item 2 的初衷修没");
    assert_ne!(rclone_deck_lib::rcd::port_listener_pid(p), Some(daemon), "被收掉的 daemon 不该还占着端口");
}

/// 裁定 R47 item 2（复审 F-4 点名的那个窗口）的**专用证人**：R45 给四条记录档用例写死了
/// `port_leg = false`，于是"spawn 之后、`pidfile::write` 之前"这个窗口里 panic 就没人收尾；
/// 而 #11（`a_credential_wipe_that_cannot_finish_is_reported_not_swallowed`）的 `start()`
/// 跑在后台任务里，这个窗口是真的。收窄当时唯一的理由"端口档没有可靠归属判据"已经被 R46 的
/// fixture 路径判据消灭，所以端口档对**所有**守卫点都开。这条量的就是那件新能力：
/// **守卫按记录档那个构造器建、`start()` 之前建好，daemon 的记录压根没落下（这里用"起完之后
/// 删掉"来等价复现"那次写入还没发生"），panic 展开时它必须收得掉。**
///
/// 今天（R47 之前）这条做不到：同一个构造器 `port_leg = false`，展开时端口档直接 return，
/// daemon 搁浅 → 下面"它还该被收掉"那条断言红。也就是说本条正是那次收窄的破坏一次证人。
/// 收尾有兜底：变红时不许顺手给下一条用例留一个 daemon（本文件开头的规则）。
#[test]
fn a_record_tier_guard_reaps_when_the_pid_record_never_landed() {
    let p = p();
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("rcd.pid");
    let s = spec(dir.path());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all()
        .build().expect("夹具 runtime");

    // 守卫在 `start()` **之前**建好（#11 的形状就是这一条），并且活在那个展开作用域里。
    let mut spawned: Option<u32> = None;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _sweep = StrayGuard::orphan_sweep(s.rclone_bin.clone(), pid_path.clone(), &[]);
        let daemon = rt.block_on(async { RcdSupervisor::start(s.clone()).await.unwrap().pid });
        spawned = Some(daemon);
        // 夹具：把记录抹掉 = "那次写入还没发生"。档一因此注定落空，端口档是唯一依据。
        std::fs::remove_file(&pid_path).expect("夹具：删掉 pid 记录");
        panic!("故意的展开：验记录档守卫在档一落空时收不收得掉自己那个 daemon");
    }));

    // 那次 panic 必须是我们自己那次"故意的展开"，不能是夹具里的断言（否则下面的红会说谎）。
    let err = outcome.err().expect("夹具没有 panic = 展开路径根本没被走到，这条空转");
    let msg = err.downcast_ref::<&str>().copied().unwrap_or("");
    assert!(msg.contains("故意的展开"), "夹具里另有一条断言先 panic 了，这条验的不是端口档：{msg}");

    let daemon = *spawned.as_ref().expect("夹具：daemon 该已经起来");
    // 关键的一半：它被收掉了。和 `a_port_tier_unwind_still_reaps_...` 一样先兜底再断言：
    // 这条正在变红的形状里不许顺手给下一条用例搁浅一个 daemon（本文件开头的规则）。
    let still_alive = !rclone_deck_lib::rcd::command_line_of(daemon).is_empty();
    if still_alive {
        assert!(rclone_deck_lib::rcd::kill_pid(daemon), "收尾兜底也没能终结 PID {daemon}");
    }
    assert!(!still_alive,
        "记录档守卫在档一落空时把**本用例自己**的 daemon 放走了——端口档又对哪条用例关掉了");
    assert_ne!(rclone_deck_lib::rcd::port_listener_pid(p), Some(daemon), "被收掉的 daemon 不该还占着端口");
    assert!(pidfile::read(&pid_path).is_none(), "收尾应连记录一起清掉（这里本来就没有记录）");
}

/// 裁定 R42 item 2 的证人：**擦除凭据回显失败不许被 `let _ =` 吞掉**。
/// 夹具：把启动放进后台任务（假 flag → 20s 健康窗口给了稳定的注入时间），等 rclone 自己把
/// DEBUG 回显写进 `rcd-stderr.log`，就在监管器走到那次擦除之前把它**改名**并在原位置放一个
/// **目录**——`std::fs::write(path, "")` 打不开一个目录，那次擦除因此必然失败
/// （std 在 Windows 没有稳定的"设只读位"API，改名 + 目录是唯一纯 std 的确定性注入）。
/// - 实现正确：拿到 `RcdError::Io`，话术里"擦不掉"和"启动本身的错误"两个都在（谁也不顶掉谁）。
/// - 破坏一次 #2（把那次擦除恢复成 `let _ =`）：拿到的是 `NotHealthy`，本条变红——
///   而磁盘上那份明文回显就这么静默留下了。
#[tokio::test]
async fn a_credential_wipe_that_cannot_finish_is_reported_not_swallowed() {
    let dir = tempfile::tempdir().unwrap();
    let (user, pass) = random_credentials();
    let mut s = spec(dir.path());
    s.flags.push("--this-flag-does-not-exist".into());
    let stderr_path = dir.path().join("rcd-stderr.log");
    let moved_path = dir.path().join("rcd-stderr.echo");
    // 裁定 R45 item 2：启动在后台任务里，用例手上没有句柄（拿到的一直是 `Err`）。
    // 下面 `join.await.expect(...)` 之前那两条断言（回显是否落盘、注入是否成功）一 panic，
    // 那个在飞的 `start()` 就留下了记录里那个进程——无句柄档先按 pid 记录收尾。
    // 裁定 R47 item 2 特别点到这一条：`start()` 跑在后台任务里，"spawn 之后、`pidfile::write`
    // 之前"那个窗口是真的，而 R45 时期这条的守卫写死了 `port_leg = false`，panic 在窗口里就没人
    // 收尾。端口档现在对所有守卫点开着（证人：`a_record_tier_guard_reaps_when_the_pid_record_never_landed`）。
    let _sweep = StrayGuard::orphan_sweep(s.rclone_bin.clone(), dir.path().join("rcd.pid"), &[]);

    let join = tokio::spawn({
        // 任务要 'static，所以把这一对克隆进去；断言用的是外面那一对（值相同）。
        let (task_user, task_pass) = (user.clone(), pass.clone());
        async move { RcdSupervisor::start_with_credentials(s, &task_user, &task_pass).await }
    });

    // 注入前先确认那份回显真的落盘了：否则"擦除失败"是没发生的事故，断言就成了空转。
    let mut echoed = false;
    for _ in 0..150 {
        if std::fs::metadata(&stderr_path).map(|m| m.len() > 0).unwrap_or(false) {
            echoed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(echoed, "rcd-stderr.log 里没有出现 rclone 的启动回显，这条断言就是空转");
    std::fs::rename(&stderr_path, &moved_path).expect("注入：把 stderr 日志改名");
    std::fs::create_dir(&stderr_path).expect("注入：在原位置放一个目录，让那次擦除必然失败");

    let result = join.await.expect("启动任务被 panic 打断");

    match result {
        Err(RcdError::Io(msg)) => {
            // 裁定 R47 item 3 的同一形状：这两句排在下面那两条"不含凭据"断言**之前**，
            // 把 `msg` 插进话术等于在生产话术回归的那一刻自己先把详情贴进终端。
            // 判定不动（还是含不含那两个词），只报长度。
            assert!(msg.contains("清空"), "Io 话术没指到那次擦除（详情 {} 字符，不打印）", msg.chars().count());
            assert!(msg.contains("启动本身的错误"), "原来的启动错误被擦除失败顶掉了（详情 {} 字符，不打印）", msg.chars().count());
            // 这条新增的话术同样归"交出去的错误里不许有凭据"管（裁定 R37 的口径）。
            assert!(!msg.contains(&pass), "Io 话术把口令交了出去");
            assert!(!msg.contains(&user), "Io 话术把用户名交了出去");
        }
        Err(other) => panic!("期望 Io（擦不掉就必须报出来），实际是变体 {}", err_variant(&other)),
        Ok(_) => panic!("期望 Io（擦不掉就必须报出来），实际是 Ok（那次擦除居然成功了）"),
    }
    assert!(
        std::fs::metadata(&stderr_path).map(|m| m.is_dir()).unwrap_or(false),
        "注入没生效（原位置被擦除重建成了文件）= 上面那条 Io 断言是空转"
    );

    // 夹具收尾：改名出去的那一份**带着明文回显**，不能靠 TempDir 顺手删，当场删掉并确认。
    std::fs::remove_dir(&stderr_path).expect("夹具收尾：删掉注入用的目录");
    std::fs::remove_file(&moved_path).expect("夹具收尾：那份明文回显不能留在磁盘上");
    assert!(!moved_path.exists());
}

#[tokio::test]
async fn an_orphaned_rcd_is_reaped_not_reused() {
    let p = p();
    let dir = tempfile::tempdir().unwrap();
    let s = spec(dir.path());
    let pid_path = dir.path().join("rcd.pid");
    // 裁定 R45 item 1 + item 2：和 `a_dead_panels_record_is_still_reaped` 同一个交界形状——
    // A 被**故意**丢成孤儿（`drop(a)` 就是崩溃模拟），句柄形态收不了它，只能靠 pid 记录档
    // （裁定 R47 item 2：记录读不到时端口档接着兜，两处守卫点都两档全开）。
    let _sweep = StrayGuard::orphan_sweep(s.rclone_bin.clone(), pid_path.clone(), &[]);

    let a = RcdSupervisor::start(s.clone()).await.unwrap();
    let (a_pid, a_version) = (a.pid, a.rclone_version.clone());
    drop(a); // 不调 shutdown：模拟面板崩溃，rcd 留在原地
    // 注意记录里的 `panel_pid` 这里就是**本进程**（裁定 R42 item 3 的自例外）：
    // 同进程造不出"已经死掉的前一个面板"，那条路归 `a_dead_panels_record_is_still_reaped`，
    // "另一个面板还活着所以绝不许杀"归 `a_live_sibling_panels_daemon_is_never_reaped`。

    assert!(!port_free(p).await, "孤儿 rcd 应该还占着端口，否则这个测试什么都没测");

    let b = RcdSupervisor::start(s.clone()).await.unwrap();
    let (b_pid, b_version) = (b.pid, b.rclone_version.clone());
    let mut guard = StrayGuard::new(b, s.rclone_bin.clone(), pid_path.clone(), &[]);
    assert_ne!(b_pid, a_pid, "复用了孤儿 rcd——可它的凭据只在已死进程的内存里，我们根本连不上");
    assert_eq!(b_version, a_version);
    assert!(guard.client().core_version().await.is_ok(), "重开之后的 rcd 必须能用新凭据说话");
    guard.release().shutdown().await.unwrap();
}

#[tokio::test]
async fn a_failed_startup_reports_rclone_own_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("rcd.pid");
    // 裁定 R45 item 1 数出来的补集（不在 R45 那份名单里，但同样 spawn 过真进程）：
    // 非法 flag 让 rclone 自己就退了，可 future 在 `start()` 里被打断时记录已经在盘上，
    // 守卫按记录收尾；记录还没落下的那段窗口由端口档兜（裁定 R47 item 2）。
    // 正常路径这条永远不命中（出口自己删了记录）。
    let mut s = spec(dir.path());
    s.flags.push("--this-flag-does-not-exist".into());
    let exe = s.rclone_bin.clone();
    let _sweep = StrayGuard::orphan_sweep(exe.clone(), pid_path.clone(), &[]);

    match RcdSupervisor::start(s).await {
        Err(RcdError::NotHealthy(_, detail)) => {
            assert!(!detail.is_empty(), "光说'起不来'没用，要带 rclone 自己的报错");
        }
        Err(other) => panic!("期望 NotHealthy，实际是变体 {}", err_variant(&other)),
        Ok(h) => {
            let _h = StrayGuard::new(h, exe, pid_path, &[]);
            panic!("非法 flag 居然起来了");
        }
    }
}

/// 裁定 R39 的证人：**失败启动那个 `redact_credentials` 调用点必须有见证**。
/// 上一条用例走的是 `RcdSupervisor::start`——凭据是监管器自己生成的，测试根本不知道值，
/// 于是它只能断言 `!detail.is_empty()`：**把失败分支里的脱敏调用整段删掉，全套 143 条仍然全绿**。
/// 那条调用点没有证人，就等于没有那条调用点（R37 立的不变量"错误详情里不许带凭据"没人守着）。
///
/// 这条把形状补成：`start_with_credentials(spec, user, pass)`（测试知道确切值）
/// + 一个必定让 rclone 非零退出的假 flag（实测 exit=2）+ `log_level_debug: true`（沿用 `spec()`
/// 夹具的设置，rclone 因此在退出前把环境里的凭据原样回显到 stderr：
/// `DEBUG : Setting rc_pass="<口令>" from environment variable RCLONE_RC_PASS`，本机在 v1.75.1
/// 上量到四行启动回显、其中两行含口令）。失败分支把那份 stderr 读进内存拼成 `NotHealthy` 的详情，
/// 所以**交出去的字节里本来就有口令**——这条断言因此是真在跑那条脱敏代码，不是自说自话。
/// 破坏一次（删掉 `redact_credentials` 调用）这条必须变红；若它仍绿，说明夹具没走到那条路，
/// 要修夹具而不是修断言。
#[tokio::test]
async fn a_failed_startup_detail_carries_no_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let (user, pass) = random_credentials();
    // 同 `a_failed_startup_reports_rclone_own_stderr`：这条也 spawn 过真进程（假 flag），
    // 守卫先走 pid 记录档、记录没落下时走端口档（裁定 R47 item 2）；正常路径不命中。
    let mut s = spec(dir.path());
    s.flags.push("--this-flag-does-not-exist".into());
    let exe = s.rclone_bin.clone();
    let _sweep = StrayGuard::orphan_sweep(exe.clone(), dir.path().join("rcd.pid"), &[]);

    match RcdSupervisor::start_with_credentials(s, &user, &pass).await {
        Err(RcdError::NotHealthy(_, detail)) => {
            // 先钉住详情不是空的（否则下面两条 `!contains` 是对空串说话，永远成立 = 空转）。
            assert!(!detail.is_empty(), "详情为空的话这两条脱敏断言就是空转");
            // 断言里不带 `detail` 的原文：话术本身可能含凭据（这正是要防的东西），
            // 失败时只报"哪一侧命中"，把值留在内存里而不是贴进终端/CI 日志。
            assert!(!detail.contains(&pass), "启动失败的错误详情把口令交了出去（脱敏没跑或被删）");
            assert!(!detail.contains(&user), "启动失败的错误详情把用户名交了出去（脱敏没跑或被删）");
        }
        Err(other) => panic!("期望 NotHealthy，实际是变体 {}", err_variant(&other)),
        Ok(h) => {
            let _h = StrayGuard::new(h, exe, dir.path().join("rcd.pid"), &[]);
            panic!("非法 flag 居然起来了");
        }
    }
}

/// 同一个调用点的纯函数侧：嵌套密钥（一个是另一个的子串）按给的顺序替换会留下尾巴，
/// 掩码串必须与 Task 8 落盘日志用的 `[REDACTED]` 一致（两种写法进同一条人眼读的输出流
/// = 读的人分不清哪个是被抹掉的值）。
#[test]
fn redacting_nested_secrets_is_longest_first_and_uses_the_shared_mask() {
    let nested = ["deck-abc", "deck-abcdef"];
    // 故意按"短的在前"递给 pub API：函数自己必须保证顺序。
    let red = redact_credentials("rc-user=deck-abc pass=deck-abcdef", &nested);
    assert!(!red.contains("deck-"), "子串密钥没擦干净：{red}");
    assert_eq!(red.matches("[REDACTED]").count(), 2, "{red}");
    assert_eq!(red, "rc-user=[REDACTED] pass=[REDACTED]", "留了尾巴或换了掩码写法：{red}");

    // 空串仍须整条跳过：`replace("", x)` 会把每个字符之间都插一遍掩码。
    assert_eq!(redact_credentials("plain line", &[""]), "plain line");
}

/// 裁定 R44 的前提证人：正典 `redact_credentials` 的 `>= 4` 下限（从 `tasklog::scrub`
/// 并进正典，两处从此共用一个数）**只有一个立足点**——走到那里的密钥全部来自
/// `random_credentials()`（`deck-<8hex>` = 13 字符，口令 32 位 hex），不是因为短密钥不存在。
/// 所以那个前提必须钉在生成器上而不是钉在注释里：谁改了 `random_credentials()` 的格式，
/// 或者有调用方开始往 `TaskLog::create` / `redact_credentials` 递一个短口令，这条先红，
/// 而不是让"擦过密"的输出继续假装安全。
/// 后半条（互不为子串）今天由降序替换兜住，这里按裁定**重复断言一次**，让"没有嵌套"
/// 成为看得见的前提而不是隐含的运气。
/// 失败话术只报长度：随机凭据也不该被原样印进终端/CI 日志。
#[test]
fn the_generated_credentials_clear_the_scrub_floor_and_never_nest() {
    // 抽一批而不是一次：这条断言的是生成器的形状，不该靠某一次的运气绿。
    for _ in 0..256 {
        let (user, pass) = random_credentials();
        let (u_len, p_len) = (user.chars().count(), pass.chars().count());
        assert!(u_len >= 4, "用户名只有 {u_len} 字符，会被正典的 4 字符下限原样放过");
        assert!(p_len >= 4, "口令只有 {p_len} 字符，会被正典的 4 字符下限原样放过");
        assert!(!pass.contains(user.as_str()), "用户名成了口令的子串：{u_len}/{p_len}，降序替换之外还得加一条规则");
        assert!(!user.contains(pass.as_str()), "口令成了用户名的子串：{p_len}/{u_len}，同上");

        // 下限确实作用在**这一对**值上：真递进正典就必须两处都消失（不是只验长度）。
        // 失败话术**不印 `red`**：这条一旦红，那段文本里就还带着凭据本身，印出来等于第二次泄漏。
        let red = redact_credentials(&format!("rc-user={user} rc-pass={pass}"), &[user.as_str(), pass.as_str()]);
        assert_eq!(red.matches("[REDACTED]").count(), 2, "这对随机凭据没被正典成对擦掉（掩码处数不是 2）");
    }
}

/// 旗标白名单管的是 argv，管不到进程环境：rclone 把 `RCLONE_<FLAG>` 当成旗标默认值，
/// 操作员 shell 里一句 `export RCLONE_COPY_LINKS=1` 就能让面板"明明没写 -L"却开始跟随
/// junction，桶里多出本地扫描结构上不可能产生的 key —— 对 prune 就是假孤儿。
/// 这条测试不走打桩：起真 rcd，提交真任务，只是两端都是临时目录，零云端。
#[tokio::test]
async fn a_stray_rclone_link_env_does_not_reach_the_jobs() {
    std::env::set_var("RCLONE_COPY_LINKS", "1");
    let dir = tempfile::tempdir().unwrap();
    let s = spec(dir.path());
    let h = RcdSupervisor::start(s.clone()).await.unwrap();
    // 守卫接手这个 daemon 和这个环境变量：下面任何一条断言 panic（或者提前 return），
    // 那个号上都不会留下真 daemon，进程环境里也不会留下 RCLONE_COPY_LINKS（裁定 R42 item 4）。
    let mut guard = StrayGuard::new(h, s.rclone_bin.clone(), dir.path().join("rcd.pid"),
        &["RCLONE_COPY_LINKS"]);

    // src 里放一个指回 src 自己的 junction，和 build_command 那侧的夹具同一形状
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("sub/a.webp"), b"x").unwrap();
    let mk = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(src.join("link")).arg(&src)
        .output().unwrap();
    assert!(mk.status.success(), "junction 夹具建不出来：{:?}", String::from_utf8_lossy(&mk.stderr));

    let dst = dir.path().join("dst");
    std::fs::create_dir_all(&dst).unwrap();
    let ack = guard.client().submit_async("sync/copy",
        &[("srcFs", src.display().to_string()), ("dstFs", dst.display().to_string())], false)
        .await.unwrap();
    // 有限次轮询，不写无界等待：任务卡住要让测试失败并**报出最后一次状态**，而不是永远挂着。
    // 轮询本身失败**不等于**任务失败——记下原因、继续到预算用完，最后由 `finished` 那条断言
    // 把 `last` 摊出来（这一段当初改回 `job/status` 时把容错和"报出最后一次状态"一起丢了，
    // 而上面的话术还承诺着它：注释不许描述测试已经没有的行为。R39 的 carry-over Minor）。
    //
    // 这里轮的是 `job/status`（不是当初绕路的 `job/list`）：**裁定 R35** 把这条变可能了。
    // R35 之前 `into_value` 会把成功任务里那个 `"error": ""` 也折成传输故障，于是
    // `job_status` 对着真 daemon 永远返回 Err，本测试当年只能改轮没有 error 键的 `job/list`
    // 才把活干完；R35 让 `job/status` 的 200+error 作为载荷交回来后，直接读 `finished` 就够了
    // ——`job/status` 才是这条用例该用的源（`job/list` 只说"结束没结束"，说不出为什么没结束）。
    let mut finished = false;
    let mut last = String::from("一次都没读到 job/status");
    for _ in 0..60 {
        match guard.client().job_status(ack.jobid).await {
            Ok(st) => {
                if st.finished { finished = true; break; }
                last = format!("还没 finished：{st:?}");
            }
            // 容错：读失败只记录，不提前 unwrap 掉整个测试（那等于把"任务卡住"报成"轮询报错"）。
            Err(e) => last = format!("job/status 读失败：{e}"),
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert!(finished, "本地对本地复制 12 秒还没结束：{last}");

    assert!(dst.join("sub").join("a.webp").is_file(), "真文件反倒没过去");
    // 漏了就要么多一个 link/ 目录（跟随），要么多一个 link.rclonelink（当链接对象搬走）——
    // 两种都是本地扫描结构上不可能产生的 key。
    assert!(!dst.join("link").exists() && !dst.join("link.rclonelink").exists(),
        "RCLONE_COPY_LINKS 漏进了子进程：{:?}", dir_path_listing(&dst));

    // 正常路径：句柄交回测试自己 `shutdown()`（它会等退出 + 确认端口释放），守卫从此不插手；
    // 环境变量由守卫的 `Drop` 恢复，所以这里**不再**手写 `remove_var`——展开路径也覆盖到了。
    let h = guard.release();
    h.shutdown().await.unwrap();
}

/// 裁定 R36：环境清扫（R26 那条 `RCLONE_<旗标>` 默认值通道）**不许**顺手清掉操作员配置文件的
/// 解密口令。`rclone_env_is_scrubbed` 是被测的那个判据，`build_command` 的循环是它唯一的调用点。
/// 这里断言三件事：
/// - 两个保留名（`RCLONE_CONFIG`、`RCLONE_CONFIG_PASS`）都不被清——口令被清会把加密 conf 的
///   操作员拖成 `NotHealthy` 而不是可用 daemon；
/// - 一个陈旧的 `RCLONE_COPY_LINKS=1` 仍被清——那才是 R26 立这条通道要拦的东西（行为侧的活证据
///   由 `a_stray_rclone_link_env_does_not_reach_the_jobs` 用真 rcd 钉住）；
/// - rc 凭据那一对（`RCLONE_RC_USER`/`RCLONE_RC_PASS`）**判定为"要清"**（方向钉死为 true）：
///   监管器随后自己 `cmd.env` 设随机凭据，继承来的陈旧一对绝不能赢。
#[test]
fn the_env_scrub_keeps_the_config_decryption_passphrase_but_still_clears_flag_defaults() {
    // 保留：去哪找配置文件，以及它的解密口令。
    assert!(!rclone_env_is_scrubbed("RCLONE_CONFIG"), "RCLONE_CONFIG 被清 = 找不到 remote 配置");
    assert!(
        !rclone_env_is_scrubbed("RCLONE_CONFIG_PASS"),
        "RCLONE_CONFIG_PASS 被清 = 加密 rclone.conf 的操作员拿到 NotHealthy，而不是可用 daemon（裁定 R36）"
    );

    // 仍清除：旗标默认值通道。
    assert!(rclone_env_is_scrubbed("RCLONE_COPY_LINKS"), "RCLONE_COPY_LINKS 必须仍被清");

    // 钉死方向：rc 凭据这对是要清的（true）——不是保留。
    assert!(rclone_env_is_scrubbed("RCLONE_RC_USER"), "继承来的 RCLONE_RC_USER 必须被清");
    assert!(rclone_env_is_scrubbed("RCLONE_RC_PASS"), "继承来的 RCLONE_RC_PASS 必须被清");

    // 与 RCLONE_ 无关的键从不由这条通道处理（返回 false 是"不在作用域"，不是"保留下来传给子进程"）。
    assert!(!rclone_env_is_scrubbed("PATH"));
    assert!(!rclone_env_is_scrubbed("RCLONE"));
}

/// 裁定 R39（R26 的漏洞）：**判据必须不区分大小写**，因为它的两侧都不是区分大小写的——
/// Windows 的环境变量查找不区分，rclone 的 `RCLONE_<FLAG>` 默认值通道也不区分。
/// 按前缀大小写敏感地清，一条从 Git Bash 继承来的 `rclone_copy_links=1` 就能躲过清扫，
/// 把旗标默认值带进 daemon（症状就是桶里多出 `link/…` 这种假孤儿），正是 R26 要拦的事。
/// 这条钉三件事：小写的旗标默认值**判定为要清**；两个保留名在**任意大小写**下都不被清
/// （清掉 `Rclone_Config_Pass` = 拿着加密 rclone.conf 的操作员拿到 NotHealthy，裁定 R36）；
/// 混排大小写的要清名同样被抓到——防止有人把修复写成"只针对全小写"的特例。
#[test]
fn the_env_scrub_judgement_is_case_insensitive_like_windows_and_rclone() {
    // 躲过大小写敏感前缀匹配的那条真实通道：小写的旗标默认值必须仍被判为"要清"。
    assert!(rclone_env_is_scrubbed("rclone_copy_links"),
        "小写的 rclone_copy_links 没被判为要清 = RCLONE_<旗标> 默认值通道漏了（裁定 R26 的洞）");
    assert!(rclone_env_is_scrubbed("Rclone_Copy_Links"), "混排大小写也必须被抓到");
    assert!(rclone_env_is_scrubbed("rclone_dump"), "任意小写旗标默认值都要被清");

    // 两个保留名在任意大小写下都不被清（R36）。
    assert!(!rclone_env_is_scrubbed("rclone_config"), "小写的 RCLONE_CONFIG 也被清 = 找不到配置文件");
    assert!(!rclone_env_is_scrubbed("Rclone_Config"), "混排大小写的 RCLONE_CONFIG 也被清");
    assert!(!rclone_env_is_scrubbed("rclone_config_pass"),
        "小写的 RCLONE_CONFIG_PASS 也被清 = 加密 conf 的操作员拿到 NotHealthy（裁定 R36）");
    assert!(!rclone_env_is_scrubbed("RCLONE_CONFIG_PASS"), "大写那份照旧必须保留");

    // 凭据那一对在任何大小写下都判定为"要清"（方向钉死为 true）：监管器自己 cmd.env 设随机值。
    assert!(rclone_env_is_scrubbed("rclone_rc_pass"), "继承来的小写 rc 凭据也必须被清");

    // 反向哨兵：与 RCLONE_ 无关的键不因大小写而变成"要清"。
    assert!(!rclone_env_is_scrubbed("path"));
    assert!(!rclone_env_is_scrubbed("Path"));
}

/// 失败时把目标目录摊开打印，省得下一轮再猜。
fn dir_path_listing(root: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p.clone()); }
            out.push(p.strip_prefix(root).unwrap_or(&p).display().to_string());
        }
    }
    out
}
