use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Deserialize;

use crate::endpoint::{endpoint_of, remote_split as parse_remote, Endpoint};
use crate::scope::{has_dot_segment, Scope};
use super::ConfigError;

// AppConfig 定义在 config/mod.rs，本模块的测试要用它（见 tests 模块）。
// 非测试构建里没人引用，所以只在 cfg(test) 下导入，否则 unused_imports。
#[cfg(test)]
use super::AppConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobKind { Copy, Sync, Gate }

impl JobKind {
    /// **配置文件与界面上那条任务的身份词**：`copy` / `sync` / `gate`。
    /// 它是 `jobs.toml` 里写的那个值，也是 `JobView.kind` 交给界面的那一个，也是临时格编号里
    /// 的那一段（`adhoc-1-sync-a`，裁定 R99）—— 三个读者一份定义。
    /// 与 [`JobKind::word`] 的区别：那一句是**给操作员看的人话**（「复制」「让两边对齐（会删）」），
    /// 这一句是数据。两者不许混用：把 `sync` 印到界面上是内部名词上屏（R56），
    /// 把「让两边对齐（会删）」拼进编号是造出一个没人认得的 id。
    pub fn toml_word(&self) -> &'static str {
        match self {
            JobKind::Copy => "copy",
            JobKind::Sync => "sync",
            JobKind::Gate => "gate",
        }
    }

    /// 给**操作员看**的那个词。界面文案与错误话术都用它，不许再用 `{kind:?}` 的 Debug 写法
    /// （`Gate` 是 Rust 的枚举名，不是人话；本项目已经因为内部术语被退回过一次）。
    /// 放在 `config`（最底层）而不是 `state`：`runner` 的拒绝话术也要用同一个口径，
    /// 而 `runner` 不许反过来依赖 `state`。
    pub fn word(&self) -> &'static str {
        match self {
            JobKind::Copy => "复制",
            JobKind::Sync => "让两边对齐（会删）",
            // 2026-10-09：`检查` → `对账`，与前端 `KIND_LABEL.gate` 同步。
            // 这一族本来就到处说「对账」（测试夹具那份清单里的任务名 `ideahub 封面对账`、报错
            // 「闸门无从对账」、设置面「对账的那几条」），只有这个词表与右列那一段
            // 写着「检查」—— 而右列紧挨着已经有一段叫「差异」，两个都像"比一比"。
            // 改的只是**当名词**的用法；`先检查 src 指向与 exclude 是否过宽` 那种
            // 动词（"去看一眼"）在 `gate.rs` / `runner/mod.rs` 里，一律不动。
            JobKind::Gate => "对账",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manifest { LocalDir, File(PathBuf), D1 }

#[derive(Debug, Clone)]
pub struct Defaults {
    pub rclone_bin: String,
    pub flags: Vec<String>,
    pub exclude: Vec<String>,
    pub log_dir: String,
    /// 盘点时**要不要跑哈希那一级**（2026-10-11 新增，默认关）。
    ///
    /// 默认关是硬理由，不是保守：第一级（大小 + 修改时间）已经能抓到绝大多数，
    /// 而第二级是 `operations/check` —— 后端不支持该哈希时它要**下载每一个文件**。
    /// 一个几千张图的库开着它就是分钟级的等待，而绝大多数时候第一级已经说清了。
    /// 所以它必须由人显式打开，而不是默认承担那份代价。
    pub check_by_hash: bool,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub kind: JobKind,
    pub src: String,
    pub dst: String,
    pub open_dir: Option<String>,
    pub manifest: Option<Manifest>,
}

#[derive(Debug, Clone)]
pub struct JobsFile { pub defaults: Defaults, pub jobs: Vec<Job> }

// ---- 只用于反序列化的镜像结构：字段名与 jobs.toml 一致，校验统一在后面做 ----

#[derive(Deserialize)]
struct RawFile {
    defaults: RawDefaults,
    #[serde(rename = "job", default)]
    job: Vec<RawJob>,
}

#[derive(Deserialize)]
struct RawDefaults {
    rclone_bin: String,
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    /// 空串 = 用默认落点（`%LOCALAPPDATA%\rclone-deck\logs`，夹具家跟着 `RCLONE_DECK_HOME` 走）。
    /// 允许不写是因为 `jobs.toml` 是静态文本，里面没有 `%LOCALAPPDATA%` 可展开 ——
    /// 出厂那份写死谁的路径都不是别人的家（见 `config::AppConfig::from_defaults`）。
    #[serde(default)]
    log_dir: String,
    /// 哈希那一级的开关（2026-10-11 新增）。**`default` = false**：旧配置文件里没有这一项，
    /// 而"没有"读成"不开"才是安全的默认 —— 这一级可能要下载每一个文件（见 `Defaults::check_by_hash`）。
    #[serde(default)]
    check_by_hash: bool,
}

#[derive(Deserialize)]
struct RawJob {
    id: String,
    name: String,
    kind: String,
    src: String,
    dst: String,
    #[serde(default)]
    open_dir: Option<String>,
    #[serde(default)]
    manifest: Option<RawManifest>,
}

#[derive(Deserialize)]
struct RawManifest {
    kind: String,
    #[serde(default)]
    path: Option<String>,
}

const BANNED_FLAGS: &[&str] = &[
    "--rc-addr", "--rc-user", "--rc-pass", "--rc-no-auth", "--rc-htpasswd", "--rc-realm",
];

/// 会改变**"什么会被推进桶"**的链接类 flag：一律拒绝。
///
/// 依据是 `rc::listing::normalize_expected` 的那条不变量：期望集合必须等于 rclone 实际会推上去
/// 的那批对象。判据全部是**实测**（`docs/superpowers/evidence/build-notes.md`；本地到本地的
/// `rclone copy <root> <临时目录>`，`root/link` 是真 NTFS junction，钉住的 rclone v1.75.1）：
/// - 默认 / 面板默认 flags：`NOTICE: link: Can't follow symlink without -L/--copy-links`，
///   `link` 那条拼写一个对象都不落地 → 期望侧照此也不跟随，这是两边唯一同源的那一种；
/// - `-L` / `--copy-links` / `-L=true`，**以及合并写法 `-Lv`、`-vL`**：exit=0，落地**多出**
///   `link/a.webp`、`link/deep/c.webp` —— 链接被跟随了，而期望集合按不跟随算；
/// - `-l` / `--links` / `--links=true`：不跟随，但把链接本身**当成一个对象推**，实测的拼写是
///   `link.rclonelink`（这台机器写不出那个对象：`ERROR : link.rclonelink: Failed to copy:
///   symlinkat …`、exit=1，可 rclone 已经在**按这个名字**推了）。写成 `p/link`（"目录里多一个文件"）
///   是错的：rclone 的写法是加 `.rclonelink` 后缀（`rclone help flags` 那句描述也是这个）；
/// - `--local-links`：local backend 自己的同一个开关（`rclone help flags`：
///   `Translate symlinks to/from regular files with a '.rclonelink' extension for the local backend`，
///   短 flag 那一列是空的 → 只有长写法）。第一次只量到"它跟本地目标一样 exit=1、只是试图推"，
///   那个记录**低估了它**：源侧暴露的就是那条拼写（`rclone lsf -R <夹具> --local-links` 比默认多吐
///   一个 `link.rclonelink`，与 `--links` 一字不差），而**目标端没有符号链接可写时它真的落地** ——
///   `rclone copy <夹具> :memory: --local-links` → exit=0、`Transferred: 4 / 4`，多出来那条就是
///   `link.rclonelink`（单独 `--include link.rclonelink` 那一档 Transferred 是 55 B；**那条对象里写了
///   什么没量** —— 闸门要的是"两侧拼写集合一不一致"，不是那条对象里写了什么。换成真实对象存储还有
///   没有别的差别也没量，云端在范围外）；
/// - `--links=false` / `--copy-links=false` / `--local-links=false` / `-L=false` / `-l=false`：
///   实测与默认**一字不差** → 不改不变量 → 见 [`link_flag_explicitly_off`]，这一档放过（**五条**）。
/// 上面这些一改，"少一条 = 删除候选"那个方向就开始出错，而且操作员从面板上看不出来：一条
/// `link/a.webp` 读起来和"上次删掉的那张图"长得一模一样，而逐条确认恰恰是抓不住它的东西。
/// 与其让两侧各按一套前提算，不如让配置过不去。
///
/// `--junction` 在 v1.75.1 **不存在**（实测 `Error: unknown flag: --junction`、exit=2；
/// `--links-follow-template`、`--check-links` 同样 unknown —— 把它们当成"真实存在的 obscure flag"
/// 是错的）。留着是**防御性**的：这种 token 进了 rcd 的 argv 只会让 rcd 启动就失败，在配置层
/// 拒绝它给操作员的是点名 flag 的中文报错，而不是 rcd 的 exit=2；上游哪天真加了这个 flag，这里
/// 也已经拦着了。
///
/// 匹配风格：**长写法**（`--…`）精确等于 或 `--flag=值`；**单横线 token 拆成逐个字母**，含 `L`/`l`
/// 就拒（`-Lv` 实测真的会跟随链接，所以"精确等于 `-L`"只拦得住分开写那一种）。合起来覆盖
/// `-L`、`-l`、`--copy-links`、`--links`、`--local-links`、`--junction` 的裸写、`=` 形式与合并形式
/// （`--local-links` 只有长写法，`=` 形式里 `--local-links=true` 实测同样吐 `link.rclonelink`；
/// `--local-links=false` 实测等于默认 → 也进了 [`link_flag_explicitly_off`] 的豁免表，见那里）。
///
/// **这一族现在被两层拦**：白名单 [`ALLOWED_FLAGS`] 本就不含任何链接 flag（不在名单=拒），这条黑名单
/// 再点名一次——留着是为了（1）白名单那句通用话术之外，操作员能看到**实测过**的具体后果（`link/...` /
/// `.rclonelink` 怎么变成删除候选），（2）不变量的来源写死在配置层、不随白名单将来被放宽而回退。
/// 判据与后果表在 [`link_flag_explicitly_off`] 之上、[`is_banned_link_flag`] 的话术里，逐条实测见
/// `docs/superpowers/evidence/build-notes.md`。
///
/// 黑名单**不拦**、而白名单**拒**的同族写法（两层判据不同，别把这一档看成漏了）：
/// `--skip-links` 实测只关掉那条 NOTICE（落地集合不变）、`--local-zero-size-links` 实测与默认一字不差
/// （`rclone lsf -R <夹具>` 同样三个对象）——它们不改不变量，但也不在白名单上，所以不再"因无害而放行"，
/// 改成"因未点名而拒绝"（过度拦、安全侧）。`--ignore-case` 的方向本来就是安全的（实测让 rclone 比我们的
/// 字节精确匹配**多**挡对象 → `expected` 比桶多几条 → gate 多挡一次检查、不是删除候选，见
/// `rc/listing.rs` 的 `normalize_expected` 注释与 `build-notes.md`）——但它是"改过滤语义"的那一族，
/// 排除只有一个入口（`exclude` 字段），所以同样落在白名单外、被拒。这张表是按 `rclone help flags`
/// 逐条实测生成的，它比手写第二列关得干净：不在名单上的（含 `--sftp-skip-links` 那种没量过
/// 的别的 backend 开关）一律进不去。
const BANNED_LINK_FLAGS: &[&str] =
    &["-L", "-l", "--copy-links", "--links", "--local-links", "--junction"];

/// 会改动链接语义的**单个短字母**：`-L`（`--copy-links`）与 `-l`（`--links`）。
/// 两条都是钉住的 v1.75.1 `rclone help flags` 里的短写法：
/// `-L, --copy-links  Follow symlinks and copy the pointed to item` /
/// `-l, --links  Translate symlinks to/from regular files with a '.rclonelink' extension`。
const BANNED_LINK_SHORTHANDS: &[char] = &['L', 'l'];

/// 显式 `=false` 的布尔链接写法：实测与默认行为一字不差（五条都是 exit=0、落地都只有
/// `rootfile.webp sub/a.webp sub/deep/c.webp`、那条 `NOTICE: … Can't follow symlink` 照样出现），
/// 所以它们不改不变量、在白名单之外被**独立豁免**放行。`--local-links=false` 也在这一列：它同样
/// 实测等于默认（同样的 NOTICE、同样三个对象，见 `build-notes.md` §Task 4 修复轮（第四轮））——
/// 它一度落在拒绝侧，而这张表自己的判据就是"实测等于默认 ⇒ 不改不变量"，那种不一致以实测为准。
/// 只放这**五条精确拼写**：`-Lv=false` 那种"合并 + 关掉"的写法 pflag 到底怎么解析没量过，落在拒绝侧。
/// 豁免是"白名单之外的一次性放行"，不是把 `--links` 一族放进白名单——裸写与 `=true` 仍走链接闸门，
/// 由白名单/链接闸门拒的方向才是不变量的地基（见 [`parse`] 里 flag 循环那四步）。
fn link_flag_explicitly_off(f: &str) -> bool {
    matches!(f,
        "--links=false" | "--copy-links=false" | "--local-links=false" | "-L=false" | "-l=false")
}

/// 单横线 token 里藏着 `L`/`l` 这个字母吗？`-Lv`、`-vL`、`-lq`、`-PvL`、`-L=true` 都算命中。
/// rclone 的 flag 库（pflag）允许把短 flag 合并进一个 token，实测 `-Lv` 与 `-vL` 都 exit=0 并且
/// **真的**多落地了 `link/…`，所以链接类黑名单不能只会"精确等于"。
/// `--` 开头的不归这里（长写法走 [`is_banned`] 那一档）。切字母的方式是**在第一个 `=` 处切开、只取
/// 前半**：所以 `-L=true` 扫到的是 `L`、`=true` 那段不算字母。但要说准——**pflag 的附着值短写法
/// `-f<值>` 没有 `=`**（`-f, --filter stringArray`，见 `rclone help flags`），此时 split 找不到 `=`、
/// 整段后半（那串"值"）也会被当成字母扫一遍。对链接黑名单这只是**过度拦**的方向（`-fnode_modules` 里
/// 的 `l` 会被当成链接字母命中），不会漏拦；漏拦才是问题。这话术来自哪一步别说反：链接闸门是 [`parse`]
/// 四步里的**第 3 步**，白名单是**第 4 步**，所以这一档**先**执行——上面那个 `-fnode_modules` 实际拿到
/// 的是第 3 步那句**链接语义**的点名拒绝，而不是第 4 步那句通用"不在白名单"。`-f` 本来就不在
/// [`ALLOWED_FLAGS`]/[`ALLOWED_SHORT_FLAGS`] 里，即使顺序反过来第 4 步也照样拒它：**判定不变、拒绝不变，
/// 变的只是操作员读到哪句话**（同理，值位上的过滤 flag 也照样走第 4 步，见
/// `refuses_a_filter_flag_even_when_it_sits_in_a_value_position`）。
/// 副作用要说清：单横线 + 名字里含 `L`/`l` 的那种手滑（`-links`、`-local-links`）由拆字母这一档拒掉，
/// 而 `-max-transfer` 这一类不含 `l` 的写法是第 4 步白名单拒的 —— 两句都点名 token，只是话术不同。
/// rclone 自己也不认那种写法（实测 `Error: unknown shorthand flag: 'm' in -max-transfer`、exit=2）
/// —— 所以这是把报错提前、并且点名是哪个 token，不是拦住一个本来能用的写法。
fn short_token_carries_link_letter(f: &str) -> bool {
    let Some(rest) = f.strip_prefix('-') else { return false };
    if rest.starts_with('-') { return false; }
    rest.split('=').next().unwrap_or(rest)
        .chars().any(|c| BANNED_LINK_SHORTHANDS.contains(&c))
}

/// 链接类黑名单的完整判定：显式 no-op 先放过，再看长写法（精确 / `--flag=值`），最后拆短字母。
fn is_banned_link_flag(f: &str) -> bool {
    !link_flag_explicitly_off(f)
        && (is_banned(f, BANNED_LINK_FLAGS) || short_token_carries_link_letter(f))
}

/// `f` 命中黑名单里的某一条吗？**精确等于**，或者 `--flag=value` 那种带值的写法。
/// 两条黑名单共用这一个风格：`--rc-*` 与链接类 flag 都要拦得住 `-x=1` 的形式，
/// 又都不能顺手把"只是前缀相同"的别的 flag 挡掉（`--list-cutoff`、`--checksum` 就是那种）。
/// 这一档看不见合并短 flag，所以链接类还要再过 [`short_token_carries_link_letter`]。
fn is_banned(f: &str, banned: &[&str]) -> bool {
    banned.iter().any(|b| f == *b || f.starts_with(&format!("{b}=")))
}

/// `[defaults] flags` 的**正面清单**（长写法，含 `--`）。不在这一列里的一律拒绝。
///
/// 为什么是白名单而不是黑名单：rclone 有几百个 flag，
/// 其中任何一个都可能改变"什么会被推进桶"，而 `expected`（本地扫描按 rclone **默认策略**算出来的
/// 那批 key）必须等于 rclone **实际会推上去**的那批——这是 `normalize_expected` 的不变量，也是
/// `gate = expected − 实际` / `盘点 = 实际 − expected` 唯一站得住的地基。一个系统性偏差在**人读的清单**
/// 里看不见：一条看着合理的孤儿和"操作员真想让对齐删掉的那条陈旧重复图"长得一模一样。
/// 黑名单只能挡住我们已经想到的那几种写法；实测到"链接类"这一族之外还有
/// `--include`/`--suffix`/`--backup-dir` 都能新增一个本地扫描结构上给不出的 key，所以白名单不是
/// "以后再说"而是正确性要求——白名单是唯一一种"rclone 明天新增 flag、或操作员粘一条论坛
/// 上看到的写法"都不会回退的形状。
///
/// 每一列都对着钉住的 rclone **v1.75.1** `rclone help flags` + 本地到本地 `copy` 实测过（不是照文档
/// 抄的），判据是"**只改怎么跑、不改哪些东西进去**"：
/// - `--fast-list` 换列举路径（内存换事务）；实测同一棵 junction 夹具，带上它落地集合与默认
///   逐字节相同（`rootfile.webp sub/a.webp sub/deep/c.webp`，那条 `NOTICE: link: Can't follow
///   symlink` 照旧）。
/// - `--transfers` / `--checkers` / `--multi-thread-streams` / `--tpslimit` / `--retries` /
///   `--bwlimit` / `--timeout`：并发、重试、限流、超时——改的是快慢与失败次数，不是对象名。
/// - `--progress` / `-P`、`--stats`、`-v`、`-q`：只改 stdout 的进度/啰嗦程度。
///
/// **实测掉出去的候选**（写在这里是为了别被再抄回来）：
/// - `--low-memory`（任务给的候选之一）在 v1.75.1 **不存在**：`rclone help flags | grep -i low-memory`
///   空、`rclone rcd --low-memory …` → `Error: unknown flag: --low-memory`、exit=2。列进白名单就是
///   给一个 rclone 自己会在启动时拒掉的拼写背书，所以不放（这正是
///   `every_allowlisted_flag_parses_and_is_accepted` 那条测试要防的那类错——名单不能藏一个 rclone 会拒的 token）。
/// - `--user-agent` 存在，但"改什么进桶"看不出来且本面板没有需要它的场景（R2 不经 user-agent 鉴权），
///   按需加入原则不放进来，白名单越小越不用重新审。
/// - `RCLONE_COPY_LINKS=1` 这类 `RCLONE_<FLAG>` **环境变量**：实测同样能越过面板的链接闸门让 junction
///   被跟随。配置这一层**只看 argv token、看不到进程环境**，那一半不归这里，由 spawn 侧收口：
///   `rcd::build_command` 把继承来的 `RCLONE_*` 逐条 `env_remove`，只留 `RCLONE_CONFIG` 与
///   `RCLONE_CONFIG_PASS`（判据是 `rcd::rclone_env_is_scrubbed`，判定先大写归一再比前缀）。
///   所以这条白名单仍然只判 argv —— 别把它说成"连环境一起管"，也别说成"环境那条口子还开着"。
const ALLOWED_FLAGS: &[&str] = &[
    "--transfers", "--checkers", "--bwlimit", "--timeout", "--retries",
    "--multi-thread-streams", "--tpslimit", "--stats",
];

/// 白名单里的**无值 bool** 长 flag：它们后面不跟值，紧跟的裸 token 不是它们的参数（见
/// [`flag_takes_separate_value`]）。其余 [`ALLOWED_FLAGS`] 都是取值 flag（int / float / Duration /
/// BwTimetable），下一个裸 token 就是它的值。
/// `--fast-list`（bool）、`--progress`（bool，`-P` 的长写法）实测都不吃下一个 token。
///
/// **`--size-only` 是这道闸门唯一一枚改判定口径的例外**（一次点名批准，不是顺手补上的）。
/// 判据本来是"只改怎么跑、不改哪些东西进去"，比法不在那句话里 —— 所以这一枚不是"归进那一族"，
/// 是**例外**，代价与实测数字写在这里：
/// - 省下来的：操作员那对目录（本地 6,107 个对象 / 391 MiB → `r2:img`）实测默认比法一趟
///   **52~57 秒**，只比大小 **12~15 秒**；差的正是"逐对象向远端要修改时间"那 6,107 次请求
///   （R2 的列举只回名字和大小，修改时间住在对象元数据里）。同口径下钱落在便宜档
///   （Class B $0.36/百万）→ 一趟约 $0.0022，一个月还在免费额度里，**省的是时间与请求数，不是钱**。
/// - 代价：**内容改了但大小没变的那一个文件不会再传一次**。他树里 `ideahub/covers/<sha8>.webp`
///   这类内容寻址的名字不受影响（名字变了才算内容变）；受影响的是原地覆盖写、又恰好同尺寸那一类。
/// - 因此这一枚必须由界面说话（顶栏那一句**只在开着时出现** + 设置那一格点名它），并由每条任务日志里的
///   `compare=` 记下当时用的是哪一种。判定只由 [`compares_by_size`] 算一次。
/// - 实测**否掉**的捷径（别再抄回来）：`--use-server-modtime` 同样省掉逐对象那一问，实测 14 秒，
///   但那一趟里有 **1,091** 个对象要为此重写一次修改时间，而重写本身又把服务器时间推后 → 每趟都
///   收敛不了。它不在名单上，是**故意**不在的。
const ALLOWED_BOOL_FLAGS: &[&str] = &["--fast-list", "--progress", "--size-only"];

/// 白名单里的**单字母短 flag**（都无值）：`-P`=`--progress`、`-v`=`--verbose`、`-q`=`--quiet`。
/// 对照钉住的 v1.75.1 `rclone help flags`：`-P, --progress` / `-v, --verbose count` /
/// `-q, --quiet`。合并写法（`-Pv`、`-qP`）里每个字母都得在这一列，含 `L`/`l` 的会被链接闸门先拒。
const ALLOWED_SHORT_FLAGS: &[char] = &['P', 'v', 'q'];

/// 过滤类 flag（整族拒绝，哪怕 `--exclude` 看着无害）。它们**不在** [`ALLOWED_FLAGS`] 里——
/// 白名单已经把它们挡在外面，这里单列一份是为了给拒绝话术点名"排除只有一个入口"。
/// 实测（v1.75.1）：`--include` 与面板自己的 `--exclude` 并写时 exit=0、
/// 只留被排除那条、rclone 只打一行 "order … indeterminate" ERROR。
const FILTER_FLAGS: &[&str] = &[
    "--exclude", "--exclude-from", "--include", "--include-from",
    "--filter", "--filter-from", "--files-from",
];

/// 这个 `-` 开头的 token 是不是白名单里的 flag？长写法：精确等于或 `--flag=值`；
/// 单横线：`=` 之前的逐个字母都必须在 [`ALLOWED_SHORT_FLAGS`] 里（空串不算，`-` 单独不是 flag）。
/// **精确匹配、不猜前缀**——与 [`is_banned`] 同一风格，`--list-cutoff` 不会因为前缀撞上 `--…` 蒙混。
///
/// **一处已知的不对称，别当 bug 顺手"修"**：[`ALLOWED_BOOL_FLAGS`] 只做**精确等于**、没有 `=` 那一档，
/// 所以 `--fast-list=true` / `--progress=true` 会被拒；而 [`ALLOWED_FLAGS`] 收 `--flag=值`，
/// pflag 自己其实也接受 bool flag 的 `--flag=true` 写法。这是**过度拦**、有意为之（安全侧：
/// 白名单宁可少放一种写法），行为不要改——要放的话得同时想清楚 `--fast-list=false` 这种"关上的
/// bool"算不算值归属（它不吃下一个 token，`--flag=值` 也不），一行 `||` 不值得为它引入一个新的形状。
fn flag_is_allowlisted(f: &str) -> bool {
    if f.starts_with("--") {
        ALLOWED_FLAGS.iter().any(|a| f == *a || f.starts_with(&format!("{a}=")))
            || ALLOWED_BOOL_FLAGS.iter().any(|a| f == *a)
    } else if let Some(rest) = f.strip_prefix('-') {
        let letters = rest.split('=').next().unwrap_or(rest);
        !letters.is_empty() && letters.chars().all(|c| ALLOWED_SHORT_FLAGS.contains(&c))
    } else {
        false
    }
}

/// 这个 `-` 开头的白名单 flag **会不会吃下一个裸 token 作为它的值**？取值 flag 会；
/// bool 长写法（[`ALLOWED_BOOL_FLAGS`]）不会；`--flag=值` 那种自带值的写法后面也不需要裸 token；
/// 单字母都是无值 bool。用来判断"下一个裸 token 是值（放行）还是孤立位置参数（拒绝）"。
fn flag_takes_separate_value(f: &str) -> bool {
    if f.starts_with("--") {
        // 带 `=` 的长写法已经把值写在串里了，不消费下一个 token。
        !f.contains('=') && ALLOWED_FLAGS.contains(&f)
    } else {
        false // 短写法（P/v/q）全是 bool
    }
}

/// 这一份 flags 是不是把比法换成了"只比大小"？**判定只在这一处算**：顶栏那句、每条任务日志里
/// 那个 `compare=`、以及以后可能有的确认层提示，都读这一个函数。各自再判一次，就是
/// "屏上写着只比大小、实际按时间比"这一族谎的来路（本仓红过好几次的同一件事）。
///
/// 只认**精确等于**那一个 token：`--size-only=false` 那种写法根本过不了 [`flag_is_allowlisted`]
/// （bool 那一族没有 `=` 那一档，见 [`ALLOWED_BOOL_FLAGS`] 上那条有意为之的不对称），
/// 所以这里不必替它编一种语义 —— 两处口径一致，钉在 `compares_by_size_reads_the_exact_token_only`。
pub fn compares_by_size(flags: &[String]) -> bool {
    flags.iter().any(|f| f == "--size-only")
}

impl JobsFile {
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let raw: RawFile = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;

        let defaults = Defaults {
            rclone_bin: raw.defaults.rclone_bin,
            flags: raw.defaults.flags,
            exclude: raw.defaults.exclude,
            log_dir: raw.defaults.log_dir,
            check_by_hash: raw.defaults.check_by_hash,
        };
        if !PathBuf::from(&defaults.rclone_bin).is_absolute() {
            return Err(ConfigError::Invalid(format!(
                "rclone_bin 必须是绝对路径，当前是 {:?}", defaults.rclone_bin)));
        }
        // `[defaults] flags` 的校验按"四步"走，顺序就是话术的顺序，别调换：
        //   1) rc 地址/鉴权黑名单——单独点名，保证这条保证在白名单之外仍一眼可辨；
        //   2) 实测与默认一字不差的链接 `=false` 写法——白名单之外的一次性豁免（no-op，不改不变量）；
        //   3) 链接类黑名单——即便白名单本就不含它们，也继续点名，为了留下实测过的后果话术（防御纵深）；
        //   4) 白名单——不在名单上的 flag 一律拒；裸 token 只有作为上一条"取值 flag"的值才放行。
        let mut prev_takes_value = false;
        for f in &defaults.flags {
            // 过滤器与 fs.Config flag 只能走 rcd 启动参数（spec §3.1.1），
            // 但绝不能把 rc 自身的地址/鉴权参数混进来。
            if is_banned(f, BANNED_FLAGS) {
                return Err(ConfigError::Invalid(format!(
                    "defaults.flags 里不允许出现 {f}：rc 的监听地址与鉴权由面板控制")));
            }
            // 第 2 步：白名单之外的独立豁免（五条实测 no-op）。放在链接黑名单判定之前，
            // 所以 `--links=false` 拿到的是"放行"，而 `--links` / `--links=true` 拿到第 3 步的点名拒绝。
            if link_flag_explicitly_off(f) {
                prev_takes_value = false;
                continue;
            }
            // 第 3 步：链接类 flag 改掉的是"哪些对象会被推进桶"这一半事实，
            // 而期望集合是按 rclone 的默认策略（不跟随链接）算出来的：两侧一不一致，
            // 对齐任务就会拿着一条真实存在的对象上删除清单。见 BANNED_LINK_FLAGS 的注释。
            // 判定含长写法与**合并短 flag**（`-Lv` 实测真的生效）。白名单本就不含它们，这里仍单独点名。
            if is_banned_link_flag(f) {
                return Err(ConfigError::Invalid(format!(
                    "defaults.flags 里不允许出现 {f}：它会改变哪些对象会被传到云端\
                     （链接会被跟随，或者链接本身被当成一个对象推上去，实测的拼写是\
                     `<链接名>.rclonelink`），而 gate/盘点的期望集合按 rclone \
                     的默认策略算——不跟随链接。两边不一致时，云端多出来的那条拼写就成了对齐任务的\
                     删除候选，删掉的是真实存在的文件，而人在清单里抓不住它——那条拼写读起来和\
                     一张真该删的孤儿一模一样。合并写法的一个 `-` 后面只要含 `L` 或 `l` 也算，\
                     rclone 认那种写法。要让某个目录不传上去，请用 exclude 的\
                     `目录名/` 写法（实测与 rclone 剪整棵子树的行为一致），不要加链接类 flag。")));
            }
            // 第 4 步：白名单。
            if f.starts_with('-') {
                // `-` 开头的是 flag token：必须在白名单上（精确或 `--flag=值`；短写法逐字母）。
                if !flag_is_allowlisted(f) {
                    // 过滤类整族给专门的话术：排除只有一个入口，就是 `exclude` 配置字段。
                    if is_banned(f, FILTER_FLAGS) {
                        return Err(ConfigError::Invalid(format!(
                            "defaults.flags 里的 {f}：不在面板允许的 flag 白名单里，而且它是过滤类。\
                             排除/包含只有**一个入口**——`[defaults] exclude` 配置字段，它会同时喂给\
                             rclone 的 `--exclude` 启动参数和面板自己的匹配器（`normalize_expected`）。\
                             从 flags 再开第二条过滤路径，两侧就会悄悄不再同一次匹配：`--include` 实测\
                             与 `--exclude` 并写时 exit=0、只留一边、rclone 只打一行 order indeterminate\
                             ERROR。要让某个目录传/不传上去，请用 exclude（剪整棵子树写 `目录名/`）。")));
                    }
                    return Err(ConfigError::Invalid(format!(
                        "defaults.flags 里的 {f}：不在面板允许的 flag 白名单里。\
                         白名单只收**只改怎么跑、不改哪些东西进去**的吞吐/并发/重试/限流/超时\
                         （--transfers、--checkers、--fast-list、--bwlimit、--timeout、--retries、\
                         --multi-thread-streams、--tpslimit）与进度/啰嗦（-P、--progress、--stats、\
                         -v、-q）参数；比法那一族只**点名**收了一枚 --size-only（改成只比大小，\
                         代价是内容变了而大小没变的那些不会重传，所以开着它时界面上要一直写着这一句），\
                         其余任何会改变\
                         哪些对象会被传到云端、或什么算「变了」的写法都不在其中。\
                         这不是前缀/子串匹配（`--list-cutoff`、`--links-follow-template` 那种只是\
                         长得像的一律拒），是正面点名放行——rclone 有几百个 flag，逐个证明它们不改\
                         落地的东西做不到，所以未点名的默认拒绝（安全侧）。要排除目录请用 exclude 字段。")));
                }
                prev_takes_value = flag_takes_separate_value(f);
                continue;
            }
            // 非 `-` 开头的是裸 token：只有紧跟在"取值 flag"后面（作为它的值）才放行，否则是 rcd 的
            // 位置参数。面板要的是点名 token 的配置层报错，而不是让 rclone 替我们说话——它不会：
            // 实测（v1.75.1，本地到本地，见 build-notes）`rclone rcd <目录>` **照常启动**并打印
            // `NOTICE: Serving files from "<目录>"`，把那个目录当成静态站点开在同一个 rc 端口上。
            if !prev_takes_value {
                return Err(ConfigError::Invalid(format!(
                    "defaults.flags 里的 {f}：既不是 `-` 开头的 flag，也不是上一条取值 flag 的值。\
                     它会被拼进 rcd 的启动参数当位置参数。实测（v1.75.1，本地到本地）\
                     `rclone rcd <目录>` **照常启动**，打印 `NOTICE: Serving files from \"<目录>\"`，\
                     并把那个目录当成静态站点开在同一个 rc 端口上：带凭据 `GET /` 返回目录清单、\
                     `GET /文件` 返回文件内容；不带凭据仍是 401（鉴权覆盖这条路径，不是开了个后门，\
                     但它已经悄悄多了一个把本地目录端出去的面）。它不会 exit=2 替你说话，所以这里必须自己拦。\
                     值请紧跟在它所属的白名单 flag 后面（如 `--transfers`, `32`），要么用 `--flag=值` 一体写法。")));
            }
            prev_takes_value = false;
        }

        let mut seen = BTreeSet::new();
        let mut jobs = Vec::with_capacity(raw.job.len());
        for rj in raw.job {
            if !seen.insert(rj.id.clone()) {
                return Err(ConfigError::Invalid(format!("job id 重复：{}", rj.id)));
            }
            let kind = match rj.kind.as_str() {
                "copy" => JobKind::Copy,
                "sync" => JobKind::Sync,
                "gate" => JobKind::Gate,
                // 裁定 R58：这个 kind 连同它的删除路径一起没了。单独给一档话术，
                // 让还留着旧 jobs.toml 的人读到"它已经不存在、任务这一侧的删除只走对齐"，
                // 而不是掉进下面那句"非法 kind"里猜自己拼错了哪个字母。
                "prune" => return Err(ConfigError::Invalid(format!(
                    "job {} 的 kind = \"prune\" 已经不存在了：清理功能已降级为只读的盘点视图\
                     （「本地没有的云端对象」），删除只能走 kind = \"sync\" 的对齐任务", rj.id))),
                other => return Err(ConfigError::Invalid(format!(
                    "job {} 的 kind 非法：{other}（只接受 copy/sync/gate）", rj.id))),
            };
            let manifest = match rj.manifest {
                None if kind == JobKind::Gate => {
                    return Err(ConfigError::Invalid(format!("gate 任务 {} 缺少 manifest", rj.id)))
                }
                None => None,
                Some(m) => Some(match m.kind.as_str() {
                    "localdir" => Manifest::LocalDir,
                    "file" => Manifest::File(PathBuf::from(m.path.ok_or_else(|| {
                        ConfigError::Invalid(format!("job {} 的 manifest.kind=file 需要 path", rj.id))
                    })?)),
                    // d1 属子项目 2。schema 认识它，但子项目 1 必须拒绝：
                    // 否则会在没有凭据的情况下静默跑成"全量缺失"。
                    "d1" => return Err(ConfigError::Invalid(format!(
                        "job {} 的 manifest.kind = \"d1\" 属子项目 2，子项目 1 不支持", rj.id))),
                    other => return Err(ConfigError::Invalid(format!(
                        "job {} 的 manifest.kind 非法：{other}", rj.id))),
                }),
            };
            let job = Job {
                id: rj.id, name: rj.name, kind,
                src: rj.src, dst: rj.dst, open_dir: rj.open_dir, manifest,
            };
            // 位置本身的判据（非空、对齐那一档要恰好一侧是云端、云端那一侧不许有反斜杠与点段）
            // 与临时配对卡走的是**同一段代码**：`Job::check_positions()`。两处各写一遍的话，
            // 少写的那一处就是"面板跑得成一条配置侧会拒绝的任务"，而那一族任务恰好是
            // 删除范围算错的那一族。判据的理由写在那一句的文档上，这里只留调用点。
            job.check_positions()?;
            jobs.push(job);
        }
        Ok(Self { defaults, jobs })
    }
}

