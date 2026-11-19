//! rclone `rcd` 子进程的监管：拉起、健康检查、pid 文件、孤儿回收、收尾。
//!
//! 三条实测根因都落在这一层（spec §3.1）：
//! 1. **零控制台窗口**——rclone 是控制台程序，面板是 GUI 子系统程序，不设
//!    `CREATE_NO_WINDOW` 就闪窗；本机 `QuickEdit=1`，误点一下就把子进程冻死且无报错。
//! 2. **凭据绝不进 argv**——`--rc-pass` 上命令行会同时泄进 rclone 自己的 DEBUG 日志
//!    和 `Win32_Process.CommandLine`（probe 有对照组）。只走 `RCLONE_RC_USER` /
//!    `RCLONE_RC_PASS` 环境变量。
//! 3. **后台归属**——每个配置家持有独占文件锁。启动时核验旧记录，用同一 Windows
//!    进程句柄确认身份、回收孤儿和等待退出。选用新端口也要处理上一份记录。

pub mod pidfile;
pub(crate) mod process;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

use crate::config::{jobs::Defaults, AppConfig};
use crate::rc::RcClient;

/// GUI 子系统的父进程拉控制台子进程时必须显式设这一位，否则会闪出窗口。
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const HEALTH_TIMEOUT: Duration = Duration::from_secs(20);
const HEALTH_INTERVAL: Duration = Duration::from_millis(150);
const REAP_INTERVAL: Duration = Duration::from_millis(200);
const REAP_ATTEMPTS: usize = 50;
/// 一次"端口有没有人听"的探测最多愿意等多久。**它是 `reclaim` 那段耗时的另一半**：
/// 循环的名义时长是 `REAP_ATTEMPTS × REAP_INTERVAL`，但每轮还要付一次这个超时
/// （本机对一个没人听的回环端口发 connect 要 ~2s 才被拒，于是这 300ms 经常吃满），
/// 所以真实上限是 `REAP_ATTEMPTS × (REAP_INTERVAL + PROBE_TIMEOUT)` = 25s，不是 10s。
/// 报出去的数字一律用 `Instant` 实测（裁定 R42 item 5），这个常量只用来把上限写进话术。
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Debug, Error)]
pub enum RcdError {
    #[error("端口 {port} 被别的进程占着：进程号 {pid}（{name}）。面板不静默换端口，请先关掉它")]
    PortBusyByForeignProcess { port: u16, pid: u32, name: String },
    #[error("端口 {port} 上是残留的 rcd（PID {pid}），但回收失败：{reason}")]
    OrphanNotReapable { port: u16, pid: u32, reason: String },
    #[error("无法启动 rclone rcd：{0}")]
    SpawnFailed(String),
    #[error("rcd 在 {0}s 内没有响应 rc 请求：{1}")]
    NotHealthy(u64, String),
    #[error("pid 文件读写失败：{0}")]
    Io(String),
    #[error("配置目录 {0} 的后台已由另一个面板管理，请在那个面板操作")]
    HomeBusy(PathBuf),
}

impl serde::Serialize for RcdError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct RcdSpec {
    pub rclone_bin: PathBuf,
    pub port: u16,
    pub flags: Vec<String>,
    pub exclude: Vec<String>,
    /// 指到**哪一份 rclone 配置**（`RCLONE_CONFIG`）。`None` = 用 rclone 自己的默认位置，
    /// 生产路径与 Task 7 的测试都是 `None`。需要 `Some(临时 conf)` 的只有 Task 9 的
    /// gate/prune 夹具：见裁定 R30——那两个任务的 `dst` 必须是具名 remote
    /// （`remote_for_list()` 会拒绝本地路径），而本机真实的 conf 里只有一个有 `img` 桶权限的
    /// `r2`，测试绝不能拿它当夹具。
    pub rclone_config: Option<PathBuf>,
    pub log_file: Option<PathBuf>,
    pub pid_path: PathBuf,
    pub log_level_debug: bool,
}

impl RcdSpec {
    /// `flags` / `exclude` 只能走 rcd 启动参数：实测每次调用的 `_config` 只认 `dryRun`，
    /// 其余键 rclone 会静默忽略并返回 success=true（spec §3.1.1）。
    pub fn from_config(cfg: &AppConfig, defaults: &Defaults) -> Self {
        Self {
            rclone_bin: cfg.rclone_bin.clone(),
            port: cfg.port,
            flags: defaults.flags.clone(),
            exclude: defaults.exclude.clone(),
            // 生产永远用 rclone 自己的默认配置位置；只有测试会把它换成临时 conf。
            rclone_config: None,
            log_file: Some(cfg.log_dir.join("rcd.log")),
            pid_path: cfg.pid_path.clone(),
            log_level_debug: false,
        }
    }

    fn stderr_path(&self) -> PathBuf {
        match &self.log_file {
            Some(p) => p.with_file_name("rcd-stderr.log"),
            None => std::env::temp_dir().join("rclone-deck-rcd-stderr.log"),
        }
    }
}

pub fn random_credentials() -> (String, String) {
    use rand::RngExt;
    let mut rng = rand::rng();
    let user = format!("deck-{:08x}", rng.random::<u32>());
    let pass = format!("{:032x}", rng.random::<u128>());
    (user, pass)
}

pub struct RcdHandle {
    pub client: RcClient,
    pub port: u16,
    pub pid: u32,
    /// rcd 实例级标识，崩溃后对账用；某些 rclone 版本不在 `core/version` 里返回它，那时为空串，
    /// 对账退回 pid + start_time。
    pub execute_id: String,
    pub rclone_version: String,
    child: Child,
    pid_path: PathBuf,
    secrets: Vec<String>,
    _home: Option<DaemonHome>,
}

/// A stable file path serializes daemon ownership across panel processes.
/// The OS releases the lock when the owning handle is closed, including a panel crash.
pub(crate) struct DaemonHome { _file: std::fs::File }

