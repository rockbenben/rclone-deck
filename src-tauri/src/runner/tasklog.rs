use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

pub struct TaskLog {
    path: PathBuf,
    secrets: Vec<String>,
}

impl TaskLog {
    /// `secrets` 是要按值擦掉的确切串，由调用方从 `RcdHandle::secrets()` 传入。
    /// 契约（调用方必须看得见）：**短于 4 个字符的串完全不擦**。下限只剩一处，写在正典
    /// `crate::rcd::redact_credentials` 里（裁定 R44 把排序和这个下限一起收成那一份，
    /// `scrub` 从此只转发），但它守的是**本函数的契约**：递进来的必须是 `random_credentials()`
    /// 那种凭据（`deck-<8hex>` = 13 字符，口令是 32 位 hex），不是任意短口令。
    /// 传一个短口令进来等于零保护，`tail()` 会把它原样显示在界面上——要改的是调用方，
    /// 不是正典里那个数。
    /// 这个下限是明摆着的交易，不是遗漏：没有它，一个单字母密钥就能把每一行都变成 `[REDACTED]`。
    pub fn create(log_dir: &Path, job_id: &str, secrets: &[String]) -> std::io::Result<Self> {
        std::fs::create_dir_all(log_dir)?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let mut path = log_dir.join(format!("{job_id}-{stamp}.log"));
        for n in 1.. {
            if !path.exists() {
                break;
            }
            path = log_dir.join(format!("{job_id}-{stamp}-{n}.log"));
        }
        File::create(&path)?;
        Ok(Self { path, secrets: secrets.to_vec() })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn line(&self, level: Level, msg: &str) -> std::io::Result<()> {
        self.write(level, msg, &[])
    }

    pub fn kv(&self, level: Level, msg: &str, fields: &[(&str, String)]) -> std::io::Result<()> {
        self.write(level, msg, fields)
    }

    /// 读日志尾。给界面看的是"最后若干字节的完整行"，所以砍掉开头那半行。
    ///
    /// R40（本任务执行期量出来的两条"界面上看不到日志"的形状，都是**已发布**的缺陷）：
    /// `max_bytes` 是界面的**软预算**，不是"宁可少给"的理由——这条模块存在的意义是事后能回溯，
    /// 而删除清单就是一行一条 key，最坏情况正好是"一条超长行"。原写法会把最新那一行藏起来两次：
    /// 1. 窗口起点**正好落在行边界**上时，`find('\n')` 丢掉的是第一条**完整**行而不是半行。
    ///    分不清这两种，只能回头看窗口前那一个字节是不是换行（见 `starts_mid_line`）。
    /// 2. 最新那一行**比预算还长**时，窗口里没有换行，原写法返回 `""`——
    ///    "这次运行写了一条超长行"于是渲染成"没有日志"，正好推翻了模块的立身之本。
    pub fn tail(path: &Path, max_bytes: u64) -> std::io::Result<String> {
        let mut f = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
            Err(e) => return Err(e),
        };
        let len = f.metadata()?.len();
        let start = len.saturating_sub(max_bytes);
        // 丢不丢第一条，取决于起点在不在行中间：看它前一个字节。`start == 0` 是文件开头，
        // 那里没有"半行"这回事，所以直接算落在行边界。
        let starts_mid_line = start > 0 && {
            let mut prev = [0u8; 1];
            f.seek(SeekFrom::Start(start - 1))?;
            f.read_exact(&mut prev)?;
            prev[0] != b'\n'
        };
        f.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let text = String::from_utf8_lossy(&buf).into_owned();
        if !starts_mid_line {
            // 起点在行边界（或整份文件都在预算里）：一行都不许丢。
            return Ok(text);
        }
        match text.find('\n') {
            // 起点落在行中间：只丢开头那半行（旧内容），其后全部保留。
            Some(i) if i + 1 < text.len() => Ok(text[i + 1..].to_string()),
            // 窗口里没有换行（或只有这一行的尾巴）= 最新那一行比预算还长：返回窗口里这段
            // （它是那一行的**后半**，不是整行——预算就那么多），而不是空串。
            // 宁可给半行，也别给"没有日志"。
            _ => Ok(text),
        }
    }

    fn write(&self, level: Level, msg: &str, fields: &[(&str, String)]) -> std::io::Result<()> {
        let mut out = format!(
            "{} {:5} {}",
            chrono::Local::now().to_rfc3339(),
            level.as_str(),
            msg
        );
        for (k, v) in fields {
            out.push(' ');
            out.push_str(k);
            out.push('=');
            out.push_str(v);
        }
        out.push('\n');

        // 持久性只到**操作系统缓存**，不到盘片：每行直写下去（`File` 没有用户态缓冲，
        // 句柄写完即关），但删除阶段中途掉电仍可能丢掉最新的几行——日志可能比实际删掉的少。
        // 是否在写完删除清单后 `sync_all()` 一次，是 Task 9 的决定，这里不替它做。
        let mut f = OpenOptions::new().append(true).open(&self.path)?;
        f.write_all(self.scrub(&out).as_bytes())
    }

    /// 擦密的**实现只有一份**（裁定 R44）：`crate::rcd::redact_credentials`。
    /// 这里原来抄了第二遍（自己排降序、自己定 `>= 4` 的下限），两份会在 Task 9 那条
    /// "每个调用方都要擦"的路上分头漂移，所以只剩转发：`self.secrets` 仍是本结构的字段，
    /// 排序与下限都由正典负责，本函数不再自己排一遍、不再自己定门槛。
    fn scrub(&self, text: &str) -> String {
        crate::rcd::redact_credentials(text, &self.secrets.iter().map(|s| s.as_str()).collect::<Vec<_>>())
    }
}