impl Job {
    /// 这一条的**两个位置本身**合不合面板的规矩：两侧非空、对齐那一档要恰好一侧是云端、
    /// 云端那一侧不许混进反斜杠与点段。
    ///
    /// 为什么在这里而不是只长在解析器里：配置文件与临时配对卡（`runner::drafts::pair_draft`）
    /// 交的是同一种东西——一对要写的位置，而临时卡**不经过** `JobsFile::parse`。
    /// 两处各写一遍，少写的那一处就是"面板跑得成一条配置侧会拒绝的任务"，
    /// 而那一族任务恰好是删除范围算错的那一族（反斜杠那一条会把删除范围静默放宽到整个桶）。
    ///
    /// 不含"文件里那段文本对不对"那一类判据（kind 的拼写、manifest、id 重名）：
    /// 那些只有读文件的解析器需要，一条内存里的临时任务本来就由代码给出。
    pub fn check_positions(&self) -> Result<(), ConfigError> {
        if self.src.trim().is_empty() || self.dst.trim().is_empty() {
            return Err(ConfigError::Invalid(format!("job {} 的 src/dst 不能为空", self.id)));
        }
        if self.kind == JobKind::Sync {
            // 对齐要圈定「哪些会被删掉」，没有权威的一端就没有删除范围。两侧都是本地或两侧
            // 都是 remote 都对 rclone 合法，但面板交不出可核对的清单，所以都拒、不猜。
            // 判据本体在 `pair::mirror_sides_ok`（临时卡决定"这张卡建不建这一档"读的是同一处）。
            if !crate::pair::mirror_sides_ok(&self.src, &self.dst) {
                return Err(ConfigError::Invalid(format!(
                    "job {} 是对齐任务（kind = \"sync\"）：两侧必须**恰好一侧**是 remote（形如 `名称:路径`）。\
                     现在 src = {:?}、dst = {:?}——对齐要说得出「哪些会被删掉」，\
                     没有权威的一端就没有删除范围，所以这里不猜。",
                    self.id, self.src, self.dst)));
            }
            // 引用的是**操作员写进来的那一整侧**，不是 `stray_backslash()` 交出的那段片段：
            // 那个探测器只回"第一个反斜杠之后"的尾巴（`r2:img/ideahub\covers` → `covers`），
            // 把尾巴印进一句关于删除范围的话里，操作员读到的是一个他文件里根本不存在的东西，
            // 只能更糊涂。探测器继续用 `stray_backslash()`，话术用 `self.src` / `self.dst`——
            // 与下面那条兄弟检查（`rejects_backslashes_after_the_remote_name` 钉着它）同一口径。
            for (which, written, ep) in [
                ("src", &self.src, self.endpoint_src()),
                ("dst", &self.dst, self.endpoint_dst()),
            ] {
                if ep.stray_backslash().is_some() {
                    return Err(ConfigError::Invalid(format!(
                        "job {} 的 {which} 在 remote: 之后不能出现反斜杠 \\（rclone 的 remote 路径只用 /），当前是 {written:?}",
                        self.id)));
                }
            }
        }
        // remote 的 dst 里反斜杠不是分隔符，而是桶名/对象名的字面字符，可桶与路径的切分
        // 只认 `/`：`r2:img\ideahub\covers` 会让 fs_for_list 收整串、remote_part/prefix
        // 收 ""，gate 比对错范围，对齐的删除范围被静默放宽到整个桶。
        // 不在这里改写分隔符 —— 那等于替一份定义删除范围的位置猜意图。只拒绝。
        // 本地 dst（D:\vault\...）走不到这个分支：remote_split 的盘符规则让它返回 None。
        if let Some((_, after_remote)) = self.remote_split() {
            if after_remote.contains('\\') {
                return Err(ConfigError::Invalid(format!(
                    "job {} 的 dst 在 remote: 之后不能出现反斜杠 \\（rclone 的 remote 路径只用 /），当前是 {:?}",
                    self.id, self.dst)));
            }
        }
        // 名字里含点段（整段是 `.` 或 `..`）的两侧一律拒（裁定 R98）。
        // 理由不是"rclone 会怎么解析"（那一格没有实测过，裁定 R95/R89 所以不说），
        // 而是**这条键算不出同一棵树**：`Scope::norm` 只压空段、不折叠点段
        // （`scope.rs` 里 `norm` 头上那段是为什么），于是 `img/../other` 与 `other` 是两个键，
        // 挂在同一棵树上的两条写者互相看不见 —— 正是裁定 R90 要关的那一格，
        // 也正是本项目起点事故的形状换了个写法。
        // 不在这里化简：化简要猜"哪一段是路径、哪一段是对象名"，对象键里 `..` 是合法字符，
        // 猜错等于替一份定义删除范围的位置改名字。只拒绝，与反斜杠那两条同一个口径。
        for (which, written) in [("src", &self.src), ("dst", &self.dst)] {
            if has_dot_segment(written) {
                return Err(ConfigError::Invalid(format!(
                    "job {} 的 {which} 里有一段就是一个点（.）或两个点（..）：{written:?}。\
                     面板按目录树判断两次写会不会互相踩，那种写法算不出是哪一棵树，\
                     所以这里不猜——请把它写成一个具体的目录名。", self.id)));
            }
        }
        Ok(())
    }