impl DaemonHome {
    pub(crate) fn claim(pid_path: &Path) -> Result<Self, RcdError> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        let lock_path = pid_path.with_extension("lock");
        if let Some(dir) = pid_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| RcdError::Io(format!("{}: {e}", dir.display())))?;
        }
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&lock_path).map_err(|e| RcdError::Io(format!("{}: {e}", lock_path.display())))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) =>
                Err(RcdError::HomeBusy(lock_path.parent().unwrap_or(&lock_path).to_path_buf())),
            Err(std::fs::TryLockError::Error(e)) =>
                Err(RcdError::Io(format!("无法锁住 {}：{e}", lock_path.display()))),
        }
    }
}

impl RcdHandle {
    /// 交给 Task 8 的 `TaskLog` 做精确值脱敏用。这些串本来就只活在内存里
    /// （`RcClient` 的 basic-auth 头里也有），多存一份不扩大暴露面。
    pub fn secrets(&self) -> &[String] {
        &self.secrets
    }

    /// 只给测试用的紧急收尾（裁定 R42 item 4）：`start_kill` **不 await**，所以它能在 `Drop` 里
    /// 被调用——而 `shutdown()` 需要 await，panic 展开时那个 future 已经没有 runtime 可等了，
    /// 于是真 daemon 会搁浅在生产端口 默认端口 上。套件按 `--test-threads=1` 跑，一次搁浅会把后面
    /// 每一条 rcd 用例连锁带红（连锁红会掩盖真正的病因，比泄漏一个进程更贵）。
    ///
    /// 这里只保证"进程已被终结"：不等退出、不删 pid 文件、不确认端口释放——那需要 runtime，
    /// 而调用方（测试守卫）用阻塞轮询自己收。返回 `Result` 而不是 `let _ =`：守卫会把失败说出来。
    #[cfg(feature = "test-helpers")]
    pub fn kill_now(&mut self) -> std::io::Result<()> {
        self.child.start_kill()
    }

    /// **故意不实现 `Drop` 杀进程**：面板崩溃时 rcd 要留下来当孤儿，让下一次启动
    /// 靠三重身份认领并回收（模块头第 3 条）。`Drop` 里杀进程会让那条路径无法被测试，
    /// 还会让 panic 路径悄悄留下一个握着无法重发凭据的守护进程。
    pub async fn shutdown(mut self) -> Result<(), RcdError> {
        stop_child(&mut self.child, self.port, self.pid).await?;
        if let Err(e) = pidfile::remove(&self.pid_path) {
            return Err(RcdError::Io(format!(
                "删除 pid 记录 {} 失败：{e}——那份记录还留在磁盘上，请手动删除",
                self.pid_path.display())));
        }
        Ok(())
    }
}

async fn stop_child(child: &mut Child, port: u16, pid: u32) -> Result<(), RcdError> {
    let uncertain = |reason| RcdError::OrphanNotReapable { port, pid, reason };
    if child.try_wait().map_err(|e| uncertain(e.to_string()))?.is_some() {
        return Ok(());
    }
    child.start_kill().map_err(|e| uncertain(e.to_string()))?;
    tokio::time::timeout(HEALTH_TIMEOUT, child.wait()).await
        .map_err(|_| uncertain("未能确认后台已退出，保留进程记录".into()))?
        .map_err(|e| uncertain(e.to_string()))?;
    Ok(())
}

/// 手写而不是 derive：`RcClient` **故意**不 derive Debug（密码在它的 basic-auth 头里，
/// `src/rc/mod.rs` 那一处），derive 会直接编不过。测试里的 `unwrap_err()` 需要
/// `Result<RcdHandle, _>: Debug`，所以这里给一个只暴露身份字段、把凭据相关的两个字段
/// 写成占位串的 impl——`{h:?}` 永远不可能吐出用户名或密码。
impl std::fmt::Debug for RcdHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RcdHandle")
            .field("port", &self.port)
            .field("pid", &self.pid)
            .field("execute_id", &self.execute_id)
            .field("rclone_version", &self.rclone_version)
            .field("client", &"<RcClient：含凭据，不打印>")
            .field("secrets", &"<redacted>")
            .finish()
    }
}

pub struct RcdSupervisor;

impl RcdSupervisor {
    pub async fn start(spec: RcdSpec) -> Result<RcdHandle, RcdError> {
        let (user, pass) = random_credentials();
        Self::start_with_credentials(spec, &user, &pass).await
    }

    pub async fn start_with_credentials(
        spec: RcdSpec,
        user: &str,
        pass: &str,
    ) -> Result<RcdHandle, RcdError> {
        let home = DaemonHome::claim(&spec.pid_path)?;
        Self::start_in_home(spec, home, user, pass).await
    }

    pub(crate) async fn start_owned(spec: RcdSpec, home: DaemonHome) -> Result<RcdHandle, RcdError> {
        let (user, pass) = random_credentials();
        Self::start_in_home(spec, home, &user, &pass).await
    }

