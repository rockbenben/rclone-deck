//! 起真 rclone daemon 的共享夹具（从 `tests/runner_local.rs` 搬来，裁定 R60）。
//! 搬而不是复制：两处各一份夹具，早晚会漂移，而漂移的下游是"某个 harness 其实什么都没测"。
//! 单线程跑（`--test-threads=1`），每条起 daemon 的用例都必须持有返回的 `DaemonGuard`。
//!
//! 这个文件的规则（裁定 R45 / R46 / R47 的口径，形状抄 `tests/rcd_supervisor.rs` 的 `StrayGuard`，
//! 但只保留这里用得到的两档）：**凡能留下活 daemon 的用例都必须持有守卫。**
//!
//! 为什么这里也有一份而不是复用那一份：`StrayGuard` 是 `rcd_supervisor.rs` 的私有项
//! （`struct` 没有 `pub`），把它提出来要么改那边那个已经收敛了三轮的文件（100 行 diff 是 churn
//! 不是整洁），要么新造一个 `tests/` 公共模块——这个模块就是后者。这里要的比那边少：
//! 这里的 daemon 一律由 `RcdSupervisor::start` 交出**自己的** PID，所以第一档按 PID 认
//! （父进程收自己的子进程，没有撞同行的风险），只有 `start()` 连句柄都没交出来那条路走端口档，
//! 而端口档照抄 R46 的两条判据（命令行带本用例的 fixture 目录 + exe 取自**起这个 daemon 的那份
//! spec**，都不是文件级常量）。
//!
//! 端口每次跑现挑（见下面 `PORT`），与生产那个号不同：夹具与生产同号会互相顶掉，而它们
//! 本来就是要能同时起来的。生产那一侧的号来自 `deck_port()`（名义默认 `AppConfig::PORT`）。

use std::path::{Path, PathBuf};
use std::time::Duration;

use rclone_deck_lib::config::jobs::{Defaults, Job, JobKind};
use std::sync::LazyLock;

use rclone_deck_lib::config::AppConfig;
use rclone_deck_lib::rc::types::StatsView;
use rclone_deck_lib::rcd::{pidfile, RcdError, RcdSpec, RcdSupervisor};
use rclone_deck_lib::runner::{Runner, TaskState};

/// 夹具 daemon 的端口：**挑一个当下绑得上的**，不是钉死一个号。
///
/// 这一条被 Windows 咬过两次，两次的病因一模一样：
///   2026-10-09  5598 与当时的默认 5599 同时落在动态排除区（5460–5659）→ 三个 target 整组红
///   2026-10-10  排除区漂到 6383–6482，把当时钉的 6399 一起吃了 → 同样整组红
/// 第一次的修法是"挪号"（5598 → 6399）。那只是把同一颗地雷挪了个位置 —— 排除区会继续漂，
/// 24 小时后就又踩上了。所以这次改成**每次跑现挑**。
///
/// 为什么「试一下能不能绑」就够：被占住 bind 失败，**落在排除区里 bind 同样失败**
/// （WSAEACCES，`An attempt was made to access a socket in a way forbidden by its
/// access permissions`）—— 两种病因在判据上是同一件事，不用去解析 `netsh` 那两张表，
/// 也不会漏掉第三种。
///
/// 仍然**不读 `RCLONE_DECK_PORT`**：那个出口是给生产的，套件要的是"与生产不同"，
/// 免得两边的 daemon 抢同一个号。候选窗口 15000–49000 里没有生产的 6400。
/// （`tools/free-rcd-port.ps1` 是面板启动器那边的同一件事，Windows 侧要跑 PowerShell，
/// 这里在测试里绑一下更省。）
///
/// `LazyLock` 保证**一个测试进程里只挑一次**：五条起真后台的 target 各是独立进程，
/// 靠 cargo 串行跑各 target + `--test-threads=1` 互斥（这一点 Cargo.toml 里写着）。
pub static PORT: LazyLock<u16> = LazyLock::new(|| {
    for p in 15000..=49000u16 {
        // 绑上就立刻放掉：我们要的是"这个号现在归我们"，不是"占着它"。
        // 之后 rclone 真去绑时那一下由 `runner_local` 的等待循环兜底。
        if std::net::TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return p;
        }
    }
    panic!("15000–49000 里没有一个绑得上的号：这台机器的端口比想象中还满");
});