    /// `dst` 里的 remote 部分。Windows 盘符也长成 `X:`，靠"冒号前不止一个字符"区分：
    /// `r2:img/x` 是 remote，`D:\vault\x` 是本地路径。判据本体在 `endpoint::remote_split`，
    /// 这里只是"旧的 dst 口径"那一层——**新代码别再往上加消费者**。
    fn remote_split(&self) -> Option<(&str, &str)> { parse_remote(&self.dst) }

    /// 两侧的端点。对齐按方向取边（下载方向的 remote 在 `src`），gate/盘点仍用上面那组
    /// 只认 `dst` 的旧方法——那两类的 dst 恒为远端，前提没反。
    pub fn endpoint_src(&self) -> Endpoint { endpoint_of(&self.src) }
    pub fn endpoint_dst(&self) -> Endpoint { endpoint_of(&self.dst) }

    /// 列举用的 fs。`r2:img/ideahub/covers` -> `r2:img`；本地路径原样返回（换成正斜杠）。
    pub fn fs_for_list(&self) -> String {
        match self.endpoint_dst() {
            Endpoint::Remote { fs, .. } => fs,
            // 这一档留着只为兼容旧调用方。新代码别用它判"是不是 remote"——
            // `endpoint_dst().remote_target()` 才是会说话的那一个。
            Endpoint::Local(p) => p.to_string_lossy().replace('\\', "/"),
        }
    }