    async fn start_in_home(
        spec: RcdSpec,
        home: DaemonHome,
        user: &str,
        pass: &str,
    ) -> Result<RcdHandle, RcdError> {
        reclaim(&spec).await?;
        if port_in_use(spec.port).await {
            return Err(foreign_listener(spec.port));
        }

        let stderr_path = spec.stderr_path();
        if let Some(dir) = stderr_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| RcdError::Io(e.to_string()))?;
        }
        // 轮转排在 spawn **之前**：rclone 一开就是追加模式，先挪走才有得追。
        // 这件事必须在界面上看得见吗？它是启动路径上的文件系统动作，没有对应的任务日志可写，
        // 所以落点是新日志的第一行——操作员翻开 `rcd.log` 往上追，看见的就是这一句。
        if let Some(log_file) = spec.log_file.as_deref() {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
            match rotate_rc_log(log_file, &stamp, LOG_ROTATE_BYTES, LOG_KEEP) {
                Ok(Some(r)) => {
                    let note = format!(
                        "rclone-deck: 上一份 rcd.log 有 {} 字节，已转存为 {}；保留最新 {} 份，剪掉 {}\n",
                        r.bytes, r.archive, LOG_KEEP,
                        if r.dropped.is_empty() { "没有".to_string() } else { r.dropped.join(", ") });
                    // 写不进去不是"没写就算了"：那份归档存在而没人知道，等于操作员凭空丢了一段历史。
                    std::fs::write(log_file, note)
                        .map_err(|e| RcdError::Io(format!("轮转后往新日志写那句说明失败（{r:?}）：{e}")))?;
                }
                Ok(None) => {}
                Err(e) => return Err(RcdError::Io(format!("轮转 {} 失败：{e}", log_file.display()))),
            }
        }
        // 打开**一次**，直接交给子进程。原来这里是 `fs::write` 截断一次、`build_command` 里再
        // `OpenOptions::open(...).expect(...)` 打开一次：两次打开之间任何事都可能发生（原位置被
        // 换成目录、被杀软锁住），而 `.expect` 在 release 的 GUI 子系统里是**没有窗口也没有消息**
        // 的死亡。截断语义留在这一次打开里，失败从同一个出口变成 `RcdError::Io`，界面看得见。
        let stderr_file = std::fs::OpenOptions::new()
            .create(true).write(true).truncate(true)
            .open(&stderr_path)
            .map_err(|e| RcdError::Io(format!("{}: {e}", stderr_path.display())))?;

        // 裁定 R42 item 2：**擦除凭据回显不许半途**。启动结果收进内层，出去之前无条件擦一次——
        // 原来 spawn 之后的两条 `SpawnFailed` 出口压根没擦（注释却写着"两条分支都清空"），
        // 而擦除本身写成 `let _ =`：truncate 失败（杀软占用 / 权限 / 路径过长）就静默留着，
        // 同函数里其他任何 IO 失败都会变成 `RcdError::Io`。今天生产走不到（`from_config` 硬编
        // `log_level_debug: false`），但"凭据不上磁盘"是不变量，不能靠一个开关没打开来保证。
        let outcome = Self::spawn_and_verify(spec, user, pass, &stderr_path, stderr_file).await;
        match outcome {
            Ok(mut handle) => {
                handle._home = Some(home);
                if let Err(e) = wipe_credential_echo(&stderr_path) {
                    return match handle.shutdown().await {
                        Ok(()) => Err(e),
                        Err(shutdown) => Err(RcdError::Io(format!("{e}；后台收尾失败：{shutdown}"))),
                    };
                }
                Ok(handle)
            }
            Err(e) => {
                // 错误出口同样无条件擦一次。擦不掉 = 操作员日志目录里还躺着一份明文 rc 用户/口令，
                // 这条事实比原来的启动错误更要紧，所以两个都报出去，不让任何一个顶替另一个。
                match wipe_credential_echo(&stderr_path) {
                    Ok(()) => Err(e),
                    Err(w) => Err(RcdError::Io(format!("{w}（启动本身的错误：{e}）"))),
                }
            }
        }
    }

    /// spawn → pid 记录 → 健康检查 → `core/version`：这段里任何一步失败都从这里返回，
    /// 凭据回显的擦除由调用方 `start_with_credentials` 统一负责（裁定 R42 item 2 的形状要求）。
    async fn spawn_and_verify(
        spec: RcdSpec,
        user: &str,
        pass: &str,
        stderr_path: &Path,
        stderr_file: std::fs::File,
    ) -> Result<RcdHandle, RcdError> {
        let mut child = build_command(&spec, user, pass, stderr_file)
            .spawn()
            .map_err(|e| RcdError::SpawnFailed(format!("{}: {e}", spec.rclone_bin.display())))?;
        let pid = child.id()
            .ok_or_else(|| RcdError::SpawnFailed("子进程刚拉起就读不到 PID（可能已经退出）".into()))?;

        let identity = process::Process::open(pid, false)
            .and_then(|process| process.ok_or_else(|| std::io::Error::other("子进程已经退出")))
            .and_then(|process| Ok((process.creation_time()?, process)));
        let (created, process) = match identity {
            Ok(identity) => identity,
            Err(e) => {
                stop_child(&mut child, spec.port, pid).await?;
                return Err(RcdError::SpawnFailed(format!("无法确认新后台进程 {pid} 的创建时间：{e}")));
            }
        };

        // 裁定 R42 item 1：**pid 记录写在任何可能早退的动作之前**（这里四个字段刚齐）
        // ——`core/version` 出错 → `NotHealthy`、`!healthy` 出口，都是"子进程活着、端口还绑着、
        // `Child` 句柄即将被丢弃（没有 `kill_on_drop`）"的出口。记录要是等到健康检查之后才写，
        // 这些出口就留下一个**没有任何 pid 文件可回收**的 daemon：下一次启动端口被占、
        // 读不到 pid → 永远 `PortBusyByForeignProcess`，面板自己变砖，而且那个 rclone
        // 握着的凭据只有已经死掉的上一代面板知道。写在前头 = 早退顶多多一个待收尸的记录，
        // 而记录正是收尸的依据（`!healthy` 那条会当场把它删掉，见下）。
        if let Err(e) = pidfile::write_owned(&spec.pid_path, pidfile::PidRecord {
            pid, start_time: process::unix_seconds(created), port: spec.port,
            panel_pid: std::process::id(),
        }, created) {
            // 写不进去 = 这个 daemon 从此不可回收，正是 item 1 要防的残局。所以这条出口自己收拾：
            // 句柄还在手里，直接 `start_kill()`（比让 `reclaim` 按 PID 号重新查一遍可靠，
            // 见裁定 R42 item 5），终结不掉就把两件事一起报出去，别让操作员只看到一句"pid 文件失败"。
            match child.start_kill() {
                Ok(()) => {
                    // 等它真退出，端口才算放掉；`wait()` 报错只会是"已经被回收"，不值得覆盖 pid 文件的错。
                    let _ = child.wait().await;
                    return Err(RcdError::Io(e.to_string()));
                }
                Err(k) => {
                    return Err(RcdError::Io(format!(
                        "{e}；而且刚拉起的 rcd（PID {pid}）没能当场终结（{k}），端口 {port} 上可能留着一个没有记录的 daemon",
                        port = spec.port
                    )));
                }
            }
        }

        let mut client = RcClient::new(spec.port, user, pass);
        client.bind_process(process);
        let mut last = String::from("一直没有响应");
        let mut healthy = false;
        // 按墙上时钟收，不按尝试次数收（brief 里写的是 `for _ in 0..attempts`，
        // attempts = 20000/150 = 133 次）。本机实测：对着一个**没人听的**回环端口发 connect，
        // 要 2.03s 才拿到 ECONNREFUSED（同一台机器上 `curl` 打过去也是 2.1s，不是 reqwest 的事），
        // 于是那 133 次实际是 292s，而报出去的话术仍写着"20s 内没有响应"——又慢又谎报耗时。
        // deadline 口径两头都对：端口活着一次就过（连上了是 0ms），端口死了 20s 就放弃。
        let deadline = tokio::time::Instant::now() + HEALTH_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(HEALTH_INTERVAL).await;
            match client.call("rc/noop", &[]).await {
                Ok(_) => { healthy = true; break; }
                Err(e) => last = e.to_string(),
            }
        }
        if !healthy {
            let mut cleanup_note = String::new();
            match stop_child(&mut child, spec.port, pid).await {
                Ok(()) => {
                    if let Err(e) = pidfile::remove(&spec.pid_path) {
                        cleanup_note = format!("；此外 pid 记录（{}）没能删掉，请手动删除：{e}",
                            spec.pid_path.display());
                    }
                }
                Err(e) => cleanup_note = format!("；保留后台记录以便重新核验：{e}"),
            }
            // 先杀再读（要拿到退出前最后那几行）；磁盘上那一份由调用方**无条件**擦掉（裁定 R42 item 2）。
            let detail = std::fs::read_to_string(stderr_path).unwrap_or_default();
            // 裁定 R37：`log_level_debug=true` 时这份 stderr 里带着 rclone 的回显
            // `DEBUG : Setting rc_pass="<口令>" ...`，所以把 `detail` 直接拼进错误消息 = 让错误
            // 消息自己成为泄漏面（它会进面板日志/TaskLog）。落盘的已经清空，交出去的这份先脱敏。
            let detail = redact_credentials(detail.trim(), &[user, pass]);
            let mut message = if detail.is_empty() {
                last
            } else {
                format!("{last}\nrclone 自己的报错：{detail}")
            };
            message.push_str(&cleanup_note);
            return Err(RcdError::NotHealthy(HEALTH_TIMEOUT.as_secs(), message));
        }
        // rc 活了，`stderr_path` 里那段"启动回显"就没有回看价值了，而它**带凭据**——但擦除**不在这条
        // 分支里做**（原来在这里写成 `let _ = std::fs::write(..)`，失败就静默留着）：一律由
        // `start_with_credentials` 在拿到本函数的结果之后无条件擦一次并把失败抛出去（裁定 R42 item 2）。
        let version = client.call("core/version", &[]).await
            .map_err(|e| RcdError::NotHealthy(HEALTH_TIMEOUT.as_secs(), e.to_string()))
            .and_then(|v| {
                if port_owner_pid(spec.port) == Some(pid) {
                    Ok(v)
                } else {
                    Err(foreign_listener(spec.port))
                }
            });
        let v = match version {
            Ok(v) => v,
            Err(original) => {
                stop_child(&mut child, spec.port, pid).await?;
                pidfile::remove(&spec.pid_path).map_err(|e| RcdError::Io(format!(
                    "{original}；删除 pid 记录 {} 失败：{e}", spec.pid_path.display())))?;
                return Err(original);
            }
        };
        let rclone_version = v.get("version")
            .and_then(serde_json::Value::as_str).unwrap_or("unknown").to_string();
        let execute_id = v.get("executeId")
            .and_then(serde_json::Value::as_str).unwrap_or_default().to_string();

        Ok(RcdHandle {
            client, port: spec.port, pid, execute_id, rclone_version,
            child, pid_path: spec.pid_path.clone(), _home: None,
            secrets: vec![user.to_string(), pass.to_string()],
        })
    }
}

