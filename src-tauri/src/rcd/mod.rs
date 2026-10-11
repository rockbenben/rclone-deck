//! rclone `rcd` 子进程的监管：拉起、健康检查、pid 文件、孤儿回收、收尾。
//!
//! 三条实测根因都落在这一层（spec §3.1）：
//! 1. **零控制台窗口**——rclone 是控制台程序，面板是 GUI 子系统程序，不设
//!    `CREATE_NO_WINDOW` 就闪窗；本机 `QuickEdit=1`，误点一下就把子进程冻死且无报错。
//! 2. **凭据绝不进 argv**——`--rc-pass` 上命令行会同时泄进 rclone 自己的 DEBUG 日志
//!    和 `Win32_Process.CommandLine`（probe 有对照组）。只走 `RCLONE_RC_USER` /
//!    `RCLONE_RC_PASS` 环境变量。
//! 3. **孤儿认领**——面板崩溃后留下的 rcd 必须靠四件事认出来并**终结重开**：pid 文件的
//!    start_time == sysinfo 的进程启动时间、进程映像路径 == `rclone_bin`、端口在听，
//!    以及**写这条记录的面板进程已经不在**（`panel_pid`，裁定 R42 item 3——同一用户的 dev 构建
//!    和 release 构建共用 `pid_path` 与端口 默认端口，前三件事全对也可能是在杀活同行的 daemon）。
//!    绝不复用：它内存里的凭据不是我们的。
//!    认领的依据（pid 记录）必须在**任何可能早退的动作之前**落盘（裁定 R42 item 1）：
//!    没有记录的 daemon 是不可回收的 daemon，而不可回收的 daemon 让面板自己变砖。