    /// 列举用的 remote 部分，不会失败。`r2:img/ideahub/covers/` -> `"ideahub/covers"`；
    /// `r2:img` 和本地路径都 -> `""`。
    pub fn remote_part(&self) -> String {
        match self.endpoint_dst() {
            Endpoint::Remote { remote, .. } => remote.replace('\\', "/"),
            Endpoint::Local(_) => String::new(),
        }
    }

    /// 与 `remote_part` 同值，但要求 `dst` 确实是个 remote——
    /// 生产上的 gate/盘点任务必须指向 remote，写成本地路径就是配置写错了。
    pub fn remote_for_list(&self) -> Result<String, ConfigError> {
        self.endpoint_dst().remote_target()
            .map(|(_, remote)| remote.replace('\\', "/"))
            .ok_or_else(|| ConfigError::Invalid(format!(
                "job {} 的 dst 必须形如 remote:path，当前是 {:?}", self.id, self.dst)))
    }

    /// gate/盘点归一化用的前缀，带结尾斜杠；桶根为 `""`。
    pub fn prefix(&self) -> String {
        let remote = self.remote_part();
        if remote.is_empty() { String::new() } else { format!("{remote}/") }
    }

    /// `src=>dst` 那根方向串。它**既不是锁的键**（锁的键见 `write_scopes`）**也不是配对的键**
    /// （卡上那一对读 `pair_key` / `pair_slot`），界面上没有任何一处用它。
    /// 现在读它的只有它自己那两条证人测试；要给生产路径接上它，先看 `scope.rs` 头上那段——
    /// 那把方向串挡不住父子两棵树。
    pub fn mutex_key(&self) -> String {
        let n = |s: &str| s.replace('\\', "/");
        format!("{}=>{}", n(&self.src), n(&self.dst))
    }