/// `rcd.log` 的轮转阈值与保留份数。
///
/// 阈值是**量出来的**：操作员真实日志目录里那份 `rcd.log` 两天长了 **3.19 MB / 24,018 行**
/// （rclone 每传一个对象写一行 `NOTICE:`），而同目录里每任务那份日志只有 **0.1 KB**。
/// 所以"日志无限增长"这件事的全部重量都在这一个文件上；动它、**不动**每任务日志，是量完之后的结论：
/// 每任务日志正是本项目承诺的那份可回溯证据，为 0.1 KB 一档去删它没有任何收益。
/// 4 MiB ≈ 两天重度使用；留 5 份归档加当前这份，磁盘上限是几十 MB 而不是无限。
/// `LOG_KEEP` 这个数是**拍的**，理由只有一条：它把上限钉住。
///
/// 只在**启动时**轮转：rclone 握着那个文件的追加句柄，运行中把它挪走等于把日志尾巴从它控制里拽走。
pub const LOG_ROTATE_BYTES: u64 = 4 * 1024 * 1024;
pub const LOG_KEEP: usize = 5;

/// 一次轮转干了什么。调用方要把这件事写进**新**的那份日志：操作员翻开下一份文件时，
/// "上一份去哪了"是第一个会问的问题，而答案是另一个文件名，不是一句"日志被清理过"。
#[derive(Debug, PartialEq, Eq)]
pub struct LogRotation {
    pub archive: String,
    pub bytes: u64,
    /// 被剪掉的更老的归档，按名字交出去（不静默）。
    pub dropped: Vec<String>,
}

