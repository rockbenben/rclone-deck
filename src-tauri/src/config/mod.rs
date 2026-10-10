pub mod edit;
pub mod jobs;

use std::path::PathBuf;

use jobs::Defaults;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("jobs.toml 解析失败：{0}")]
    Parse(String),
    #[error("{0}")]
    Invalid(String),
    #[error("读写配置失败：{0}")]
    Io(#[from] std::io::Error),
}

impl serde::Serialize for ConfigError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// 面板自己的运行参数。不含任何密钥。
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub rclone_bin: PathBuf,
    pub port: u16,
    pub log_dir: PathBuf,
    pub pid_path: PathBuf,
}

/// 面板自己的落点开关：`RCLONE_DECK_HOME`。见裁定 R52。
///
/// 设了它，面板自己的**两个落点**（`jobs_toml_path()` 与 `from_defaults()` 里的 `pid_path`）
/// 都从它派生，且共用同一个根：`$HOME/jobs.toml` + `$HOME/rcd.pid`。
/// 两处必须同源——否则夹具窗口会读夹具的 `jobs.toml`、却把 pid 写进操作员目录，
/// 那是比现状更糟的形状。选 `$HOME/...` 而不是 `$HOME/rclone-deck/...`：
/// 变量本身已经叫 DECK_HOME，再套一层 rclone-deck 是冗余，便携配置时也少一级目录。
///
/// 没设 → 一切照旧走已知目录（`%APPDATA%/rclone-deck`、`%LOCALAPPDATA%/rclone-deck`），
/// 行为与今天完全一致。
///
/// 空串按**报错**处理，不当没设：`set RCLONE_DECK_HOME=` 或空展开出来的半设置，
/// 若静默回落到操作员真配置，正是这条开关要挡的事故；要回落就彻底取消该变量。
///
/// 读的是 `std::env::var`（调用时），不是 `option_env!`（编译期）——夹具窗口是运行时才设的。
/// 这不是测试专用开关，操作员拿它做便携配置走的是同一条路径，故不藏进 feature 门。
/// 它只改根，不改校验：`rclone_bin` / `log_dir` 的绝对路径校验照旧在 `jobs.toml` 里管。
fn deck_home() -> Result<Option<PathBuf>, ConfigError> {
    match std::env::var("RCLONE_DECK_HOME") {
        Err(_) => Ok(None),
        Ok(s) if s.is_empty() => Err(ConfigError::Invalid(
            "RCLONE_DECK_HOME 被设成空串：要么给出绝对路径，要么彻底取消该变量——空串不是「没设」".into(),
        )),
        Ok(s) => {
            let p = PathBuf::from(s);
            if !p.is_absolute() {
                return Err(ConfigError::Invalid(format!(
                    "RCLONE_DECK_HOME 必须是绝对路径：相对路径会跟着 CWD 漂（那个 type=local 夹具把整个 %TEMP% 列出来就是这个形状），\
                     而它的下游是「读哪份 jobs.toml」和「收哪个 rcd.pid」。当前值 {:?} 不是绝对路径",
                    p
                )));
            }
            Ok(Some(p))
        }
    }
}

/// 后台端口的显式出口：`RCLONE_DECK_PORT`。读的是 `std::env::var`（调用时），同 `deck_home`。
///
/// **为什么需要这个出口**（2026-10-09 实测）：本机 Windows 的**动态排除端口区**会漂。
/// 有一轮它正好盖住 5460–5659，把当时的默认 5599 一起吃了，于是 `rclone rcd` 报
/// `bind: An attempt was made to access a socket in a way forbidden by its access permissions`
/// —— 那不是"端口被占"，是"这个端口不许绑"，面板从此起不来后台（读数在 `logs/rcd.log` 里，
/// 第一行就是那句 bind）。仓里已有同族处置：CDP 那个端口当初也是被排除区盖住，解法是
/// `--remote-debugging-port=0` 让系统自己挑；这里**不能照抄**那个解法 —— 端口要写进 pid 记录、
/// 还要出现在错误话术与「详情」里，得是个具体的数，所以只能给一个显式出口。
/// 默认值随后从 5599 抬到 **6400**（理由见 `AppConfig::PORT` 上那段），但**抬默认值不是根治**：
/// 排除区会继续漂。这个出口才是。
///
/// **设了它就等于放弃跨进程单实例那道闸**（裁定 R42 item 3 的前提是"端口对同一用户是常量"，
/// 见 `rcd::reclaim` 上面那段）：两个面板各占一个端口时谁也看不见谁，于是它们会各起一个 rcd。
/// 想要彻底隔离，得**连 `RCLONE_DECK_HOME` 一起设** —— pid 记录跟着 home 走，那边才是分家的地方。
/// 不设它时操作员的日常路径一个字节都不变。
/// 一个端口号今天的状态。分三档是因为"绑不上"有两种，处置**完全相反**：
/// 有人在听 → 按老规矩报错退出（并说出占用者）；没人听但不让绑 → 那才该换号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PortState {
    Free,
    Taken,
    Unusable,
}