pub fn rclone_bin() -> String {
    std::env::var("RCLONE_BIN").unwrap_or_else(|_| {
        "C:\\Users\\Administrator\\AppData\\Local\\Microsoft\\WinGet\\Links\\rclone.exe".into()
    })
}

pub fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 裁定 R47 item 3：`RcdError` 每个变体都携着**生产写进去的字符串**（`Io(msg)`、
/// `SpawnFailed(msg)`、`NotHealthy(_, detail)` 的 detail 是从 rclone 的 stderr 读来的），
/// 所以 `panic!("rcd 起不来：{e}")` 安不安全取决于生产那一刻往字段里放了什么——
/// 正是 `7fc1725` 关掉的那类口子。这里只报**变体名**，够定位"跑到哪个出口了"，不含内容。
pub fn err_variant(e: &RcdError) -> &'static str {
    match e {
        RcdError::PortBusyByForeignProcess { .. } => "PortBusyByForeignProcess",
        RcdError::OrphanNotReapable { .. } => "OrphanNotReapable",
        RcdError::SpawnFailed(_) => "SpawnFailed",
        RcdError::NotHealthy(..) => "NotHealthy",
        RcdError::Io(_) => "Io",
    }
}

/// 路径比较的统一口径（抄 `rcd_supervisor.rs` 的同一个函数，同一条理由）：剥引号、
/// 大小写不敏感、`/` 与 `\` 等价；**不做 canonicalize**——两侧都是本用例自己交出去的
/// 同一段字符串，canonicalize 只会让"一侧有 `\\?\` 前缀一侧没有"变成永远对不上。
pub fn norm_path(p: &str) -> String {
    p.trim_matches('"').to_lowercase().replace('/', "\\")
}

/// 共享夹具的守卫。**`Drop` 里一条 `unwrap`/`expect` 都没有**：守卫自己 panic 会把展开变成中止。
pub struct DaemonGuard {
    /// 起这个 daemon 的那份 spec 的 `rclone_bin`（裁定 R47 item 1：两档身份都以它为准，
    /// 不读文件级 helper `rclone_bin()`——spec 可以从配置派生，届时两者可能不是一回事，
    /// 而身份判据必须跟着"起它的那份 spec"走，否则会比着另一个可执行文件认领）。
    expected_exe: PathBuf,
    /// 本用例的 fixture 目录（`fx.root()`）：候选的命令行必须带着它才算"是我起的"
    /// （裁定 R46 item 1）。**不**从 `pid_path.parent()` 取——这里的 pid 文件是跨测试固定的
    /// （见 `setup_with_conf`），父目录是 `%TEMP%` 本身，那条判据会宽到命中任何命令行。
    fixture_dir: PathBuf,
    pid_path: PathBuf,
    /// `start()` 交出的 PID。`None` = 连句柄都没拿到，只走端口档。
    pid: Option<u32>,
}

impl DaemonGuard {
    /// 建在 `start()` **之前**：那条路失败时手上没有句柄，`spawn` 与 `pidfile::write` 之间
    /// 的窗口也得有人收尾（裁定 R45 item 2 + R47 item 2）。拿到句柄后由 `arm` 升到句柄档。
    pub fn before_start(spec: &RcdSpec, fixture_dir: &Path) -> Self {
        Self {
            expected_exe: spec.rclone_bin.clone(),
            fixture_dir: fixture_dir.to_path_buf(),
            pid_path: spec.pid_path.clone(),
            pid: None,
        }
    }