/// 把超阈值的 `rcd.log` 挪成一份带时间戳的归档，并把归档剪到最新 `keep` 份。
/// 没到阈值、或还没有这份日志（第一次启动）→ `Ok(None)`。
///
/// 剪**只**碰我们自己造的那个形状 `rcd.<yyyyMMdd-HHmmss>.log`。这一点是刻意的：同目录里躺着
/// `rcd-stderr.log`，它同样以 `rcd` 开头，凭"以 rcd 开头"去删就会删掉它——而那份是凭据回显的
/// 落点，删它的后果比删一份旧日志重得多。时间戳取字典序即时间序，所以既不读 mtime 也不解析日期。
pub fn rotate_rc_log(
    log_file: &Path,
    now: &str,
    max_bytes: u64,
    keep: usize,
) -> std::io::Result<Option<LogRotation>> {
    let meta = match std::fs::metadata(log_file) {
        Ok(m) => m,
        // 只有"这份日志还不存在"才算"没东西可轮转"。别的失败（权限、路径）不许冒充首次启动。
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if meta.len() <= max_bytes {
        return Ok(None);
    }
    let stem = log_file.file_stem().map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rcd".to_string());
    let archive_name = format!("{stem}.{now}.log");
    std::fs::rename(log_file, log_file.with_file_name(&archive_name))?;
    let dir = log_file.parent().ok_or_else(|| std::io::Error::new(
        std::io::ErrorKind::InvalidInput, "日志路径没有父目录，剪不出归档"))?;
    let dropped = trim_archives(dir, &stem, keep)?;
    Ok(Some(LogRotation { archive: archive_name, bytes: meta.len(), dropped }))
}

/// 留下最新 `keep` 份 `<stem>.<时间戳>.log`，删掉更老的。
/// **删不掉就把它报出去**（不写成 `let _ =`）：轮转的全部意义是钉住磁盘上限，
/// 一次静默失败的剪等于没有上限。
fn trim_archives(dir: &Path, stem: &str, keep: usize) -> std::io::Result<Vec<String>> {
    let prefix = format!("{stem}.");
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        // `prefix` 带着那个点，所以 `rcd-stderr.log` 天生进不来；再加一条"点和时间戳之间
        // 得真有点东西"，是为了 `rcd..log` 这种空时间戳的畸形名字也不许被当成归档去删。
        .filter(|n| n.len() > prefix.len() + 4 && n.starts_with(&prefix) && n.ends_with(".log"))
        .collect();
    names.sort();
    let mut dropped = Vec::new();
    if names.len() > keep {
        let cut = names.len() - keep;
        for n in names.drain(..cut) {
            std::fs::remove_file(dir.join(&n))?;
            dropped.push(n);
        }
    }
    Ok(dropped)
}

/// 把 `rcd-stderr.log` 里那份启动回显清空（裁定 R42 item 2）。
///
/// 为什么必须有这一步：实测 rclone 在 `--log-level DEBUG` 下会把环境里的凭据**原样**打到 stderr
/// （两行 `DEBUG : Setting --rc-pass "<口令>" from environment variable RCLONE_RC_PASS="<口令>"`），
/// 而本监管器把子进程的 stderr 落成操作员日志目录里的 `rcd-stderr.log` 长期留存；同一个进程的
/// `--log-file` 里却一行都没有（本机 raw 抓取，见 task-7-report.md）。生产侧 `log_level_debug`
/// 恒为 false，所以今天不落盘——但"凭据不上磁盘"是不变量，不能靠一个开关没打开来保证，
/// 何况哪天加一个 DEBUG 排障开关，这条就是磁盘上的明文凭据。
///
/// 为什么它**不许**是 `let _ =`：truncate 会失败（杀软占用 / 权限 / 只读 / 路径过长），
/// 而同一个函数里其他任何 IO 失败都会变成 `RcdError::Io`。擦不掉的后果是"凭据还在磁盘上"，
/// 这正是最需要让操作员立刻知道的事，所以这里返回 `Result`，由调用方把错误抛出去。
/// 调用点在 `start_with_credentials`：**所有** spawn 之后的出口（含两条 `SpawnFailed`）都先擦再返回。
fn wipe_credential_echo(path: &Path) -> Result<(), RcdError> {
    std::fs::write(path, "").map_err(|e| RcdError::Io(format!(
        "清空 {} 失败：{e}——那份 rclone 的 rc 用户/口令回显可能还留在磁盘上，请手动删除该文件",
        path.display()
    )))
}

/// 环境清扫（R26 那条通道）到底清不清某个键——抽成纯函数是为了能被测试钉住，`build_command`
/// 的循环只是它的一个调用点。
///
/// 裁定 R26 的原委：rclone 把 `RCLONE_<FLAG>` 当成旗标的默认值，操作员 shell 里一句
/// `RCLONE_COPY_LINKS=1` 就能让面板"明明没写 -L"却开始跟随 junction，桶里多出
/// `link/a.webp` 这种本地扫描结构上不可能产生的 key —— 对 prune 就是假孤儿。config 层的
/// 旗标白名单管的是 argv，管不到进程环境，所以这一侧必须把非我们主动设置的 `RCLONE_*` 全清掉。
///
/// 裁定 R36：保留两个名字——`RCLONE_CONFIG`（去哪找配置文件）与 `RCLONE_CONFIG_PASS`
/// （它的解密口令）。口令**不是**旗标默认值、也**不**改变"传什么进桶"的形状，因此不在 R26
/// 那条清扫通道的作用域内；把它一并清掉的话，拿着加密 `rclone.conf` 的操作员会得到一个
/// `NotHealthy` 的 daemon，而不是一个能干活的 daemon——这是把安全清扫做过头伤到功能。
///
/// 其余以 `RCLONE_` 开头的一律清除，**包括** `RCLONE_RC_USER` / `RCLONE_RC_PASS`：这两个由
/// 监管器自己在 `build_command` 里用 `cmd.env` 设成随机凭据，继承来的陈旧一对绝不能赢
/// （清掉它 = 我们设的那对是环境里唯一的来源）。
///
/// 裁定 R39（评审发现，R26 的漏洞）：**判据不区分大小写**。两边都不是区分大小写的——
/// - Windows 的环境变量查找本身不区分大小写（`env::vars_os()` 给出的是原样名字，而
///   `env_remove`/`env` 落在同一个不区分大小写的表上）；
/// - rclone 的 `RCLONE_<FLAG>` 默认值通道也不区分：从 Git Bash 继承来的
///   `rclone_copy_links=1`（小写）照样被当成 `--copy-links` 的默认值。
/// 按前缀**大小写敏感**地清，那条小写的键就躲过清扫、把旗标默认值带进 daemon——
/// 正是 R26 立这条通道要拦的那件事（行为侧的症状：桶里多出 `link/a.webp` 这种
/// 本地扫描结构上不可能产生的 key，对 prune 就是假孤儿）。
/// 所以先 `to_ascii_uppercase` 再判前缀与名单，名单本身写成大写。
/// 注意这只规范化**判据**：真正 `env_remove` 用的还是原始键名（见 `build_command`），
/// 否则在小写环境下删一个"大写副本"会什么都删不掉。
/// 判据方向：`RCLONE_RC_USER`/`RCLONE_RC_PASS` 在这里被**判定为"要清"**（返回 `true`）。
pub fn rclone_env_is_scrubbed(key: &str) -> bool {
    const KEEP: [&str; 2] = ["RCLONE_CONFIG", "RCLONE_CONFIG_PASS"];
    let key = key.to_ascii_uppercase();
    key.starts_with("RCLONE_") && !KEEP.contains(&&*key)
}

/// 把凭据从一段要交出去的文本里抹掉（裁定 R37）。**全仓库唯一的一份擦密实现**（裁定 R44）：
/// `runner::tasklog::scrub` 以前是同一段算法抄的第二遍，现在它是这里的一行转发。
/// Task 9 起调用方会变多（每条 `log.line`/`log.kv`、失败详情、任何新的落盘路径都要擦），
/// 所以"只改了一边"的机会从此不存在。
///
/// 用在这里是因为**失败启动**这条路上，唯一读进内存再拼进错误消息的是 stderr：
/// `log_level_debug=true` 时 rclone 会把口令原样打到 stderr（`DEBUG : Setting rc_pass="<口令>"`），
/// 那段 `detail` 会进 `NotHealthy` 的话术，于是错误消息本身成为泄漏面。落盘的 `rcd-stderr.log`
/// 由 `start_with_credentials` 在**每一个** spawn 之后的出口无条件清空（裁定 R42 item 2；
/// 原来只有健康分支和 `!healthy` 分支各擦一次、且都写成 `let _ =`，两条 `SpawnFailed` 则压根没擦），
/// 交出去的这份也要脱敏。
///
/// 审计另外三条 `SpawnFailed`：它们**从不读 stderr**，所以本就不可能带口令——
/// `spawn()` 失败只拼 OS 错误（`format!("{}: {e}", ...)`，`e` 是"找不到程序/权限"这类 OS 话术），
/// 另两条是固定文案（"读不到 PID""查不到进程信息"）。既然没有捕获 stderr 被嵌进去，
/// 这条脱敏对它们是 no-op；仍放在这里，是为了"失败启动的详情里不许有凭据"成为一条
/// **不依赖具体哪个分支**的不变量，将来谁把 stderr 读进 `SpawnFailed` 也自动受保护。
///
/// 裁定 R39（两处健壮性）+ R44（收成一份实现）：
/// - **按长度降序替换**。按调用方给的顺序换的话，一个密钥是另一个的子串时，短的先命中会
///   把长的匹配打断，把长密钥的尾巴原样留在交出去的文本里——按值脱敏因此必须长值优先。
///   今天这对随机凭据（`deck-<8hex>` 与 32 位 hex）互不包含，但这个函数是 `pub` 的，
///   调用方（Task 9 把 `RcdHandle::secrets()` 之外的串也可能会递进来）给的是任意列表。
///   `runner::tasklog::scrub` 原来自己排了一遍同样的序，R44 起它只转发到这里。
/// - **掩码串统一成 `[REDACTED]`**。原来这里是 `<redacted>` 而 Task 8 的日志写 `[REDACTED]`，
///   同一个"这里原本有个密钥"的意思在同一条人眼要读的输出流里有两种写法（`NotHealthy` 的详情
///   最终会进 `TaskLog` 的行里）。定在 `[REDACTED]`：方括号不与 rclone 自己的 `<…>` 占位/
///   标签形状撞车（grep 一眼分得清"被抹掉的值"和"rclone 写的尖括号"），且 Task 8 侧的落盘
///   日志与它的四条既有断言已经是这个写法，改动面最小。
/// - **长度下限 `>= 4`（R44 从 `tasklog::scrub` 并进正典，两边现在共用同一个数）**。
///   跳过短值会让"一个三位数的密钥"活下来，所以这条要按**契约**而不是按本函数自己来读：
///   它安全**不是因为短密钥不存在**，而是因为**走到这里的密钥全部来自 `random_credentials()`**
///   （`deck-<8hex>` = 13 字符，口令是 32 位 hex），生产路径上一条短值都递不进来。
///   那个前提是 `random_credentials()` 的契约，由证人
///   `the_generated_credentials_clear_the_scrub_floor_and_never_nest`（`tests/rcd_supervisor.rs`）
///   钉住：两个返回值都 `>= 4` 且互不为子串。哪天有调用方递进短值，下限就给不了保护，
///   要改的是那个调用方而不是这里的数。
///   反过来也不能没有下限：没有它，一个单字母"密钥"会把每一行都变成 `[REDACTED]`，
///   日志从此不可读（按值脱敏的代价就是要设一个下限，它只能让日志变啰嗦，不能让它不可用）。
pub fn redact_credentials(text: &str, secrets: &[&str]) -> String {
    let mut order: Vec<&str> = secrets.iter().copied().collect();
    order.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));

    let mut out = text.to_string();
    for s in order {
        // 短于 4 个字符的一律跳过（含空串——`replace("", x)` 会在每个字符之间插入掩码，
        // 把整段文本吃掉）。为什么这个取舍是安全的：见上面文档里指向的 `random_credentials()`
        // 契约，不要在这里给自己找理由。
        if s.chars().count() >= 4 {
            out = out.replace(s, "[REDACTED]");
        }
    }
    out
}