/// 端口从哪来：显式设的 `RCLONE_DECK_PORT` 优先，否则**现挑一个**（见 `AppConfig::pick_port`）。
///
/// **2026-10-10 的变化**：以前这里不设就直接交回 `AppConfig::PORT` 那个常量，而那个常量已经
/// 被这台机器的排除区（6383–6482）吃了 —— 于是新用户装上面板第一次启动必然起不来。
/// "抬高默认值"救不了（它被咬过两次），所以默认这一档也改成现挑。
fn deck_port() -> Result<u16, ConfigError> {
    match std::env::var("RCLONE_DECK_PORT") {
        Err(_) => Ok(AppConfig::pick_port()),
        // 空串按**报错**处理，与 `RCLONE_DECK_HOME` 同一纪律：半设置静默回落到默认端口，
        // 落到的正是那条"本该被躲开的、这轮正好被排除区吃掉的"端口。
        Ok(s) if s.trim().is_empty() => Err(ConfigError::Invalid(
            "RCLONE_DECK_PORT 被设成空串：要么给一个端口号，要么彻底取消该变量——空串不是「没设」".into(),
        )),
        Ok(s) => s.trim().parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(|| {
            ConfigError::Invalid(format!(
                "RCLONE_DECK_PORT 要一个 1..=65535 的端口号，当前值 {s:?} 不是。\
                 0 也不行：rcd 的 --rc-addr 要一个固定的回环地址，而端口号要写进 pid 记录与错误话术，\
                 「让系统自己挑一个」在这里是行不通的（CDP 那个端口能这么干是因为没人读它的具体值）"
            ))
        }),
    }
}

impl AppConfig {
    /// 端口的**名义默认**。`RCLONE_DECK_PORT` 覆盖它（见 `deck_port`）。
    ///
    /// **它不再是"一定会被用的那个号"** —— 2026-10-10 起 `deck_port()` 在它**绑不上**时会
    /// 另挑一个（理由见 `pick_port`）。留着这个常量是为了：文档、错误话术、顶栏「详情」有
    /// 一处可引用的名，以及测试有一条"名义上下界"的判据。
    ///
    /// **2026-10-09 从 5599 抬到 6400**：起因是排除区漂到 5460–5659 把 5599 吃了。抬的依据
    /// 是两条边界，**不是"越大越好"**：
    ///   * **下界**：本机实测过的排除区最高到 6139（3662–4161 / 5460–5659 / 5688–5787 /
    ///     5940–6139 四段，都是 Hyper-V/WSL 那一类动态预留）。
    ///   * **上界**：Windows 的**临时（动态客户端）端口区**从 49152 起。固定的服务端口绝不能进
    ///     那一段 —— 出站连接随手就能把那号抓走，报出来的还是一句"端口被占"，而病因与 rcd 无关。
    ///
    /// **但抬高没有根治**：排除区继续往上漂，2026-10-10 就漂到 **6383–6482** 把这里整个吃了 ——
    /// 10-09 那次抬号只管了一天。这是它第二次被咬，所以下面改成"认得绑不上的两种原因"。
    pub const PORT: u16 = 6400;

    /// 挑一个今天真用得上的端口。
    ///
    /// **只换一种**：`PortState::Unusable`（没人占，但 Windows 不许绑 = 动态排除区）。
    /// 另一种 `Taken`（有活服务在听）**不换**，交回名义默认，让启动那一步按 `rcd::reclaim`
    /// 的老规矩自证并把"是谁占着"说出来 —— 那句话里有操作员要的东西，换号会把它吞掉
    /// （`rcd/mod.rs` 那段明写"也不静默换端口"）。
    ///
    /// **扫描起点固定、逐个往后，这保证同一台机器上多个面板算出同一个号。** 排除区是机器的
    /// 属性而不是"谁在跑"的属性，所以两次算出来一样，单实例那道闸（`reclaim` 靠 pid 记录
    /// 认领）继续成立。若改成"随便找个空的"，两个面板会各拿一个号、谁也看不见谁，于是起
    /// 两个 rcd —— 那正是 R42 item 3 要挡的形状。
    fn pick_port() -> u16 {
        match Self::port_state(AppConfig::PORT) {
            PortState::Free | PortState::Taken => AppConfig::PORT,
            PortState::Unusable => (15000..=49000u16)
                .find(|p| Self::port_state(*p) == PortState::Free)
                // 一个都没挑出来：交回名义默认，让启动那一步按它自己的话术去报
                // （在这里 panic 只会给操作员一句看不懂的英文）。
                .unwrap_or(AppConfig::PORT),
        }
    }