    /// 这一次写动作真正碰到的**两棵树**（裁定 R71 / spec I3）：`[源端那棵, 目标端那棵]`。
    /// 方向不是锁的一种状态——`up`（`A→B`）与 `down`（`B→A`）交出的**是同一组**，只是顺序互换。
    /// 之所以两侧都进：一次写动作动的不是"一对目录的关系"而是**两棵树**，
    /// 只登记被写的那一侧时，反向任务与浏览器都能从源端那棵树里走过去。
    ///
    /// 交给 `JobMutex::acquire_scopes`，别交给 `acquire_all`（裁定 R90）：后者收的是键串，
    /// 只能比等号，于是"占着 `img` 的对齐"与"要写 `img/2026` 的浏览器"互不相干——
    /// 而同一棵树的上下两层恰恰是这两个入口唯一的关系。树在这里就带着树的形状，
    /// 别在递给别人之前先把它压成字符串。
    ///
    /// 两侧**互为父子**也照样交这一组：会写的任务里只有 `kind = copy` 交得出这种形状（`sync` 被
    /// "恰好一侧是 remote"那条校验挡住，两侧根必属不同类）。父子同批由锁层折到祖先那一条登记
    /// （裁定 R91），配置层**不**加拒绝——这里多加一条"不许"的代价是让操作员读一句他验证不了的
    /// 拒绝，而这个形状最坏只是往自己的子目录里多写一份文件（`ALLOWED_FLAGS` 里没有任何删除类
    /// flag，删除只可能来自到不了这个形状的 `sync`）。`gate` 与盘点也交这一组、也走同一把锁，
    /// 但它们只列不写，折不折都改不出后果。
    pub fn write_scopes(&self) -> Vec<Scope> {
        [self.endpoint_src(), self.endpoint_dst()]
            .iter()
            .map(|ep| Scope::of_endpoint(ep))
            .collect()
    }

    /// 那两棵树的**键**（`write_scopes()` 的一行派生，`Scope::key()` 逐条）。
    /// 要字符串的读者用它：登记表的读数（`MultiGuard::keys()`、`Runner::locks()` 那张键 → 持有者的表），
    /// 以及测试里"占住这一把"的实参与配对判据。锁本身不用它——锁走 `acquire_scopes`（见上面那段）；
    /// 界面那句「正在跑：谁」也不用它——那一格问的是 `Runner::blocking_owner`，交的是
    /// `write_scopes()` 本身（裁定 R92：读侧与拒绝侧比的是同一件事，不能一个拿键一个拿树）。
    pub fn scope_keys(&self) -> Vec<String> {
        self.write_scopes().iter().map(|s| s.key()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_SAMPLE: &str = r#"
[defaults]
rclone_bin = "C:\\rclone\\rclone.exe"
flags = ["--fast-list", "--transfers", "32", "--checkers", "64"]
exclude = ["Thumbs.db", "*.DS_Store"]
log_dir = "D:\\Sync\\logs\\rclone-deck"

[[job]]
id = "r2-up"
name = "R2 图床上传"
kind = "copy"
src = "D:\\vault\\img-r2"
dst = "r2:img"
open_dir = "D:\\vault\\img-r2\\imgprompt"

[[job]]
id = "ideahub-covers"
name = "ideahub 封面对账"
kind = "gate"
src = "D:\\vault\\img-r2\\ideahub\\covers"
dst = "r2:img/ideahub/covers"
manifest = { kind = "localdir" }

[[job]]
id = "r2-mirror"
name = "R2 对齐（会删除，手工加的才该有）"
kind = "sync"
src = "D:\\vault\\img-r2"
dst = "r2:img"
"#;

    #[test]
    fn parses_the_spec_sample() {
        let f = JobsFile::parse(SPEC_SAMPLE).expect("spec sample must parse");
        assert_eq!(f.jobs.len(), 3);
        assert_eq!(f.defaults.rclone_bin, "C:\\rclone\\rclone.exe");
        assert!(matches!(f.jobs[0].kind, JobKind::Copy));
        assert!(matches!(f.jobs[1].kind, JobKind::Gate));
        assert!(matches!(f.jobs[2].kind, JobKind::Sync));
        assert!(matches!(f.jobs[1].manifest, Some(Manifest::LocalDir)));
    }

    #[test]
    fn gate_dst_yields_prefix_and_remote() {
        let f = JobsFile::parse(SPEC_SAMPLE).unwrap();
        let gate = &f.jobs[1];
        assert_eq!(gate.prefix(), "ideahub/covers/");
        assert_eq!(gate.remote_for_list().unwrap(), "ideahub/covers");
    }

    #[test]
    fn trailing_slash_in_dst_is_stripped_not_passed_through() {
        // remote 带结尾斜杠会让 operations/list 返回空列表（spec §3.2），
        // 于是 gate 认为"桶里什么都没有"、全部报缺失。必须在配置层就掐掉。
        let text = SPEC_SAMPLE.replace("r2:img/ideahub/covers", "r2:img/ideahub/covers/");
        let f = JobsFile::parse(&text).unwrap();
        assert_eq!(f.jobs[1].remote_for_list().unwrap(), "ideahub/covers");
        assert_eq!(f.jobs[1].prefix(), "ideahub/covers/");
    }

    #[test]
    fn bucket_root_dst_yields_empty_remote() {
        let f = JobsFile::parse(SPEC_SAMPLE).unwrap();
        assert_eq!(f.jobs[2].remote_for_list().unwrap(), "");
        assert_eq!(f.jobs[2].prefix(), "");
    }

    #[test]
    fn fs_and_remote_are_split_at_the_first_slash_after_the_bucket() {
        let f = JobsFile::parse(SPEC_SAMPLE).unwrap();
        let gate = &f.jobs[1];
        assert_eq!(gate.fs_for_list(), "r2:img");
        assert_eq!(gate.remote_part(), "ideahub/covers");
        assert_eq!(f.jobs[2].fs_for_list(), "r2:img");
        assert_eq!(f.jobs[2].remote_part(), "");
    }

    #[test]
    fn a_windows_drive_letter_is_not_mistaken_for_a_remote_name() {
        // 本机 dst 是 D:\... 这种形式；把 "D" 当成 remote 名会让 fs 变成 "D:\vault"。
        let local = Job {
            id: "l".into(), name: "l".into(), kind: JobKind::Gate,
            src: "C:\\tmp\\expected".into(), dst: "D:\\vault\\img-r2\\ideahub\\covers".into(),
            open_dir: None, manifest: Some(Manifest::LocalDir),
        };
        assert_eq!(local.fs_for_list(), "D:/vault/img-r2/ideahub/covers");
        assert_eq!(local.remote_part(), "");
        assert_eq!(local.prefix(), "");
        assert!(local.remote_for_list().is_err(), "本地 dst 不是 remote，该报错的地方仍要报错");
    }

    #[test]
    fn mutex_key_is_direction_sensitive_and_separator_normalized() {
        let f = JobsFile::parse(SPEC_SAMPLE).unwrap();
        let up = f.jobs[0].mutex_key();
        assert!(up.contains("D:/vault/img-r2"), "key was {up}");
        assert!(up.contains("r2:img"));
        // 反方向必须是不同的 key，否则下载会锁住上传
        let down = Job {
            id: "d".into(), name: "d".into(), kind: JobKind::Copy,
            src: "r2:img".into(), dst: "D:\\vault\\img-r2".into(),
            open_dir: None, manifest: None,
        }.mutex_key();
        assert_ne!(up, down);
    }

    #[test]
    fn rejects_dot_segments_in_src_or_dst_because_the_key_cannot_name_one_tree() {
        // 裁定 R98：`..` / `.` 作为一整段出现在两侧任一处就拒。判据与浏览器运行时那道是同一个函数
        // （`scope::has_dot_segment`），所以"这个名字能不能进键"全仓只有一个答案。
        // 理由写在 parse 那里：`Scope::norm` 不折叠点段 ⇒ `img/../other` 与 `other` 是两个键 ⇒
        // 同一棵树上的两条写者互相看不见（裁定 R90 那一格）。这里**不**说"rclone 会怎么解析"——
        // 那一格没实测过（R95/R89），而拒它用不着那个说法。
        // 反面半边是 `SPEC_SAMPLE` 本身（每一条 parse 证人都在跑它）：正常名字不受影响。
        for (which, needle) in [("dst", "r2:img/../secrets"), ("src", r"D:\\vault\\..\\temp")] {
            let text = match which {
                "dst" => SPEC_SAMPLE.replace("r2:img/ideahub/covers", needle),
                _ => SPEC_SAMPLE.replace(r"D:\\vault\\img-r2", needle),
            };
            let err = JobsFile::parse(&text).unwrap_err().to_string();
            assert!(err.contains("一个点") || err.contains("两个点"), "{which} 那一侧要拒：{err}");
            assert!(err.contains(which), "话术要点名是哪一侧：{err}");
        }
        // 含点但**不是整段**的名字照旧收：`note.log` 这种写法不能被误挡。
        let ok = SPEC_SAMPLE.replace("r2:img/ideahub/covers", "r2:img/ideahub/covers.d");
        assert!(JobsFile::parse(&ok).is_ok(), "整段相等才是点段，含点不是");
    }

    #[test]
    fn rejects_backslashes_after_the_remote_name() {
        // 最可能的 Windows 手滑：把本地路径粘到 remote 后面 —— dst = "r2:img\ideahub\covers"。
        // rclone 里 \ 不是 remote 路径的分隔符（它是桶名/对象名的字面字符），而桶与路径的
        // 切分只认 /：于是 fs 收整串 "r2:img\ideahub\covers"、remote 收 ""、prefix() 也是 ""，
        // gate 比对错范围，盘点的清单与对齐的删除范围被静默放宽到整个桶。删除范围有歧义就不许放行。
        // TOML 基本字符串里反斜杠要写成 \\ ，解码后才还原成单个 \。
        let text = SPEC_SAMPLE.replace("r2:img/ideahub/covers", r"r2:img\\ideahub\\covers");
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.contains("ideahub-covers"), "error must name the job id, was {err}");
        // 报错用 {:?} 引用 dst，每个 \ 被转义成 \\，所以这里比对的是转义后的写法。
        assert!(err.contains(r"r2:img\\ideahub\\covers"), "error must quote the dst, was {err}");
    }

    #[test]
    fn rejects_unknown_kind() {
        let text = SPEC_SAMPLE.replace("kind = \"copy\"", "kind = \"mirror\"");
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.contains("mirror"), "error was {err}");
    }