fn build_command(spec: &RcdSpec, user: &str, pass: &str, stderr_file: std::fs::File) -> Command {
    let mut cmd = Command::new(&spec.rclone_bin);
    // 继承来的 RCLONE_* 全部清掉，但保留两个名字（见 `rclone_env_is_scrubbed` 的裁定 R36）。
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy().into_owned();
        if rclone_env_is_scrubbed(&key) {
            cmd.env_remove(&key);
        }
    }
    cmd.arg("rcd")
        .arg("--rc-addr").arg(format!("127.0.0.1:{}", spec.port))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file));
    // 凭据只走环境变量：走 argv 会同时泄进 DEBUG 日志和 Win32_Process.CommandLine。
    // 环境里那两个键正是上面清理循环留下的唯一"我们主动设置"的 RCLONE_* 通道。
    for (k, v) in [("RCLONE_RC_USER", user), ("RCLONE_RC_PASS", pass)] {
        cmd.env(k, v);
    }
    // 指哪一份 rclone 配置：默认不设（用 rclone 自己的位置）。测试夹具靠这一条把 rcd
    // 关进一份只写着临时 local 远端的 conf，见裁定 R30。
    if let Some(conf) = spec.rclone_config.as_deref() {
        cmd.env("RCLONE_CONFIG", conf);
    }
    for f in &spec.flags {
        cmd.arg(f);
    }
    for e in &spec.exclude {
        cmd.arg("--exclude").arg(e);
    }
    if let Some(log) = &spec.log_file {
        cmd.arg("--log-file").arg(log);
    }
    if spec.log_level_debug {
        cmd.arg("--log-level").arg("DEBUG");
    }
    apply_no_window(&mut cmd);
    cmd
}