pub mod pidfile;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use sysinfo::{Pid, ProcessesToUpdate, System};
// 这两个只有 `command_line_of` 用得到（它必须显式要 cmd），而它在 test-helpers 后面：
// 放在顶层 use 里会让 `cargo check --all-targets`（不带 feature）报 unused_imports。
#[cfg(feature = "test-helpers")]
use sysinfo::{ProcessRefreshKind, UpdateKind};
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
    use rand::Rng;
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
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        for _ in 0..REAP_ATTEMPTS {
            if !port_in_use(self.port).await {
                break;
            }
            tokio::time::sleep(REAP_INTERVAL).await;
        }
        // 收尾失败不许静默（裁定 R56 item 3）。两条出口各自把它拼在自己那句话的**后面**：
        // "端口仍在监听"先说，"记录没删掉"跟在后面，两者一起进 `OrphanNotReapable.reason`；
        // 端口已经让开时这条就成了唯一的问题，单独作为 `Io` 报出去
        // （那个变体的话术本来就是"pid 文件读写失败"）。都不新增变体。
        let cleanup = pidfile::remove(&self.pid_path);
        if port_in_use(self.port).await {
            let mut reason = String::from("shutdown 之后端口仍在监听");
            if let Err(e) = cleanup {
                reason.push_str(&format!(
                    "；此外 pid 记录（{}）没能删掉，请手动删除：{e}", self.pid_path.display()));
            }
            return Err(RcdError::OrphanNotReapable { port: self.port, pid: self.pid, reason });
        }
        if let Err(e) = cleanup {
            return Err(RcdError::Io(format!(
                "删除 pid 记录 {} 失败：{e}——那份记录还留在磁盘上，请手动删除",
                self.pid_path.display())));
        }
        Ok(())
    }
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
        if port_in_use(spec.port).await {
            reclaim(&spec).await?;
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
            Ok(handle) => {
                wipe_credential_echo(&stderr_path)?;
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

        let info = inspect(pid)
            .ok_or_else(|| RcdError::SpawnFailed("子进程刚拉起就查不到它的进程信息".into()))?;

        // 裁定 R42 item 1：**pid 记录写在任何可能早退的动作之前**（这里四个字段刚齐）
        // ——`core/version` 出错 → `NotHealthy`、`!healthy` 出口，都是"子进程活着、端口还绑着、
        // `Child` 句柄即将被丢弃（没有 `kill_on_drop`）"的出口。记录要是等到健康检查之后才写，
        // 这些出口就留下一个**没有任何 pid 文件可回收**的 daemon：下一次启动端口被占、
        // 读不到 pid → 永远 `PortBusyByForeignProcess`，面板自己变砖，而且那个 rclone
        // 握着的凭据只有已经死掉的上一代面板知道。写在前头 = 早退顶多多一个待收尸的记录，
        // 而记录正是收尸的依据（`!healthy` 那条会当场把它删掉，见下）。
        if let Err(e) = pidfile::write(&spec.pid_path, pidfile::PidRecord {
            pid, start_time: info.start_time, port: spec.port, panel_pid: std::process::id(),
        }) {
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

        let client = RcClient::new(spec.port, user, pass);
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
            // 裁定 R42 item 5：**句柄还在手里，就终结自己那个进程**。`kill(pid)` 是按 PID 号
            // 重新查一遍再杀，而且不重新校验 `start_time`——PID 被系统复用的话，杀的是别人。
            // `child.start_kill()` 打的才是我们确实握着的那个进程（TerminateProcess by handle）。
            // `reclaim` 那边没有句柄，只能继续用 `kill(owner)`，但那里刚刚比对过 start_time。
            let kill_note = match child.start_kill() {
                // 假 flag 这类子进程是自己退的：`start_kill` 在那里报错不是新故障，
                // 但也不能不提——操作员需要知道我们到底有没有亲手终结过它。
                Ok(()) => String::new(),
                Err(e) => format!("；终结 rcd（PID {pid}）时又失败了一次：{e}"),
            };
            // 等它真退出：端口放掉、stderr 也写完了，下面读到的 `detail` 才是最后一版。
            // `wait()` 的错误只会是"已经被回收"，对这条话术没有增量信息，所以这里不覆盖启动错误。
            let _ = child.wait().await;
            // 这条出口上 daemon 已经被终结，记录再留着就是**假线索**：PID 会被系统复用，
            // 下一个恰好复用该 PID 的进程会带着 `port` 与 `start_time` 的比对被误认成"我们的 rcd"。
            // 所以早退路径自己收尾（裁定 R42 item 1 的后半句）。
            //
            // 收尾失败不许静默（裁定 R56 item 3）：`pidfile::remove` 现在把 io::Error 交回来，
            // 这一条出口手上正好有一个要交出去的 `RcdError`，就拼进它的话术——**排在原始失败
            // 与终结说明之后**（先说"rcd 没响应"，再说"顺手删记录也没删掉"），
            // 不为它新增错误变体，也不让它顶掉真正的原因。
            let cleanup_note = match pidfile::remove(&spec.pid_path) {
                Ok(()) => String::new(),
                Err(e) => format!(
                    "；此外 pid 记录（{}）没能删掉，请手动删除，否则下一个复用该 PID 的进程可能被误认成我们的 rcd：{e}",
                    spec.pid_path.display()),
            };
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
            message.push_str(&kill_note);
            message.push_str(&cleanup_note);
            return Err(RcdError::NotHealthy(HEALTH_TIMEOUT.as_secs(), message));
        }
        // rc 活了，`stderr_path` 里那段"启动回显"就没有回看价值了，而它**带凭据**——但擦除**不在这条
        // 分支里做**（原来在这里写成 `let _ = std::fs::write(..)`，失败就静默留着）：一律由
        // `start_with_credentials` 在拿到本函数的结果之后无条件擦一次并把失败抛出去（裁定 R42 item 2）。
        let v = client.call("core/version", &[]).await
            .map_err(|e| RcdError::NotHealthy(HEALTH_TIMEOUT.as_secs(), e.to_string()))?;
        let rclone_version = v.get("version")
            .and_then(serde_json::Value::as_str).unwrap_or("unknown").to_string();
        let execute_id = v.get("executeId")
            .and_then(serde_json::Value::as_str).unwrap_or_default().to_string();

        Ok(RcdHandle {
            client, port: spec.port, pid, execute_id, rclone_version,
            child, pid_path: spec.pid_path.clone(),
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

async fn port_in_use(port: u16) -> bool {
    tokio::time::timeout(PROBE_TIMEOUT,
        TcpStream::connect(("127.0.0.1", port))).await.is_ok()
}

/// 端口被占时的处置：先自证是不是我们上一次崩溃留下的 rcd，是就终结重开，
/// 不是就报错退出并给出占用者身份。**不做"杀一次算一次"，也不静默换端口。**
///
/// **单实例假设（裁定 R42 item 3，写成明文而不是留在暗里）**：`pid_path` 与端口对同一个 Windows
/// 用户是**常量**。端口这一档今天仍然是常量，但**理由变了**：它来自 `deck_port()` —— 没设
/// `RCLONE_DECK_PORT` 时按 `AppConfig::pick_port` 现挑，而扫描起点固定，所以同一台机器上两个面板
/// 算出的是同一个号。**设了那个变量这条假设就断**（两个面板各占一个号、谁也看不见谁，于是各起一个
/// rcd），要真隔离得连 `RCLONE_DECK_HOME` 一起设（pid 记录跟着 home 走）。dev 与 release 构建共用，
/// 而 Task 6 的互斥锁只在进程内（`AtomicU64`），挡不住两个面板同时跑。下面的 `panel_pid`
/// 活性检查因此是最后一道"别杀活同行的 daemon"的闸；真正强制同一时刻只有一个面板属于
/// Task 12（跨进程单实例），那一步落地之后这条检查仍然留着——它是防御性的，成本是一次进程查询。
async fn reclaim(spec: &RcdSpec) -> Result<(), RcdError> {
    let Some(owner) = port_owner_pid(spec.port) else { return Ok(()) };
    let foreign = |pid: u32| RcdError::PortBusyByForeignProcess {
        port: spec.port,
        pid,
        name: inspect(pid).map(|i| i.name).unwrap_or_else(|| "未知进程".into()),
    };

    let Some(rec) = pidfile::read(&spec.pid_path) else { return Err(foreign(owner)) };
    if rec.pid != owner || rec.port != spec.port {
        return Err(foreign(owner));
    }
    let Some(info) = inspect(owner) else { return Err(foreign(owner)) };
    if info.start_time != rec.start_time || !same_path(info.exe.as_deref(), &spec.rclone_bin) {
        return Err(foreign(owner));
    }

    // 四条身份全对上只能说明"端口上这个 rclone 是某个面板按同一份配置拉起来的"，
    // 说明不了"写记录的那个面板已经不在"。那个进程还活着 = 这是**同行正在用的 daemon**：
    // 它内存里的凭据我们不知道，杀掉它会把对方传了一半的 sync 拦腰截断——而本项目要防的
    // founding incident 正是两次反向 sync 互相删文件。所以这里只报错，绝不动它。
    // 两个"记录的主人不算活着"的例外：`panel_pid == 0` 是还没有这个字段的旧记录
    // （`pidfile::PidRecord` 的 `#[serde(default)]`，退回三重身份判定，别让升级期的孤儿变成
    // 永久不可回收）；`panel_pid == 自己` 是本进程上一轮留下的残局——测试里"面板崩溃"只能
    // 同进程模拟，生产里同一进程第二次 start 是调用方的用法错误，两种都照常回收。
    if rec.panel_pid != 0 && rec.panel_pid != std::process::id() && process_is_alive(rec.panel_pid) {
        return Err(RcdError::PortBusyByForeignProcess {
            port: spec.port,
            pid: owner,
            name: format!(
                "另一个面板进程（PID {}）还在跑的 rcd——本面板不抢它，请先关掉那个面板",
                rec.panel_pid
            ),
        });
    }

    if !kill(owner) {
        return Err(RcdError::OrphanNotReapable {
            port: spec.port, pid: owner, reason: "kill() 返回 false".into(),
        });
    }
    // 裁定 R42 item 5：话术里的耗时必须是**量出来的**。原来写的是
    // `REAP_ATTEMPTS × REAP_INTERVAL = 50 × 200ms = 10s`，可那只是循环的**下限**：
    // 每一轮还要付一次 `port_in_use` 探测，探测外面套的是 300ms 连接超时
    // （本机对一个没人听的回环端口发 connect，要 ~2s 才被拒，所以这 300ms 经常吃满），
    // 于是最坏是 50 × (200 + 300)ms ≈ 25s。操作员按话术等 10s 就会以为"卡住了"。
    // 这里改成起表 + `elapsed()`：报出去的数字是那一段真正花掉的秒数。
    let reap_started = std::time::Instant::now();
    for _ in 0..REAP_ATTEMPTS {
        tokio::time::sleep(REAP_INTERVAL).await;
        if !port_in_use(spec.port).await {
            // 收尾失败不许静默（裁定 R56 item 3）：孤儿已经终结、端口已经让开，这条记录留下的
            // 只是**假线索**，删不掉就得让操作员知道（同样落在 `Io` 那个变体上，不新增）。
            pidfile::remove(&spec.pid_path).map_err(|e| RcdError::Io(format!(
                "回收之后删除 pid 记录 {} 失败：{e}——那份记录还留在磁盘上，请手动删除",
                spec.pid_path.display())))?;
            return Ok(());
        }
    }
    Err(RcdError::OrphanNotReapable {
        port: spec.port,
        pid: owner,
        reason: format!(
            "{}.{:03}s 后端口仍在监听（上限 {upper}ms = {attempts} 次间隔 {interval}ms + 每次最多 {probe}ms 探测）",
            reap_started.elapsed().as_secs(),
            reap_started.elapsed().as_millis() % 1000,
            attempts = REAP_ATTEMPTS,
            interval = REAP_INTERVAL.as_millis(),
            probe = PROBE_TIMEOUT.as_millis(),
            upper = REAP_ATTEMPTS as u64 * (REAP_INTERVAL.as_millis() as u64 + PROBE_TIMEOUT.as_millis() as u64),
        ),
    })
}

fn port_owner_pid(port: u16) -> Option<u32> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("netstat")
        .args(["-ano", "-p", "tcp"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let needle = format!(":{port}");
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 5 && cols[1].ends_with(&needle) && cols[3] == "LISTENING" {
            if let Ok(pid) = cols[4].parse::<u32>() {
                return Some(pid);
            }
        }
    }
    None
}

struct ProcInfo {
    start_time: u64,
    name: String,
    exe: Option<PathBuf>,
}

fn inspect(pid: u32) -> Option<ProcInfo> {
    // `refresh_processes` 在 sysinfo 0.38 里的 RefreshKind 是
    // `nothing().with_memory().with_cpu().with_disk_usage().with_exe(OnlyIfNotSet).with_tasks()`
    // （sysinfo-0.38.4 src/common/system.rs:309-318），所以 name / start_time / exe 都有，
    // 孤儿认领的三重身份靠它够了；它**不含 cmd**，要命令行得走下面那条 specifics 路径。
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    let p = sys.process(Pid::from_u32(pid))?;
    Some(ProcInfo {
        start_time: p.start_time(),
        name: p.name().to_string_lossy().into_owned(),
        exe: p.exe().map(PathBuf::from),
    })
}

fn kill(pid: u32) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
    match sys.process(Pid::from_u32(pid)) {
        Some(p) => p.kill(),
        None => true,
    }
}

/// 这个 PID 现在是不是还活着一个进程（裁定 R42 item 3 的活性判定）。
/// 走的是和身份校验同一个 `inspect`，所以"活着"与"是当初那个进程"用的是同一份 sysinfo 口径。
fn process_is_alive(pid: u32) -> bool {
    inspect(pid).is_some()
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
/// 的 RefreshKind 里根本没有 cmd（sysinfo-0.38.4 src/common/system.rs:309-318），
/// 用它读到的命令行永远是空的，那条断言就成了空转。
#[cfg(feature = "test-helpers")]
pub fn command_line_of(pid: u32) -> Vec<String> {
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