    /// 升到句柄档（`start()` 交出了 PID）：此后 Drop 走"父进程收自己的子进程"那一档。
    pub fn arm(&mut self, pid: u32) {
        self.pid = Some(pid);
    }

    /// R48 item 3 的两段式夹具要的那只手：**在用例-body 里**（不是在 Drop 里）提前收掉本用例
    /// 自己的 daemon。守卫本来就存着它的 PID，`kill_pid` 在 `test-helpers` 里，
    /// 所以"提交成功之后 daemon 没了"这条缝不需要新开任何生产口子。
    /// `expect` 在这里是安全的：调用点不在展开路径上，panic 就是这条用例自己红。
    pub fn kill_daemon(&self) {
        let pid = self.pid.expect("daemon 没起来就没有可收的进程");
        self.reap(pid);
    }

    /// 三重身份判定：① 映像就是那份 spec 的 `rclone_bin`（完整路径，不是文件名——本机装着
    /// 两个不同构建的 `rclone.exe`）、② 子命令是 `rcd`、③ `--rc-addr` 指着本文件的端口。
    /// 判定完绝不把命令行打印出去：按不变量它不含凭据，但摊到终端只是给未来的回归留门。
    pub fn cmd_is_our_rcd(cmd: &[String], expected_exe: &Path) -> bool {
        let Some(first) = cmd.first() else { return false }; // 进程已不在（或这个 PID 从未被发出）
        if norm_path(first) != norm_path(&expected_exe.display().to_string()) {
            return false;
        }
        let joined = cmd.join(" ").to_lowercase();
        cmd.iter().any(|a| a.eq_ignore_ascii_case("rcd"))
            && joined.contains(&format!("127.0.0.1:{}", *PORT))
    }

    /// 归属判据：候选的 argv 里必须有一个**路径分量**落在本用例的 fixture 目录下。
    /// 本用例那份 spec 的 `--log-file` 就写在那个 TempDir 里，所以我们起的那个 daemon
    /// 必然带着它；别的面板、别的用例的 TempDir 都带不出**这个**目录。
    /// 按分量比而不是子串：兄弟 TempDir 的名字可能互为前缀（`.tmpabcd` 与 `.tmpabcd1234`）。
    pub fn cmd_carries_dir(cmd: &[String], dir: &Path) -> bool {
        if !dir.is_absolute() {
            return false; // 不是一个具体的目录 = 判据不成立，那就什么都不做
        }
        let dir_norm = norm_path(&dir.display().to_string());
        !dir_norm.is_empty() && cmd.iter().any(|a| {
            let arg = norm_path(a);
            Path::new(&arg).starts_with(Path::new(&dir_norm))
        })
    }