// tokio 的 `creation_flags` 是 inherent 方法（`Command` 自己带的方法，不是 trait 提供的；
// 版本就是 Cargo.lock 钉的那一支 tokio 1.53），
// 所以这里不导入 CommandExt：导了也用不上，只会换来一条 unused_imports。
#[cfg(windows)]
fn apply_no_window(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn apply_no_window(_cmd: &mut Command) {}

/// 一次「这个端口现在接不接连接」的探测。**只有真的连上才算在**：
/// `timeout` 返回 `Ok(Err(...))`（例如 ECONNREFUSED）是"有人拒了"，不是"有人在听"。
///
/// 存活读数与孤儿回收必须用同一份判据；以前 `port_in_use` 写的是 `.is_ok()`，
/// 在拒绝瞬时返回的系统上会把空端口判成占用，只是本机那约 2 秒的 RST 把它盖住了。
pub async fn port_accepts(port: u16, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(timeout, TcpStream::connect(("127.0.0.1", port))).await,
        Ok(Ok(_))
    )
}

async fn port_in_use(port: u16) -> bool {
    port_accepts(port, PROBE_TIMEOUT).await
}

/// Check the previous record even when the new panel selected a different port.
async fn reclaim(spec: &RcdSpec) -> Result<(), RcdError> {
    let Some(stored) = pidfile::read_checked(&spec.pid_path).map_err(|e| {
        RcdError::Io(format!("无法确认上一份后台记录 {}：{e}", spec.pid_path.display()))
    })? else { return Ok(()) };
    let rec = stored.record;
    let uncertain = |reason| RcdError::OrphanNotReapable {
        port: rec.port, pid: rec.pid, reason,
    };
    let Some(previous) = process::Process::open(rec.pid, true)
        .map_err(|e| uncertain(format!("无法验证进程身份：{e}")))? else { return Ok(()) };
    if previous.exited().map_err(|e| uncertain(format!("无法确认进程是否退出：{e}")))? {
        return Ok(());
    }
    let created = previous.creation_time()
        .map_err(|e| uncertain(format!("无法验证进程创建时间：{e}")))?;
    let same_creation = match stored.creation_time {
        Some(previous) => previous == created,
        None => rec.start_time == process::unix_seconds(created),
    };
    if !same_creation {
        return Ok(());
    }
    let exe = previous.executable().map_err(|e| uncertain(format!("无法验证可执行文件：{e}")))?;
    if !same_path(Some(&exe), &spec.rclone_bin) {
        return Err(uncertain("记录中的可执行文件与当前配置不同，请核实后台是否仍在运行".into()));
    }
    if rec.panel_pid != 0 && rec.panel_pid != std::process::id() {
        let panel = process::Process::open(rec.panel_pid, false)
            .and_then(|panel| panel.map_or(Ok(false), |panel| panel.exited().map(|exited| !exited)))
            .map_err(|e| uncertain(format!("无法确认原面板已退出：{e}")))?;
        if panel {
            return Err(RcdError::PortBusyByForeignProcess {
                port: rec.port, pid: rec.pid,
                name: format!("另一个面板进程（PID {}）正在管理后台", rec.panel_pid),
            });
        }
    }

    // Older records only have seconds of precision. Require the recorded listener
    // and the expected rcd arguments as additional evidence before reclaiming them.
    if stored.creation_time.is_none() {
        let addr = format!("127.0.0.1:{}", rec.port);
        let cmd = process_command_line(rec.pid);
        let is_rcd = cmd.get(1).is_some_and(|arg| arg == "rcd")
            && cmd.windows(2).any(|args| args[0] == "--rc-addr" && args[1] == addr);
        if port_owner_pid(rec.port) != Some(rec.pid) || !is_rcd {
            return Err(uncertain("旧记录无法确认后台身份，请核实进程和端口".into()));
        }
    }

    previous.terminate().map_err(|e| uncertain(format!("无法终结已确认归属的后台：{e}")))?;
    for _ in 0..REAP_ATTEMPTS {
        if previous.exited().map_err(|e| uncertain(format!("无法确认后台已退出：{e}")))? {
            return pidfile::remove(&spec.pid_path).map_err(|e| RcdError::Io(format!(
                "回收之后删除 pid 记录 {} 失败：{e}", spec.pid_path.display())));
        }
        tokio::time::sleep(REAP_INTERVAL).await;
    }
    Err(uncertain("已请求终结后台，仍未确认进程退出".into()))
}

