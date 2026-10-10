//! 写动作的坐标系（裁定 R71 / spec I3）。
//!
//! 为什么不是 `Job::mutex_key()` 那个 `src=>dst` 方向串：那把锁只认"同一对任务"，
//! 而浏览器与对齐**不共享任务**却共享同一棵树。方向串既挡不住"浏览器 purge 掉对齐正在预览的
//! 前缀"，也说不清"哪两个动作会互相踩"。Scope 说的是位置，不是任务身份。
//!
//! `root` 只到**远端名/盘符**为止，桶名与目录都进 `prefix`：桶名是远端内的第一段路径，留在 `root` 里时
//! `r2:img` 与 `r2:imgprompt` 会因根字符串不等而"自动不冲突"，祖先判定一步都没走。钉住这一条的是证人
//! `a_bucket_inside_the_fs_string_is_one_tree_with_its_subprefixes_but_not_with_its_neighbour` 里那句
//! `key() == "r2:#img"` —— `conflicts()` 那两句（父子、lookalike）在桶名进 `root` 的形状下照旧成立。
use std::path::Path;

use crate::endpoint::Endpoint;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Scope {
    root: String,
    prefix: String,
}

/// `\` → `/`；首尾斜杠与中间的空段一起压掉，否则同一个目录会有两个名字（`a//b`、`a/b/`）。
///
/// **只压空段，不折叠 `.` 与 `..`**，这是刻意的：折叠它们要猜"这个名字里哪一段是路径、哪一段是
/// 对象名"——对象键里 `..` 是合法字符（某些后端真能给出 `a/../b` 这样的 key），按本地语义去化简
/// 会把一个真存在的名字改写成另一个名字。代价是 `img/../other` 与 `other` 在这里是**两个键**，
/// 于是同一棵树上的两个写者判不出冲突（裁定 R90 要关的正是这一格）。
/// 所以这个洞不在这里堵，而是在**边界**堵：见 [`has_dot_segment`]，配置侧与浏览器侧共用那一条判据。
fn norm(s: &str) -> String {
    s.replace('\\', "/").split('/').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("/")
}

/// 这个名字里有没有**整段等于 `.` 或 `..`** 的一节（两种斜杠都算分隔符，与 [`norm`] 同一个切法）。
///
/// 判据是"整段相等"，不是"含点"：`a.b.webp`、`foo..bar.webp`、`in-progress/2026` 都是正常名字，
/// 必须照旧放行。
///
/// 为什么全仓只有这一处：带点段的名字一旦进键，`conflicts()` 就算不出"这是同一棵树"
/// （理由见 [`norm`] 头上那段），而"这一行点中的是哪棵树"必须与写动作申领的那个键是同一件事。
/// 拦它的说法**不涉及 rclone 会不会把 `..` 解析掉**——那一格没有实测过（裁定 R95/R89），
/// 也没必要：面板只需要"两条键挡不住同一棵树"这一条理由就能拒。
/// 读者名单：`config/jobs.rs` 的 parse（配置里写出来的两侧）与 `runner::browser`
/// （界面在一次动作里交回来的 `fs`/前缀）。别在别处再拼第二份"点段怎么认"。
pub fn has_dot_segment(raw: &str) -> bool {
    raw.replace('\\', "/").split('/').any(|seg| seg == "." || seg == "..")
}

impl Scope {
    /// `name` 是远端名（含结尾冒号，如 `r2:`）；`prefix` 是它下面的路径，可以为空。
    pub fn remote(name: &str, prefix: &str) -> Self {
        Self { root: name.trim().to_string(), prefix: norm(prefix) }
    }

    pub fn local(path: &Path) -> Self {
        // 大小写不敏感：归一到小写，否则 `D:\Vault` 与 `d:\vault` 会被判成两棵树。
        let s = norm(&path.to_string_lossy()).to_lowercase();
        // 根 = **第一段**（盘符 `d:`、UNC 的服务器名、或相对路径的首段），prefix = 其余。
        // 为什么不能"整串当根"：`\\srv\share` 与 `\\srv\share\img` 就成了两个不等的根 ⇒ 不冲突，
        // 于是"两个写动作同时落进同一棵树"从这条缝里漏过去 —— 那正是本项目的起点事故（R84）。
        // 代价是往安全那一侧偏：相对路径 `srv/share` 与 UNC `\\srv\share` 会被判成同一棵树（多拦不误放）。
        // 面板的本地端点只来自 jobs.toml 里的绝对路径，所以这条偏差不影响任何现有任务。
        match s.split_once('/') {
            Some((head, rest)) => Self { root: head.to_string(), prefix: rest.to_string() },
            None => Self { root: s, prefix: String::new() },
        }
    }

    pub fn of_endpoint(ep: &Endpoint) -> Self {
        match ep.remote_target() {
            Some((fs, remote)) => {
                // `fs` 到桶名为止（`r2:img`，见 endpoint.rs 的注释），所以冒号前当根、
                // 桶名并进 prefix 的第一段。没有冒号的形状原样当根。
                match fs.split_once(':') {
                    Some((name, bucket)) => {
                        let joined = if bucket.is_empty() { remote.to_string() } else { format!("{bucket}/{remote}") };
                        Self::remote(&format!("{name}:"), &joined)
                    }
                    None => Self::remote(fs, remote),
                }
            }
            None => Self::local(ep.as_local().expect("端点既不是 remote 也不是本地路径")),
        }
    }