    /// 按 PID 终结 + 阻塞轮询到它真从进程表里消失（否则端口还绑着，下一条用例的 `start`
    /// 会撞上"端口被占 + 记录里的 panel_pid 还活着（就是本进程）"的窗口）。
    pub fn reap(&self, pid: u32) {
        if !rclone_deck_lib::rcd::kill_pid(pid) {
            eprintln!("runner 守卫无法终结 PID {pid}：{} 上可能留着一个 daemon", *PORT);
            return;
        }
        for _ in 0..200 {
            if rclone_deck_lib::rcd::command_line_of(pid).is_empty() {
                // 守卫在 `Drop` 里、不许 panic，但清理失败也不许静默（裁定 R56 item 3）：说出来。
                if let Err(e) = pidfile::remove(&self.pid_path) {
                    eprintln!("runner 守卫删不掉 pid 记录 {}：{e}", self.pid_path.display());
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!("runner 守卫终结 PID {pid} 之后它仍在进程表里：{} 可能还被占着", *PORT);
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        // 句柄档（每一条跑到 `start()` 返回的用例）：PID 是我们自己的子进程，
        // 身份判定仍然要过——PID 会被系统复用，"这个 PID 还活着"推不出"还是那个 rclone"。
        // 正常路径上测试已经 `shutdown_rcd()`，进程早就不在了，这里是一条都不命中的空转。
        if let Some(pid) = self.pid {
            let cmd = rclone_deck_lib::rcd::command_line_of(pid);
            if cmd.is_empty() {
                return;
            }
            if Self::cmd_is_our_rcd(&cmd, &self.expected_exe)
                && Self::cmd_carries_dir(&cmd, &self.fixture_dir)
            {
                self.reap(pid);
            }
            return;
        }
        // 端口档（`start()` 返回 Err、手上没有句柄那条路）：照抄 R46 的两条闸——
        // ① 只在展开路径上动手（正常结束时端口上若挂着另一个面板的真 daemon，去碰它就是
        //    R42 item 3 明令禁止的那种截断）；② 候选必须带着本用例的 fixture 目录。
        // 任一条不成立就什么都不做，让下一条用例的 `reclaim` 去报 `PortBusy…`。
        if !std::thread::panicking() {
            return;
        }
        if let Some(pid) = rclone_deck_lib::rcd::port_listener_pid(*PORT) {
            let cmd = rclone_deck_lib::rcd::command_line_of(pid);
            if Self::cmd_is_our_rcd(&cmd, &self.expected_exe)
                && Self::cmd_carries_dir(&cmd, &self.fixture_dir)
            {
                self.reap(pid);
            }
        }
    }
}

pub struct Fx {
    root: tempfile::TempDir,
}

impl Fx {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::create_dir_all(root.path().join("dst")).unwrap();
        let fx = Self { root };
        fx.write_conf();
        fx
    }
    pub fn root(&self) -> &Path { self.root.path() }
    pub fn src(&self) -> PathBuf { self.root.path().join("src") }
    pub fn dst(&self) -> PathBuf { self.root.path().join("dst") }

    /// 裁定 R30：gate / prune 的 `dst` **必须**是具名 remote。
    /// `remote_for_list()`（会 `Err` 的那条推导路径）拒绝任何不带 `name:` 的写法，
    /// 所以"把本地临时目录直接当 dst"的测试根本跑不起来——而这恰恰是不能放宽的地方：
    /// 本地盘在 rclone 里也是一个 fs，`dst` 写成本地路径时"远端清单"会退化成"再扫一遍本地"，
    /// 对账永远通过、清理永远没有孤儿（那是这次事故的同族假绿）。
    ///
    /// 夹具 therefore 自带一份**临时 rclone.conf**，把 `dst` 目录注册成一个具名 remote：
    /// ```text
    /// [decklocal]
    /// type = alias
    /// remote = C:\...\泰mp\xxx\dst
    /// ```
    /// 实测（v1.75.1，`RCLONE_CONFIG` 指到这份 conf）：
    /// - `operations/list fs="decklocal:" remote=""` → 该目录的顶层，目录项带 `IsDir`；
    /// - `fs="decklocal:" remote="sub"` → `Path` 已经带 `sub/` 前缀，与 `normalize_actual` 的口径一致；
    /// - 于是 `job.dst = "decklocal:"` 时 `remote_for_list()` = `""`、`prefix()` = `""`，
    ///   期望侧走本地 `dst` 目录得到的 key 与实际侧**同一坐标系**。
    /// - `job.dst = "decklocal:ideahub/covers"` 时 `remote_for_list()` = `"covers"`、
    ///   `prefix()` = `"covers/"`、`fs_for_list()` = `"decklocal:ideahub"`：实际侧的 `Path` 是
    ///   相对那个 fs 根的 `covers/...`，期望侧拼出来的也是 `covers/...`。**同一个坐标系**——
    ///   这正是 `prefix()` 与 `remote_for_list()` 必须对上号的证人（对不上的话 gate 会把
    ///   两侧当成两个不相干的集合，缺图清单等于整个期望集合）。
    ///
    /// **为什么是 `alias` 而不是 `local`**（这条是被实测逼出来的）：
    /// `[x] type = local` 配 `path = <目录>` 会被 rclone **忽略**，远端直接按**进程 CWD** 落根——
    /// 当时那条 `lsf` 把整个 `%TEMP%` 列了出来（94MB 清单，且一声不响）。
    /// 用 `local` 型夹具会让测试"看起来通过"而实际在对账一个完全无关的目录，
    /// 比测试跑不起来危险得多。
    pub fn write_conf(&self) {
        let conf = self.root.path().join("rclone.conf");
        let body = format!("[decklocal]\ntype = alias\nremote = {}\n", fwd(&self.dst()));
        std::fs::write(&conf, body).unwrap();
    }
    /// 给 `RcdSpec.rclone_config` 用的那份 conf（生产路径传 `None`，用 rclone 自己的默认位置）。
    pub fn conf(&self) -> PathBuf { self.root.path().join("rclone.conf") }
    /// 具名 remote 的 `dst` 写法：`decklocal:` = 根（= `fx.dst()`），
    /// `fx.remote("ideahub/covers")` = 根里面的前缀。**冒号是必需的**——
    /// 少了它 `remote_split()` 就不认，`remote_for_list()` 会（正确地）拒绝。
    pub fn remote(&self, sub: &str) -> String { format!("decklocal:{sub}") }
}

/// 返回 `(Runner, 守卫)`：守卫活到测试函数结束，展开路径上收尾 daemon（见文件顶部的规则）。
/// 返回 `(Runner, 守卫)`：守卫活到测试函数结束，展开路径上收尾 daemon（见文件顶部的规则）。
pub async fn setup(root: &Path, jobs: Vec<Job>) -> (Runner, DaemonGuard) {
    setup_with_conf_and_exclude(root, None, jobs, vec![]).await
}

/// `rclone_config`：夹具 conf 的路径。生产路径传 `None`（用 rclone 自己的默认位置）。
pub async fn setup_with_conf(
    root: &Path, rclone_config: Option<PathBuf>, jobs: Vec<Job>,
) -> (Runner, DaemonGuard) {
    setup_with_conf_and_exclude(root, rclone_config, jobs, vec![]).await
}

/// 与 `setup_with_conf` 同体，只差两处：`RcdSpec.exclude` 与 `Defaults.exclude` 都换成传进来的那份。
///
/// **两条必须同源**：排除规则只有 rcd 启动参数这一条入口（子项目 1 记过），而面板那道
/// "清空前缀"的闸门读的是配置里的 `Defaults.exclude`。分开给就会"闸门与列表不一致"——
/// 那正是 R74 要拦的形状，拿它当证人等于自己造一个假场景。
/// 上面两个入口的签名不动（只做转发），所以 `runner_local` / `mirror_runner` 的调用点
/// 一个都不必改：那两个目标各有自己的证人，动它们的夹具形状等于动它们的断言。
pub async fn setup_with_conf_and_exclude(
    root: &Path, rclone_config: Option<PathBuf>, jobs: Vec<Job>, exclude: Vec<String>,
) -> (Runner, DaemonGuard) {
    setup_full(root, rclone_config, jobs, exclude, vec![]).await
}

/// 与上面两个入口同体，只差 `flags` 那一串：它**同时**喂给 rcd 的启动参数与 runner 那份
/// `Defaults`（生产里那是同一个 `shared_defaults` 的两个派生值，见 `app_state.rs` 那条证人）。
/// 分开给就等于自己造一个假场景：rcd 按新比法跑、面板却按旧的说话。
/// 存在的理由只有一条：`--size-only` 那一枚改的是每一趟的判定口径，而"日志与顶栏说的是在跑的那种"
/// 只能在真的带上它的 daemon 上验。
pub async fn setup_with_flags(
    root: &Path, jobs: Vec<Job>, flags: Vec<String>,
) -> (Runner, DaemonGuard) {
    setup_full(root, None, jobs, vec![], flags).await
}

#[allow(clippy::too_many_arguments)]
async fn setup_full(
    root: &Path, rclone_config: Option<PathBuf>, jobs: Vec<Job>,
    exclude: Vec<String>, flags: Vec<String>,
) -> (Runner, DaemonGuard) {
    let spec = RcdSpec {
        rclone_bin: PathBuf::from(rclone_bin()),
        port: *PORT,
        flags: flags.clone(),
        exclude: exclude.clone(),
        rclone_config,
        log_file: Some(root.join("rcd.log")),
        // 固定位置：泄漏的 rcd 才能被下一个测试认领并回收
        pid_path: std::env::temp_dir().join("rclone-deck-runner-test.pid"),
        log_level_debug: false,
    };
    let mut guard = DaemonGuard::before_start(&spec, root);
    let handle = std::sync::Arc::new(
        RcdSupervisor::start(spec.clone()).await
            .unwrap_or_else(|e| panic!("rcd 起不来：{}", err_variant(&e))),
    );
    guard.arm(handle.pid);
    let app = AppConfig {
        rclone_bin: PathBuf::from(rclone_bin()),
        port: *PORT,
        log_dir: root.join("logs"),
        pid_path: std::env::temp_dir().join("rclone-deck-runner-test.pid"),
    };
    let defaults = Defaults {
        rclone_bin: rclone_bin(),
        flags,
        exclude,
        log_dir: fwd(&root.join("logs")),
    };
    (Runner::new(handle, app, defaults, jobs), guard)
}

pub fn job(id: &str, kind: JobKind, src: &Path, dst: &Path) -> Job {
    Job {
        id: id.into(), name: id.into(), kind,
        src: fwd(src), dst: fwd(dst),
        open_dir: None, manifest: None,
    }
}

/// 与 `job()` 同形，但 src/dst 是**字符串**：对齐的两侧之一是 remote，`&Path` 装不下。
pub fn job_at(id: &str, kind: JobKind, src: &str, dst: &str) -> Job {
    Job {
        id: id.into(), name: id.into(), kind,
        src: src.into(), dst: dst.into(), open_dir: None, manifest: None,
    }
}

/// 往一个目录里写几个空文件，返回它们的相对名（相对该目录，正斜杠）。
pub fn seed(dir: &Path, names: &[&str]) -> Vec<String> {
    for n in names {
        let p = dir.join(n.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
    }
    names.iter().map(|s| s.to_string()).collect()
}

/// 列出目录里的普通文件（递归，正斜杠相对名）。用来独立核对"到底少没少"。
pub fn files_in(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() { stack.push(p); }
            else { out.push(p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/")); }
        }
    }
    out.sort();
    out
}

pub fn empty_stats() -> StatsView {
    StatsView {
        transfers: 0, total_transfers: 0, checks: 0, total_checks: 0,
        bytes: 0, total_bytes: 0, speed: 0.0, eta: None, errors: 0,
        deletes: 0, deleted_dirs: 0, retry_error: false, fatal_error: false,
        elapsed_time: 0.0,
    }
}

pub fn stats_of(st: TaskState) -> StatsView {
    match st {
        TaskState::Done { stats, .. } => stats,
        other => panic!("期望 Done，实际 {other:?}"),
    }
}

/// 等到 `transfer` 把状态推到 `Running`——那一刻**提交已经返回**（jobid 已经在 rcd 手里），
/// 这正是 R48 那几条证人要站的时间点。用它而不是 `sleep` 猜时序：状态就是免费的 observable，
/// 而"提交之后"这件事在测试里唯一可辨识的形状就是"状态里出现了 jobid"。
pub async fn wait_until_running(r: &Runner, id: &str) {
    for _ in 0..2000 {
        if matches!(r.state(id).await, TaskState::Running { .. }) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("等了 10 秒 {id} 都没进 Running：提交根本没成功，夹具的前提不成立");
}