fn foreign_listener(port: u16) -> RcdError {
    let pid = port_owner_pid(port).unwrap_or_default();
    let name = inspect(pid).map(|info| info.name).unwrap_or_else(|| "无法确认监听进程".into());
    RcdError::PortBusyByForeignProcess { port, pid, name }
}

fn port_owner_pid(port: u16) -> Option<u32> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let loopback = format!("127.0.0.1:{port}");
    let wildcard = format!("0.0.0.0:{port}");
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 5 && (cols[1] == loopback || cols[1] == wildcard) && cols[3] == "LISTENING" {
            if let Ok(pid) = cols[4].parse::<u32>() {
                return Some(pid);
            }
        }
    }
    None
}

struct ProcInfo {
    #[cfg(feature = "test-helpers")]
    start_time: u64,
    name: String,
    #[cfg(feature = "test-helpers")]
    exe: Option<PathBuf>,
}

fn inspect(pid: u32) -> Option<ProcInfo> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    let p = sys.process(Pid::from_u32(pid))?;
    Some(ProcInfo {
        #[cfg(feature = "test-helpers")]
        start_time: p.start_time(),
        name: p.name().to_string_lossy().into_owned(),
        #[cfg(feature = "test-helpers")]
        exe: p.exe().map(PathBuf::from),
    })
}

#[cfg(feature = "test-helpers")]
fn kill(pid: u32) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    match sys.process(Pid::from_u32(pid)) {
        Some(p) => p.kill(),
        None => true,
    }
}

/// Windows 路径大小写不敏感，且 sysinfo 给的可能是 8.3 短名或不同大小写的盘符。
fn same_path(a: Option<&Path>, b: &Path) -> bool {
    let Some(a) = a else { return false };
    let norm = |p: &Path| {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy().to_lowercase().replace('/', "\\")
    };
    norm(a) == norm(b)
}

/// 子进程的命令行。测试用它钉"凭据不上 argv"这条不变量，所以它属于 `test-helpers`，
/// 不进生产构建。
///
/// 必须走 `refresh_processes_specifics` 并显式要 `cmd`：`System::refresh_processes`
/// 的 RefreshKind 里根本没有 cmd（sysinfo-0.39.6 src/common/system.rs:317-322），
/// 用它读到的命令行永远是空的，那条断言就成了空转。
#[cfg(feature = "test-helpers")]
pub fn command_line_of(pid: u32) -> Vec<String> {
    process_command_line(pid)
}

fn process_command_line(pid: u32) -> Vec<String> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    match sys.process(Pid::from_u32(pid)) {
        Some(p) => p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect(),
        None => vec![],
    }
}

/// 端口监听者的 PID。导出它**只**是为了让测试守卫复用 `reclaim` 那一份"谁占着端口"的判定，
/// 免得测试文件里长出第二套 netstat 解析（裁定 R45 item 2 明确要求复用而不是重写）。
/// 判定逻辑一个字都没改：这就是 `reclaim` 在用的那个函数。
#[cfg(feature = "test-helpers")]
pub fn port_listener_pid(port: u16) -> Option<u32> {
    port_owner_pid(port)
}

/// 按 PID 号终结进程，走的是 `reclaim` 认领孤儿用的同一个 `kill()`（sysinfo 的
/// `Process::kill`，进程已不存在时返回 true）。测试守卫的**无句柄形态**只能用这一档：
/// 它手上没有 `Child`，拿不到 `RcdHandle::kill_now`。调用方必须先自己确认那个 PID
/// 确实是目标进程（守卫用 `command_line_of` 做三重身份判定），所以这里不加身份逻辑。
#[cfg(feature = "test-helpers")]
pub fn kill_pid(pid: u32) -> bool {
    kill(pid)
}

/// 这条 PID 现在是不是**当初写下 `start_time` 的那个、映像就是 `rclone_bin` 的进程**。
/// 导出它**只**是为了让测试守卫的 pid 记录档复用 `reclaim` 的那一条身份判据
/// （裁定 R46 item 3：口径必须只有一个，不能新写一套）——判据就是 `reclaim` 里
/// `info.start_time != rec.start_time || !same_path(info.exe.as_deref(), &spec.rclone_bin)`
/// 那两行的正面写法，`inspect` 与 `same_path` 都是生产在用的那两个函数，一个字没改。
/// 进程已经不在时返回 false：没有东西可收，也就没有东西可杀。
#[cfg(feature = "test-helpers")]
pub fn record_identity_matches(pid: u32, start_time: u64, rclone_bin: &Path) -> bool {
    match inspect(pid) {
        Some(info) => info.start_time == start_time && same_path(info.exe.as_deref(), rclone_bin),
        None => false,
    }
}