    /// 锁与登记表用的键。`#` 只是把 `root` 与 `prefix` 分开的分隔符，**不是**"哪里都不会出现"的字符：
    /// 对象名里 `#` 是合法的，所以这个映射不保证一对一。
    ///
    /// 撞在一起需要什么：`root` 自己带 `#`。只要两种根都不带 `#`，键里**第一个** `#` 就唯一地是分隔符，
    /// 于是"同键 ⇒ 同 `root` 且同 `prefix` ⇒ 同一个 `Scope`"成立（前缀里有多少个 `#` 都不影响，
    /// 它整段落在分隔符右边）。当下的两种根形（远端名 `r2:`、本地首段＝盘符或 UNC 服务器名）都不带 `#`，
    /// 但"远端名里到底能不能出现 `#`"没有实测过，所以这一段是**形状说明**，不是担保。
    ///
    /// 一旦真撞了，后果不再是"两把锁并成一把、多拦不误放"（这句话在裁定 R90 之后**不成立了**，
    /// 控制者复核时改的）：登记表以键为索引，而冲突判定读的是 `Scope` 值，
    /// 于是同键的两棵不同的树在表里**互相看不见**——第二次的 `insert` 会把第一条活着的登记顶掉，
    /// 那棵树从此没人挡（**误放**，本项目的最坏方向），而先那条的 `Drop` 会因为 token 不匹配
    /// 落到"键已被 force_release"那句多出来的读数上。要真关掉这一格，得让键一对一（或让登记表
    /// 不按键覆盖）。冲突判定本身不读这个键：`conflicts()` 比的是 `root` 与 `prefix` 两个字段，
    /// 键只给锁与登记表用。
    pub fn key(&self) -> String {
        format!("{}#{}", self.root, self.prefix)
    }

    /// 给人看的那一条（I1 的"回显完整路径"就落在这里）。根形有**三种**，拼法不能一视同仁：
    /// 远端根自带结尾冒号（`r2:` + `img` = `r2:img`，中间不该有斜杠），而盘符根（`d:`）与
    /// 不以冒号结尾的本地首段（相对路径 `srv\share` 与 UNC `\\srv\share` 都切出 `srv`）都要插斜杠 ——
    /// 盘符不插会拼成 `d:vault`，看着像一个远端名（操作员按远端去理解就错了）；
    /// 首段不插会把 `srv` 与 `share` 粘成 `srvshare`，边界就读没了。
    /// 盘符与远端名都以冒号结尾，只看结尾分不开两者，判据用**根的长度**（见 `is_drive_root()`）：
    /// 恰好两个字符的是盘符（`local()` 归一后是小写），更长的冒号结尾根是远端名。
    /// 三种根形在证人 `describe_reads_as_a_path_for_the_three_root_shapes` 里各钉两句：带前缀的与只到根的。
    /// 残余：一个真叫 `x` 的远端（根 `x:`）按这条长度规则落进盘符那一形，显示成 `x:/a`。
    /// 两种写法都读得出是哪棵树，这里不为它加判别式：真要加就加进 `Scope` 的构造
    /// （构造那一刻本来就知道自己来自 `remote()` 还是 `local()`），不要加进字符串猜测 ——
    /// 长度规则就是这一支的边界，再猜一个字符类别也分不开 `x:` 与 `d:`。
    pub fn describe(&self) -> String {
        if self.is_drive_root() {
            // 盘符根：`d:/vault/img`（正斜杠是 `norm()` 定下的仓库内部规范形状）。
            if self.prefix.is_empty() { self.root.clone() } else { format!("{}/{}", self.root, self.prefix) }
        } else if self.root.ends_with(':') {
            // 远端根：`r2:` + `img` → `r2:img`（中间不加斜杠）
            format!("{}{}", self.root, self.prefix)
        } else {
            // 不以冒号结尾的本地首段（UNC 的 `srv`、相对路径的首段）：也必须插斜杠，
            // 直接拼会把 `srv` 与 `share` 粘成 `srvshare`，边界就读没了。
            if self.prefix.is_empty() { self.root.clone() } else { format!("{}/{}", self.root, self.prefix) }
        }
    }

    /// 盘符根 = **两个字符**且以 `:` 结尾（`d:`）；远端名的根至少三个字符（`r2:` 含冒号正好三个）。
    /// 单字母远端名落进盘符那一形是残余，见 `describe()` 的注释。
    ///
    /// 这条判据与 `endpoint::remote_split` 的"冒号前不止一个字符"（`src-tauri/src/endpoint.rs`）
    /// 是**同一个问题上的姊妹判据**：两边都在答"冒号前面是盘符还是远端名"，只是各答一遍。
    /// 两处必须同步改：只改一处，同一个端点就会在"是不是本地"上得到两个答案
    /// （那边决定 `Endpoint::Local` 还是 `Remote`，这边决定那句回显按哪种根形拼）。
    /// 不合并成一份，是因为两处拿到的输入形状不同：那边是**裸串**，`remote_split` 交出
    /// `(scheme, rest)`，到桶名为止的 `fs` 由 `endpoint_of` 拼；这边是已经切好的 `root`
    /// （`local()` 与 `of_endpoint()` 各切过一刀）。
    /// 要合就得让 `scope` 反向依赖端点解析的切法，或让 `endpoint` 知道锁的坐标系，两条都比同步贵。
    fn is_drive_root(&self) -> bool {
        self.root.len() == 2 && self.root.ends_with(':')
    }

    /// `a` 是 `b` 的祖先或就是它。空前缀 = 该根的全体，所以是所有前缀的祖先。
    fn is_ancestor(a: &str, b: &str) -> bool {
        a.is_empty() || a == b || b.starts_with(&format!("{a}/"))
    }

    /// 两棵树会不会互相踩。**对称**，且不包含 = 不冲突。
    pub fn conflicts(&self, other: &Scope) -> bool {
        self.root == other.root
            && (Self::is_ancestor(&self.prefix, &other.prefix) || Self::is_ancestor(&other.prefix, &self.prefix))
    }
}