    /// 判"这个号今天能不能用" —— **因为"绑不上"有两种，处置完全不同**：
    ///
    /// * `Free` —— 绑得上，直接用。
    /// * `Taken` —— 绑不上但**有人在听**：这是"端口被占"，走 `reclaim` 那套老规矩
    ///   （自证是不是我们上次留下的 rcd，是就终结重开，不是就报错并说出占用者是谁）。
    /// * `Unusable` —— 绑不上、也没人听：**动态排除区**，Windows 不许任何进程绑它
    ///   （`bind` 报 WSAEACCES）。这一档换号才对，不换面板永远起不来。
    ///
    /// 2026-10-10 实测：排除区漂到 6383–6482，把 6400 整个吃了。
    fn port_state(port: u16) -> PortState {
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(_) => PortState::Free,
            Err(_) => match std::net::TcpStream::connect(("127.0.0.1", port)) {
                Ok(_) => PortState::Taken,
                Err(_) => PortState::Unusable,
            },
        }
    }

    pub fn from_defaults(d: &Defaults) -> Result<Self, ConfigError> {
        let rclone_bin = PathBuf::from(&d.rclone_bin);
        if !rclone_bin.is_absolute() {
            return Err(ConfigError::Invalid(
                "rclone_bin 必须是绝对路径：本机 PATH 上有两个 rclone，靠 PATH 顺序选是赌博".into(),
            ));
        }
        // 落点同源：开了 RCLONE_DECK_HOME 就用它，否则照旧 %LOCALAPPDATA%/rclone-deck。
        // 日志目录与 pid 文件**共用这一个根**（2026-10-09 起）。
        let home = match deck_home()? {
            Some(home) => home,
            None => dirs::data_local_dir()
                .ok_or_else(|| ConfigError::Invalid("无法解析 %LOCALAPPDATA%".into()))?
                .join("rclone-deck"),
        };
        // `log_dir` 留空/不写 = 用 `home\logs`。**为什么允许空**：`jobs.toml` 是一份**静态文本**，
        // 里面没有 `%LOCALAPPDATA%` 可展开 —— 出厂那份不管写什么字面量，对别人都不是他的家
        // （改之前写死的是作者那台的 `D:\Sync\logs\...`，没有 D 盘的新用户连启动都过不去）。
        // 留空则由这里按**当前用户**算出来，谁装都成立，而夹具家（RCLONE_DECK_HOME）也自动跟着走。
        let log_dir = if d.log_dir.trim().is_empty() {
            home.join("logs")
        } else {
            let p = PathBuf::from(&d.log_dir);
            if !p.is_absolute() {
                return Err(ConfigError::Invalid("log_dir 必须是绝对路径（或者留空用默认落点）".into()));
            }
            p
        };
        let pid_path = home.join("rcd.pid");
        Ok(Self { rclone_bin, port: deck_port()?, log_dir, pid_path })
    }

    /// jobs.toml 的位置：开了 `RCLONE_DECK_HOME` 则是 `$HOME/jobs.toml`，
    /// 否则照旧 `%APPDATA%/rclone-deck/jobs.toml`。见 `deck_home` 注释（R52）。
    pub fn jobs_toml_path() -> Result<PathBuf, ConfigError> {
        if let Some(home) = deck_home()? {
            return Ok(home.join("jobs.toml"));
        }
        let base = dirs::config_dir()
            .ok_or_else(|| ConfigError::Invalid("无法解析 %APPDATA%".into()))?
            .join("rclone-deck");
        Ok(base.join("jobs.toml"))
    }

    pub fn load_jobs() -> Result<jobs::JobsFile, ConfigError> {
        let path = Self::jobs_toml_path()?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ConfigError::Io(std::io::Error::new(
                e.kind(), format!("读 {} 失败：{e}", path.display()))))?;
        jobs::JobsFile::parse(&text)
    }
}