    /// 裁定 R58：`prune` 这个 kind 连同它的删除路径一起去掉了。还留着旧 jobs.toml 的人
    /// 必须读到"这个 kind 已经不存在了"与删除入口现在在哪（对齐），
    /// 而不是掉进"非法 kind"那句泛泛的话术里以为自己拼错了字母。
    #[test]
    fn rejects_the_removed_prune_kind_saying_it_no_longer_exists() {
        let text = SPEC_SAMPLE.replace("kind = \"copy\"", "kind = \"prune\"");
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.contains("已经不存在"), "报错要说清这个 kind 没了，实际是：{err}");
        assert!(err.contains("sync"), "报错要指回唯一的删除入口（对齐任务），实际是：{err}");
    }

    #[test]
    fn rejects_duplicate_job_ids() {
        let text = SPEC_SAMPLE.replace("id = \"r2-mirror\"", "id = \"r2-up\"");
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.contains("r2-up"), "error was {err}");
    }

    #[test]
    fn rejects_gate_without_manifest() {
        let text = SPEC_SAMPLE.replace("manifest = { kind = \"localdir\" }", "");
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.to_lowercase().contains("manifest"), "error was {err}");
    }

    #[test]
    fn rejects_d1_manifest_in_subproject_1() {
        let text = SPEC_SAMPLE.replace("kind = \"localdir\"", "kind = \"d1\"");
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.contains("d1"), "error was {err}");
    }

    #[test]
    fn rejects_relative_rclone_bin() {
        let text = SPEC_SAMPLE.replace("C:\\\\rclone\\\\rclone.exe", "rclone");
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.to_lowercase().contains("rclone_bin"), "error was {err}");
    }

    #[test]
    fn rejects_rc_flags_sneaking_into_defaults() {
        // rc 的监听地址与鉴权由面板控制，配置文件里不许覆盖
        let text = SPEC_SAMPLE.replace("\"--fast-list\"", "\"--rc-no-auth\"");
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.contains("--rc-no-auth"), "error was {err}");
    }

    // ---- 链接类 flag：会改变"什么会被推进桶"，而期望侧按 rclone 的默认策略不跟随链接 ----
    // 每条都是同一个形状：换上这个 flag，报错必须点名它。
    //
    // 断言的形式是 `{flag}：`（token 后面紧跟话术里那个全角冒号），不是 `err.contains(flag)`：
    // 后者会被 `--copy-links` 里那个 `-li` 白送一次绿 —— 断言的强度取决于
    // 话术里写不写别的 flag"，这里直接把形式定死，helper 只有一条路径能过。
    fn flag_error(flag: &str) -> String {
        let text = SPEC_SAMPLE.replace("\"--fast-list\"", &format!("\"{flag}\""));
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.contains(&format!("{flag}：")), "报错必须点名 {flag}，实际是：{err}");
        err
    }

    fn flag_is_accepted(flag: &str) {
        let text = SPEC_SAMPLE.replace("\"--fast-list\"", &format!("\"{flag}\""));
        JobsFile::parse(&text).unwrap_or_else(|e| panic!("{flag} 不该被拒：{e}"));
    }

    /// 把 SPEC_SAMPLE 的整个 `flags` 数组换成给定的 token 序列。`flag_error`/`flag_is_accepted` 只换
    /// 一个 token，钉不住"某个 token 处在**值位**上"这种形状——值位的测试必须自己控制数组。
    /// 替换没命中就立刻炸，免得 SPEC_SAMPLE 改行后这两条测试悄悄测的还是原配置。
    fn flags_replaced(flags: &[&str]) -> String {
        const ORIG: &str = "flags = [\"--fast-list\", \"--transfers\", \"32\", \"--checkers\", \"64\"]";
        let joined = flags.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", ");
        let want = format!("flags = [{joined}]");
        let text = SPEC_SAMPLE.replace(ORIG, &want);
        assert!(text.contains(&want), "SPEC_SAMPLE 的 flags 行改了，这个 helper 的替换目标也要跟着改");
        text
    }

    /// 解析这串 flags，返回报错文本（期望被拒时用）。
    fn flags_error(flags: &[&str]) -> String {
        JobsFile::parse(&flags_replaced(flags))
            .unwrap_err()
            .to_string()
    }

    /// 这串 flags 该整体过掉（期望放行时用）。
    fn flags_are_accepted(flags: &[&str]) {
        JobsFile::parse(&flags_replaced(flags))
            .unwrap_or_else(|e| panic!("{flags:?} 不该被拒：{e}"));
    }

    #[test]
    fn rejects_short_copy_links_flag() {
        let err = flag_error("-L");
        assert!(err.contains("云端"), "报错要说清它改了哪一侧的东西，实际是：{err}");
    }

    /// `-l` 就是 `--links` 的短写法，钉住的 v1.75.1 原样（`rclone help flags`）：
    /// `-l, --links  Translate symlinks to/from regular files with a '.rclonelink' extension`。
    /// 长写法在黑名单里而短写法不在 = 闸门只关了一半，而操作员手打的恰恰是短的那一个。
    #[test]
    fn rejects_short_links_flag_lowercase_l() {
        let err = flag_error("-l");
        assert!(err.contains("云端"), "报错要说清它改了哪一侧的东西，实际是：{err}");
    }

    /// 合并短 flag 是 rclone **认**的写法，实测（同一棵 junction 夹具，
    /// `rclone copy <root> <临时目录>`）：`-Lv` 与 `-vL` 都 exit=0，并且**真的**多落地了
    /// `link/a.webp`、`link/deep/c.webp`。所以单横线 token 必须拆成逐个字母来看，
    /// 不然把 `-L` 写进黑名单只是把"操作员分开写"的那种情况关掉了。
    #[test]
    fn rejects_combined_short_flag_tokens_because_rclone_honours_them() {
        for flag in ["-Lv", "-vL", "-lq", "-ql", "-PvL", "-L=true", "-l=true"] {
            let err = flag_error(flag);
            assert!(err.contains("链接"), "{flag} 的报错要说清是链接语义，实际是：{err}");
        }
    }

    #[test]
    fn rejects_long_copy_links_flag() { flag_error("--copy-links"); }

    /// `--links` 不跟随链接，但它把链接**存成一个对象**。实测那条拼写是 `link.rclonelink`
    /// 而不是 `link`（`rclone copy <junction 夹具> <本地临时目录> --links` →
    /// `ERROR : link.rclonelink: Failed to copy: symlinkat …`、exit=1）：这台机器造不出那个对象，
    /// 但 rclone 已经在**按这个名字推**了。对期望集合来说这照样是同一件事：本地扫描按"不跟随"
    /// 永远给不出 `link.rclonelink` 这条 key。（换成对象存储的目标能不能写成功，没量 —— 云端
    /// 在范围外；闸门不赌这个，赌的就是"两侧拼写集合必须同源"。）
    #[test]
    fn rejects_links_flag_because_it_changes_the_objects_that_land() { flag_error("--links"); }

    /// `--junction` 在钉住的 v1.75.1 里**不存在**：实测 `Error: unknown flag: --junction`、exit=2
    /// （`--links-follow-template`、`--check-links` 实测同样 unknown，见正向测试的注释）。
    /// 仍然留在黑名单里是**防御性**的，理由要说清：这个 token 进了 rcd 的 argv 会让 rcd 启动就失败，
    /// 在配置层拒绝它给操作员的是点名 flag 的中文报错，而不是 rcd 的 exit=2；而且哪天上游真加了
    /// 这个 flag，配置层已经拦着了。
    #[test]
    fn rejects_junction_flag_even_though_v1_75_1_does_not_have_it() { flag_error("--junction"); }

    /// `--flag=value` 那一档照旧要拦（沿用 `--rc-*` 已有的匹配风格）。
    #[test]
    fn rejects_link_flag_with_an_equals_form() { flag_error("--links=true"); }

    /// `--local-links` 是 local backend 自己的那个链接开关（钉住的 v1.75.1 `rclone help flags`：
    /// `--local-links  Translate symlinks to/from regular files with a '.rclonelink' extension for
    /// the local backend`，短 flag 那一列是空的 → 只有长写法，不需要拆字母）。
    /// 实测（报告 §17 / `docs/superpowers/evidence/build-notes.md`）它和 `--links` 是同一后果，
    /// 而且第一次量的时候还低估了它：**目标端不支持符号链接时它真的落地**
    /// （`rclone copy <junction 夹具> :memory: --local-links` → exit=0、`Transferred: 4 / 4`，
    /// 多出来的那条就是 `link.rclonelink`）。
    /// 裸写与 `=true` 走链接类闸门（点名 flag + 实测话术）。`--local-links=false` 实测与默认
    /// 一字不差，所以在 [`link_flag_explicitly_off`] 那张豁免表上 → 见
    /// [`accepts_boolean_link_flags_explicitly_switched_off`]，它被放过。
    #[test]
    fn rejects_local_links_flag_because_the_rclonelink_object_really_lands() {
        for flag in ["--local-links", "--local-links=true"] {
            let err = flag_error(flag);
            assert!(err.contains("链接"), "{flag} 的报错要说清是链接语义，实际是：{err}");
        }
    }

    /// 显式 `=false` 的布尔链接写法实测与默认**一字不差**（同一棵 junction 夹具，五条都是 exit=0、
    /// 落地都只有 `rootfile.webp sub/a.webp sub/deep/c.webp`、`NOTICE: link: Can't follow symlink
    /// without -L/--copy-links` 照样出现）。它们不改不变量 → [`link_flag_explicitly_off`] 里白名单
    /// 之外的独立豁免，闸门这一档不拦、其余按 no-op 放过。第五种拼写 `--local-links=false`
    /// 同样实测等于默认，所以也在这张表上。
    #[test]
    fn accepts_boolean_link_flags_explicitly_switched_off() {
        for flag in ["--links=false", "--copy-links=false", "-L=false", "-l=false",
                     "--local-links=false"] {
            flag_is_accepted(flag);
        }
    }

    // ---- 白名单：`[defaults] flags` 只认点名放行，其余一律拒绝 ----
    // 一条"别顺手拦别的"式的正向测试会把绕过钉成期望行为，正是这一类口子能活过 review 的原因，
    // 所以把它反过来：白名单比"精确/前缀匹配"更严，凡是不在名单上的 flag（含只与危险 flag
    // 共享前缀的合法 flag）都拒。

    /// 每一条允许项都必须**能被解析并放过**，这样名单里不可能藏一个 rclone 启动就会拒的拼写错误。
    /// 名单是数据，不是话术：这条测试遍历它自己，加一条就自动被覆盖。
    #[test]
    fn every_allowlisted_flag_parses_and_is_accepted() {
        for flag in ALLOWED_FLAGS.iter().chain(ALLOWED_BOOL_FLAGS) {
            flag_is_accepted(flag);
        }
        // `--flag value` 形式（SPEC_SAMPLE 就是这么写的）：值紧跟在白名单 flag 后面 → 放行。
        flag_is_accepted("--transfers");
        // 允许的单字母与它们的合并写法都要过（P/v/q 都是无值 bool）。
        for flag in ["-P", "-v", "-q", "-Pv", "-vP", "-Pqv"] {
            flag_is_accepted(flag);
        }
    }

    /// `--flag=value` 那一档（值里含空格/多段）也要过——这是面板默认里 `--transfers 32` 的另一种写法。
    #[test]
    fn allowlisted_flags_with_attached_value_are_accepted() {
        for flag in ["--transfers=32", "--checkers=64", "--stats=1m", "--bwlimit=1M"] {
            flag_is_accepted(flag);
        }
    }

    /// 过滤类**整族**拒绝，哪怕 `--exclude` 看起来人畜无害：排除只有一个入口 —— `exclude` 配置字段
    /// （它同时喂给 rclone 的 `--exclude` 启动参数和面板自己的匹配器），从 `flags` 再开第二条路
    /// 就是两侧悄悄不再同一次匹配的来源。实测（报告 §18 / build-notes）：`--include` 与面板的
    /// `--exclude` 并写时 rclone 只留一条、还只打一行"order … indeterminate" ERROR、exit=0。
    #[test]
    fn refuses_the_whole_filter_flag_family_even_exclude() {
        for flag in ["--exclude", "--exclude-from", "--include", "--include-from",
                     "--filter", "--filter-from", "--files-from"] {
            let err = flag_error(flag);
            assert!(err.contains("exclude"), "{flag} 的报错要指回 exclude 配置字段，实际是：{err}");
        }
    }

    /// 顺序保证：一个 `-` 开头的 token 走不走白名单，**与上一条 flag 期不期待一个值无关**。
    /// `["--transfers", "--exclude", …]` 里那个 `--exclude` 站的是 `--transfers` 的**值位**，
    /// 但第 4 步先看它 `-` 开头，就按 flag 处理：拿到过滤类整族那句"排除只有一个入口"，
    /// 既不是通用白名单话术，也不会被当成 `--transfers` 的值悄悄放过去。
    /// 第二组顺带钉住 `--fast-list` 是 bool（不吃下一个 token，§18.8.2 那一档），
    /// 所以它后面的 `--exclude` 更是无路可走。
    /// 这一条钉的正是一句容易说反的话（链接闸门=第 3 步、白名单=第 4 步，值位不豁免）：
    /// 判据落成测试而不是只写在注释里 —— 以后谁把值位做成"跳过白名单"，这里先红。
    #[test]
    fn refuses_a_filter_flag_even_when_it_sits_in_a_value_position() {
        for flags in [
            &["--transfers", "--exclude", "secret/*"][..],
            &["--fast-list", "--exclude", "a"][..],
        ] {
            let err = flags_error(flags);
            assert!(err.contains("--exclude："), "报错要点名 --exclude，实际是：{err}");
            assert!(err.contains("过滤类"), "{flags:?} 要拿到过滤类整族的话术，实际是：{err}");
            assert!(err.contains("一个入口"),
                "{flags:?} 的报错要说\"排除只有一个入口\"，实际是：{err}");
            // 通用白名单话术独有的那句**不许**出现：出现就说明这个 token 被当成别的什么处理了。
            assert!(!err.contains("正面点名放行"),
                "{flags:?} 不该拿到通用的白名单话术，实际是：{err}");
        }
    }

    // ---- 比法那一族：整闸门只点名放行 `--size-only` 这一枚 ----
    //
    // 这道闸门的判据本来是"只改怎么跑、不改哪些东西进去"，而比法**不在**那句里：改比法改的是
    // "什么算变了"。这一枚是点名批准开进来的（实测：同一对目录、6,107 个对象，
    // 默认比法 52~57 秒、只比大小 12~15 秒，差的就是那 6,107 次逐对象问远端要修改时间）。
    // 这一枚是**唯一**的例外，代价写在 [`ALLOWED_BOOL_FLAGS`] 的注释上，界面上两处要说话。

    /// 这一枚要放过：它现在在白名单里，[`every_allowlisted_flag_parses_and_is_accepted`]
    /// 那条遍历名单的测试会自动验证 rclone 认这个拼写（v1.75.1 `rclone help flags` 实测在册）。
    #[test]
    fn the_one_comparison_flag_is_allowed() {
        flag_is_accepted("--size-only");
    }

    /// bool 那一族只做**精确等于**（那是有意为之的不对称），所以带值的写法照旧拒。
    /// 这条不是凑数：`--size-only=false` 如果被放过，rclone 按 false 跑、而
    /// [`compares_by_size`] 按"出现过这个名字"算，界面上那句就说成一个没在跑的比法。
    #[test]
    fn a_size_only_flag_with_an_attached_value_stays_refused() {
        for flag in ["--size-only=false", "--size-only=true"] {
            let err = flag_error(flag);
            assert!(err.contains("白名单"), "{flag} 要说清是白名单拒的，实际是：{err}");
        }
    }

    /// 其余改比法的拼写一律拒（每一条都是 v1.75.1 `rclone help flags` 实测在册的真实写法，
    /// 不是编出来凑数的假 token——名单要拒的东西得先证明它存在）。
    /// 其中 `--use-server-modtime` 是实测**否掉**的那一条捷径：它同样省掉逐对象那一问，
    /// 但实测有 1,091 个对象要为此重写一次修改时间，而重写又把服务器时间推后 → 每趟都收敛不了。
    #[test]
    fn every_other_comparison_flag_stays_refused() {
        for flag in ["--ignore-checksum", "--ignore-size", "--modify-window",
                     "--use-server-modtime", "--no-update-modtime", "--track-renames"] {
            let err = flag_error(flag);
            assert!(err.contains("白名单"), "{flag} 要拿到白名单那句：{err}");
        }
    }

    /// 判定只有一处家：顶栏那句和每条任务日志里的 `compare=` 都读 [`compares_by_size`]。
    /// 界面/日志各自再判一次，就是"屏上写着只比大小、实际按时间比"这类谎的来路。
    #[test]
    fn compares_by_size_reads_the_exact_token_only() {
        use super::compares_by_size;
        assert!(compares_by_size(&["--size-only".into()]));
        assert!(compares_by_size(&["--fast-list".into(), "--size-only".into()]));
        assert!(!compares_by_size(&["--fast-list".into()]));
        assert!(!compares_by_size(&[]));
        // 带值的写法根本过不了上面的闸门，这里也不把它算成"只比大小"：两处口径一致。
        assert!(!compares_by_size(&["--size-only=false".into()]));
        assert!(!compares_by_size(&["--size-only-x".into()]));
    }

    /// 其余"改的是什么会进桶"的写法，实测都会制造**本地扫描给不出的删除候选**，一律拒：
    /// - `--suffix` / `--backup-dir`：move-then-copy，实测目标端**同时**留着 `002.webp` 和
    ///   `002.webp.bak`（内容变了的那一次 copy），`actual − expected` 白送一条删除候选、exit=0、
    ///   一声不响；
    /// - 任何不在名单上的普通 flag（`--checksum`、`--list-cutoff`、`--skip-links`、`--dry-run`、
    ///   `--ignore-case`、`--transfers-limit`）——白名单不猜语义，不在就是拒。
    #[test]
    fn refuses_everything_not_on_the_allowlist() {
        for flag in ["--suffix", "--backup-dir", "--checksum", "--list-cutoff",
                     "--skip-links", "--dry-run", "-n", "--ignore-case", "-I",
                     "--transfers-limit", "--links-follow-template", "--check-links"] {
            let err = flag_error(flag);
            assert!(err.contains("白名单"), "{flag} 要说清是白名单拒的，实际是：{err}");
        }
        // 只与危险 flag 共享前缀 / 子串撞上字母的合法 flag，同样被白名单拒（不靠拆字母那档）。
        flag_error("-c");
        flag_error("-vPc"); // v、P 允许，但 c 不在名单上 → 整条拒
    }

    /// 非 `-` 开头、且前面不是白名单 flag 的裸 token（值没有主人 → 是 rcd 的位置参数）要拒。
    /// 反过来 `--transfers 32` 里那个 `32` 有主人 → 放行（SPEC_SAMPLE 依赖这一条）。
    /// 正向半边也要钉：只测拒绝的话，`flag_takes_separate_value` 退化成"永远 false"
    /// 也能让拒绝半边漂亮地绿，而 spec 默认里那两条 `--transfers 32` / `--checkers 64` 会一起躺下。
    #[test]
    fn refuses_an_orphan_value_token_but_accepts_a_flag_owned_value() {
        // 拒绝半边：第一个 token 就是裸值：它不可能是任何 flag 的值。
        let text = SPEC_SAMPLE.replace("flags = [", "flags = [\"node_modules\", ");
        let err = JobsFile::parse(&text).unwrap_err().to_string();
        assert!(err.contains("node_modules"), "孤立裸值要报错，实际是：{err}");
        // 正向半边：裸值紧跟在**白名单取值 flag** 后面 → 是它的值，放行。
        // `["--transfers", "32"]` 就是 spec 默认依赖的那个形状；其余几条是它的组合。
        for flags in [
            &["--transfers", "32"][..],
            &["--checkers", "64", "--transfers", "32"][..],
            &["-v", "--bwlimit", "1M", "--retries", "3"][..],
        ] {
            flags_are_accepted(flags);
        }
        // 半个反例钉住"值归属只认取值 flag"：bool 白名单 flag 后面那个裸 token 没有主人 → 仍拒。
        // （§18.8.2：`--fast-list` 一度被放进取值名单，就是这一档漏的。）
        let err = flags_error(&["--fast-list", "32"]);
        assert!(err.contains("32："), "孤立的 32 要点名，实际是：{err}");
    }

    #[test]
    fn defaults_log_dir_and_pid_path_are_absolute() {
        // 这条也算"碰那两个变量"：`from_defaults` 读 `RCLONE_DECK_HOME` 算 `pid_path`、
        // 读 `RCLONE_DECK_PORT` 定端口。不持锁时它会被同一条 test target 里那几条 R52
        // 用例的相对值撞红（`npm run test:fast` 不带 `--test-threads=1`，实测红过一次）。
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // 端口这一条钉的是**默认值**，所以先把变量摘掉：它默认没被设过，可这条钉的是
        // "操作员的日常路径一个字节没变"，被同 target 的另一条用例设过就不算证据了。
        std::env::remove_var("RCLONE_DECK_PORT");
        let cfg = AppConfig::from_defaults(&Defaults {
            rclone_bin: "C:\\rclone\\rclone.exe".into(),
            flags: vec![], exclude: vec![],
            log_dir: "D:\\Sync\\logs\\rclone-deck".into(),
            check_by_hash: false,
        }).unwrap();
        assert!(cfg.pid_path.is_absolute());
        // 2026-10-10：这一条原来是 `== AppConfig::PORT`。默认端口改成现挑之后它就该钉
        // **性质**（拿到的号绑得上）而不是某个数 —— 名义默认 6400 在本机的排除区
        // （6383–6482）里，钉死那个数恰好是让"面板起不来"通过的那一种红。
        assert!(std::net::TcpListener::bind(("127.0.0.1", cfg.port)).is_ok(),
            "没设端口时挑出来的 {} 绑不上：它此刻在排除区里或被别人占着", cfg.port);
    }

    // R53：RCLONE_DECK_PORT 开关。2026-10-09 起 —— 本机的动态排除端口区有一轮正好盖住
    // 5460–5659，把 5599 一起吃了，`rclone rcd` 于是报 `bind: … forbidden by its access
    // permissions`（不是"被占"，是"不许绑"），面板起不来后台。判据与 R52 那三条同一族，
    // 理由也写在 `config::deck_port` 的注释里。
    //
    // 环境变量是进程全局的：本仓库跑 --test-threads=1，串行下不会互撞；
    // 但「设了又泄漏给下一条」仍是陷阱。这里两重保险（与 R52 共用同一把锁与同一个 RAII 形状）：
    //   * DECK_ENV_LOCK：任何碰这两个变量的用例先持锁，即便哪天忘了 -1 也不会互撞；
    //   * DeckEnvVar：RAII，Drop 时 remove_var，就算断言 panic 也把变量收干净、不漏给后人。
    static DECK_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct DeckEnvVar;
    impl DeckEnvVar {
        // 只剩端口那一半：`RCLONE_DECK_HOME` 的证人 1–3 用的是下面那个 `DeckHomeVar`（R52），
        // 两处各管各的，所以这里不再留一个没人调用的 `home` 构造。
        fn port(value: impl AsRef<std::ffi::OsStr>) -> Self {
            std::env::set_var("RCLONE_DECK_PORT", value);
            DeckEnvVar
        }
    }
    impl Drop for DeckEnvVar {
        fn drop(&mut self) {
            std::env::remove_var("RCLONE_DECK_HOME");
            std::env::remove_var("RCLONE_DECK_PORT");
        }
    }

    // 证人 1（端口）：没设 → 落在**一个今天绑得上**的号上。
    //
    // 2026-10-10 改的判据：原来是 `== AppConfig::PORT`（"没设就是这个数"），而那个数已经被
    // 本机的排除区（6383–6482）吃了 —— 于是新用户装上面板第一次启动必然起不来，套件也整组红。
    // 抬高默认值救不了（它被咬过两次），所以默认这一档也现挑（`AppConfig::pick_port`）。
    // 现在判的是**性质**而不是某个数：拿到的号得真的绑得上。
    #[test]
    fn an_unset_deck_port_lands_on_a_port_that_actually_binds() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("RCLONE_DECK_PORT");
        let p = AppConfig::from_defaults(&fixture_defaults()).unwrap().port;
        // 唯一的事实来源：能不能绑上。落在排除区里的号 bind 会失败（WSAEACCES），
        // 而这正是 10-09/10-10 两次把整组用例带红的那件事。
        assert!(std::net::TcpListener::bind(("127.0.0.1", p)).is_ok(),
            "没设端口时挑出来的 {p} 绑不上：它此刻在排除区里或被别人占着，面板起不来");
    }

    // 证人 1b：**同一个号、两次算出来一样** —— 单实例那道闸就靠这个。
    //
    // `rcd::reclaim` 靠 pid 记录认领，而它认的前提是"同一用户端口是常量"（R42 item 3）。
    // 若 `pick_port` 改成"随便找个空的"，两个面板会各拿一个号、谁也看不见谁，于是起两个 rcd。
    // 所以这里钉的不是某个数，而是**确定性**。
    #[test]
    fn picking_the_default_port_twice_gives_the_same_answer() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("RCLONE_DECK_PORT");
        let a = AppConfig::from_defaults(&fixture_defaults()).unwrap().port;
        let b = AppConfig::from_defaults(&fixture_defaults()).unwrap().port;
        assert_eq!(a, b, "两次挑出来的端口不一样：单实例那道闸（reclaim 靠 pid 记录）就不成立了");
    }

    // 证人 1c：显式设的 `RCLONE_DECK_PORT` **优先于**现挑，即使那个号此刻绑不上。
    // 人要的就是那个号 —— 抢不抢得到是启动那一步按 `reclaim` 去说的话术，不是这里替他改主意。
    #[test]
    fn an_explicit_deck_port_wins_over_the_picker_even_when_it_cannot_bind() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _var = DeckEnvVar::port("1");   // 1 几乎肯定绑不上（特权端口）
        assert_eq!(AppConfig::from_defaults(&fixture_defaults()).unwrap().port, 1);
    }

    // 证人 2（端口）：设了它 → 后台起在那个端口上。这是这个开关存在的全部理由。
    #[test]
    fn deck_port_env_moves_the_rcd_listener() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _var = DeckEnvVar::port("5601");
        assert_eq!(AppConfig::from_defaults(&fixture_defaults()).unwrap().port, 5601);
    }

    // 证人 3（端口）：空串按**报错**处理，不当没设。半设置静默回落到默认端口，
    // 落到的正是那条开关要躲开的、这轮正好被排除区吃掉的端口 —— 与 R52 空串那条同一纪律。
    #[test]
    fn an_empty_deck_port_is_rejected_not_treated_as_unset() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _var = DeckEnvVar::port("");
        let err = AppConfig::from_defaults(&fixture_defaults()).unwrap_err().to_string();
        assert!(err.contains("RCLONE_DECK_PORT"), "端口侧的报错没点名变量：{err}");
    }

    // 证人 4（端口）：0 与非数字都拒。0 尤其要拒 —— 它在 rclone 的 `--rc-addr` 那里意味着
    // "随便挑一个"，而端口号要写进 pid 记录与错误话术，挑出来的值没人知道。
    #[test]
    fn a_zero_or_unparsable_deck_port_is_rejected_naming_the_variable() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for bad in ["0", "not-a-port", "70000", "-1"] {
            let _var = DeckEnvVar::port(bad);
            let err = AppConfig::from_defaults(&fixture_defaults()).unwrap_err().to_string();
            assert!(err.contains("RCLONE_DECK_PORT"), "端口 {bad:?} 没被点名报错：{err}");
        }
    }

    // R52：RCLONE_DECK_HOME 开关。这三条证人挂在 config 目标里，无 rcd。
    struct DeckHomeVar;
    impl DeckHomeVar {
        fn set(value: impl AsRef<std::ffi::OsStr>) -> Self {
            std::env::set_var("RCLONE_DECK_HOME", value);
            DeckHomeVar
        }
    }
    impl Drop for DeckHomeVar {
        fn drop(&mut self) {
            std::env::remove_var("RCLONE_DECK_HOME");
        }
    }

    fn fixture_defaults() -> Defaults {
        Defaults {
            rclone_bin: "C:\\rclone\\rclone.exe".into(),
            flags: vec![], exclude: vec![],
            log_dir: "D:\\Sync\\logs\\rclone-deck".into(),
            check_by_hash: false,
        }
    }

    // 证人 1：设了它 → 两个落点都在这棵子树里，且同源（父目录相等，比断言字符串形状抗重构）。
    #[test]
    fn deck_home_env_redirects_both_landing_points_to_one_root() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::TempDir::new().unwrap();
        let _var = DeckHomeVar::set(home.path());

        let jobs = AppConfig::jobs_toml_path().unwrap();
        let pid = AppConfig::from_defaults(&fixture_defaults()).unwrap().pid_path;

        assert!(jobs.starts_with(home.path()), "jobs.toml 不在夹具子树里：{}", jobs.display());
        assert!(pid.starts_with(home.path()), "rcd.pid 不在夹具子树里：{}", pid.display());
        // 同源：两条的父目录必须相等，否则夹具窗口会读夹具配置却把 pid 写去操作员目录。
        assert_eq!(jobs.parent(), pid.parent(), "两个落点不同源：{:?} vs {:?}", jobs.parent(), pid.parent());
    }

    // 证人 2：没设 → 两者都不在这棵子树里（「照旧」的证据；缺了它下次改动会悄悄默认化）。
    #[test]
    fn unset_deck_home_leaves_both_landing_points_on_the_known_folders() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("RCLONE_DECK_HOME");
        let home = tempfile::TempDir::new().unwrap();

        let jobs = AppConfig::jobs_toml_path().unwrap();
        let pid = AppConfig::from_defaults(&fixture_defaults()).unwrap().pid_path;

        assert!(!jobs.starts_with(home.path()), "未设开关时 jobs.toml 却落进了夹具子树：{}", jobs.display());
        assert!(!pid.starts_with(home.path()), "未设开关时 rcd.pid 却落进了夹具子树：{}", pid.display());
    }

    // 证人 3：相对值 → Err，且话术点名 RCLONE_DECK_HOME（两侧都得拒）。
    #[test]
    fn a_relative_deck_home_is_rejected_naming_the_variable() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _var = DeckHomeVar::set(std::path::Path::new("rclone-deck/relative"));

        let jobs_err = AppConfig::jobs_toml_path().unwrap_err().to_string();
        assert!(jobs_err.contains("RCLONE_DECK_HOME"), "jobs 侧报错没点名变量：{jobs_err}");
        let pid_err = AppConfig::from_defaults(&fixture_defaults()).unwrap_err().to_string();
        assert!(pid_err.contains("RCLONE_DECK_HOME"), "pid 侧报错没点名变量：{pid_err}");
    }

    // 我的裁定：空串按报错处理，不当没设——半设置静默回落到真配置正是要挡的事故。
    #[test]
    fn an_empty_deck_home_is_rejected_not_treated_as_unset() {
        let _lock = DECK_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _var = DeckHomeVar::set("");

        let err = AppConfig::jobs_toml_path().unwrap_err().to_string();
        assert!(err.contains("RCLONE_DECK_HOME"), "空串报错没点名变量：{err}");
    }
}
