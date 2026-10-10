use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{types::ListEntry, ListError, ListShape, RcClient};

/// 一次调用拿到整棵树的**文件**条目（`operations/list` + 一枚带 `recurse` / `filesOnly` /
/// `noModTime` / `noMimeType` 四个键的 `opt`，完整串只定义在 `ListShape::opt` 一处）。
///
/// 这里以前是逐层 BFS：按 `IsDir` 下探、每个目录发一个 LIST，配 `max_requests` 跑飞闸门和
/// `visited` 防环。量过：递归开关走 **form 编码**也生效（`tools/rc-contract-probe.mjs`
/// §4b 钉着，带对照组；把 flag 拿掉那条会变红），于是换成一次调用。同一份夹具
/// （10,020 个对象 / 200 个目录）实测：**203 个请求 2894 ms → 1 个请求 59 ms**，
/// 两侧集合逐字节相同、`--exclude` 在两种走法里同样生效、子路径 `remote` 返回的 `Path`
/// 一样带前缀（`ideahub/covers/deep/b.webp`），所以归一化那套一字未动。
///
/// 那两样防护不是被放宽了，是**没了对象**：没有循环就没有跑飞，不需要预算；更重要的是，
/// 203 次请求之间桶可以被别人改 203 次，集合是那些时刻的拼贴——现在是一份快照。
///
/// 失败口径留在 `RcClient::operations_list`：404 → `NotFound`（调用方按"这一侧是空的"还是"报错"分情况处理，
/// 见裁定 R64）、缺 `list` 键 / 非数组 / 200 里带 error 一律报错，**绝不把"没看见"读成"是空的"**。
pub async fn list_files_recursive(
    client: &RcClient,
    fs: &str,
    remote: &str,
) -> Result<Vec<ListEntry>, ListError> {
    Ok(client
        // 这一处**要**整棵树：集合差与盘点的两侧都得是同一时刻的完整快照。
        // 浏览器那一面是另一档（单层，见 `runner::browser::browser_list`）——两边不许互相顺手改掉，
        // 各自有证人钉着（`tests/frontend_browser.rs` 与 `tests/rc_client.rs`）。
        .operations_list(fs, remote, ListShape::Tree)
        .await?
        .into_iter()
        .filter(|e| !e.is_dir)
        .collect())
}

/// 实际侧：`Path` 已经是"相对 fs 根"的完整 key（实测 `remote="ideahub"` 拿到的就是
/// `ideahub/covers/a.webp`），**不能再拼一次前缀**。
/// 所以这个函数压根不收 `prefix`：拼不了，也就拼不错。坐标系是"相对桶根、正斜杠、
/// 无前导斜杠"，rc 替我们拼好了前半段；期望侧必须自己拼（见 `normalize_expected`）。
/// 这个不对称是有意的——本地扫描从映射目录起步，远端列举从桶起步。
pub fn normalize_actual(entries: &[ListEntry]) -> BTreeSet<String> {
    entries.iter()
        .filter(|e| !e.is_dir)
        .map(|e| e.path.replace('\\', "/"))
        .filter(|p| !p.is_empty())
        .collect()
}

/// 对齐的坐标系：**相对列举根自己**的名字，不是桶全 key。
///
/// 为什么与 `normalize_actual` 故意不同：gate 交出去的是发布许可，它要比"桶里有没有这个完整 key"，
/// 所以两侧都拼到桶全 key（§5.1）。对齐要圈定删除范围——`sync` 删的是**目标端那一行的名字**，
/// 操作员在目标端能看见的也正是它。用桶全 key，下载方向就得把桶前缀翻译回本地路径，
/// 而那一步正是 §4.1 记着的那次假清单事故。
///
/// `root` 是列举根（`remote=` 那一段，可以为空）。回来的 `Path` 不以它开头就是坐标系对不上：
/// **报错**。少剥或多剥一个分隔符，交出的就是一份范围错的删除清单。
pub fn normalize_relative(root: &str, entries: &[ListEntry]) -> Result<BTreeSet<String>, ListError> {
    let root = root.trim_matches('/');
    let prefix = if root.is_empty() { String::new() } else { format!("{root}/") };
    let mut out = BTreeSet::new();
    for e in entries.iter().filter(|e| !e.is_dir) {
        let path = e.path.replace('\\', "/");
        if !root.is_empty() && path == root {
            // 列举根自己作为"文件"回来了：它不是根下面的一个对象，没有相对名可给，
            // 也永远不可能是删除目标（`sync` 删的是根之下的 key）。跳过，不报错。
            continue;
        }
        let rel = path.strip_prefix(prefix.as_str()).ok_or_else(|| ListError::Invariant(format!(
            "列举根 {root:?} 之下回来了一个 {path}，它不以列举根开头 —— 两侧坐标系对不上，\
             拒绝交出一份删除范围不明的清单。")))?;
        if !rel.is_empty() {
            out.insert(rel.to_string());
        }
    }
    Ok(out)
}

/// 本地扫描的工作上限。**封顶的是工作量，不是深度**：
/// 深度上限既拦不住"浅而巨大"的库，也拦不住 2^m 形状的镜像链，而后者才是真实的那一种
/// （实测 `rclone copy -L` 撞一个指回祖先的 junction：255 次列举、128 个对象之后
/// 死在 Windows 的路径长度上，见 `build-notes.md`——rclone 自己没有环守卫）。
///
/// 数字是照本机实测的输入校准的，不是拍出来的：面板两个真实 `src` 目录
/// `D:\vault\img-r2` = 3 层目录 / 6,105 个文件，`D:\vault\imagehosting` = 8 层 / 1,556 个文件，
/// 整个 `D:\vault`（最宽的假设）= 12,123 层 / 47,254 个文件。取整个 vault 的四倍，
/// 意思是"正常的库永远碰不到这条线"，而失控的镜像链十几次翻倍就到。
pub const MAX_WALK_READ_DIRS: usize = 50_000;
/// 同上，管的是往期望集合里放了多少个 key。
pub const MAX_WALK_KEYS: usize = 200_000;

/// [`normalize_expected_with_budget`] 的工作预算。默认取上面两个常量；
/// 单独拿出来是为了这条上限有生产者、也有测试能打得到它（不然只能造一棵五万层的树）。
#[derive(Debug, Clone, Copy)]
pub struct WalkBudget {
    /// 最多打开多少层目录（一次 `read_dir` 算一层，`root` 自己算第一层）。
    pub read_dirs: usize,
    /// 最多往期望集合里放多少个 key。
    pub keys: usize,
}

impl Default for WalkBudget {
    fn default() -> Self {
        Self { read_dirs: MAX_WALK_READ_DIRS, keys: MAX_WALK_KEYS }
    }
}

/// 期望侧：本地扫描得到的是相对 `root` 的路径，必须拼上 `prefix` 才和实际侧同坐标系。
///
/// 这个函数唯一要守的不变量是**"期望集合 == rclone 会被推进桶的那批对象"**：
/// gate 用它算 `expected − actual`（缺图），prune 用它算 `actual − expected`（孤儿候选），
/// 而这两个差集里只有一个是安全的——`expected` 少一个 key，那个 key 就成了**删除候选**；
/// `expected` 多一个 key，只是多挡一次检查。所以下面每一条判据都是照着实测的 rclone 行为写的，
/// 不是照着"本地目录树应该长什么样"写的（证据：`docs/superpowers/evidence/build-notes.md`）。
///
/// - **重解析点（junction / 符号链接）不进目录，也不进 key 集合。**
///   实测：`rclone copy <src> <dst>`（面板的默认 flags，不含 `-L/--copy-links`）对 junction 报
///   `NOTICE: link: Can't follow symlink without -L/--copy-links`，那条拼写一个对象都不会落进目标。
///   死链接（目标没了）同理，而且它更阴：`path.is_dir()` 对它返回 **false**，照 `is_dir()` 分文件
///   还是目录的写法会把它当成一个文件收进集合（实测 `p/gone`），于是 gate 永远报一张传不上的缺图。
///   判据用 `DirEntry::file_type()`（来自父目录那一次枚举的数据，不再对路径做第二次会失败的 stat）：
///   `is_symlink()` 就是"名字代理重解析点"（junction 的 tag 实测 0xa0000003 = MOUNT_POINT），
///   而 OneDrive 占位符那一类不带名字代理位的 tag 报 `is_dir()`，照旧下探——两边在同一件事上一致。
///   **这条策略的外部前提**是 rcd 的启动参数里没有会改链接语义的 flag：加了 `-L`/`--copy-links`
///   rclone 会**跟随**链接、把 `link/...`（实测合并写法 `-Lv` 也一样）推进桶；加了 `-l`/`--links`/
///   `--local-links` 它不跟随，但把链接本身当成一个对象推上去，实测的拼写是 `<链接名>.rclonelink`
///   （`:memory:` 那侧 exit=0 真的落地）。这里按默认策略不跟随，桶里多出来的那条拼写就成了 prune 的
///   删除候选。（`--junction` 不在这一列——实测它在 v1.75.1 是 `Error: unknown flag`、exit=2，
///   什么都不推；`config::jobs` 仍拒绝它只是防御性的，见那里的 `BANNED_LINK_FLAGS` 注释。）
///   闸门现在的形状要说准（判据在 `config::jobs`）：`[defaults] flags` 走**正面白名单**，
///   不在名单上的 flag 一律拒——这比"改链接语义的那几条"宽得多，`--skip-links`、`--ignore-case` 这些
///   过去"因无害而放行"的写法现在也进不来（因为未被点名，不是因为它们有害；方向仍是过度拦）。
///   白名单之外还留着两条**单独点名**的拒绝以保住实测话术：rc 地址/鉴权黑名单，和链接类黑名单
///   （`-L`/`-l`/`--copy-links`/`--links`/`--local-links`/`--junction` 的裸写与 `=` 与合并短写法）。
///   只有一档例外：显式 `=false` 的五种拼写（`--links=false`、`--copy-links=false`、`--local-links=false`、
///   `-L=false`、`-l=false`）实测与默认一字不差、不改不变量，被点名豁免放过（`--local-links=false` 也在这一列，
///   见 `config::jobs` 的 `link_flag_explicitly_off`）。
///   配置这一层的白名单只看 **argv token**，看不到进程**环境** —— `RCLONE_COPY_LINKS=1` 这类
///   `RCLONE_<FLAG>` 环境变量实测同样能让 junction 被跟随（`link/a.webp`、`link/deep/c.webp` 落地、
///   exit=0），而闸门一个 token 都没见到。**那一半已经由 spawn 侧收口**：`rcd::build_command` 把继承来的
///   `RCLONE_*` 逐条 `env_remove`，只留 `RCLONE_CONFIG` 与 `RCLONE_CONFIG_PASS`（判据
///   `rcd::rclone_env_is_scrubbed`，它先大写归一再比前缀 —— 小写那条躲得过只看前缀的写法）。
///   所以本地遍历这一层不必再看环境一遍；但也别把这条读成"环境随便设"。
/// - **环**：当前分支上的目录（按 `fs::canonicalize` 后的身份）第二次出现就不再下探。
///   判据是"这条分支上有没有它"，不是"我来过没有"——后者会把不构成环的菱形镜像也一起剪掉。
///   分支栈之外还留着一个全局 `seen`：一个只在不相干分支上见过的目录是镜像，按上面那条实测
///   事实同样不下探。这台机器上造不出"不经过重解析点的环或镜像"（NTFS 上目录只能靠重解析点
///   被两条路径指到，目录符号链接要 SeCreateSymbolicLinkPrivilege，实测 os error 1314），
///   所以这两个守卫没有自己的夹具：它们是第一道判据失效时的封顶，不是日常起作用的那一条，
///   这里如实这么写。（无限分支必然重复某个节点，所以分支栈 + 工作预算是可证的终止。）
/// - **顺序是树决定的，不是文件系统决定的**：每一层的条目按名字排过再下探。
///   不排的话"哪条拼写先被枚举到"就决定期望集合长什么样，对齐预览与它动手前那次重列
///   可能在**没动过的盘**上给出两份不同的删除候选（那一句拒绝说的是「这两个位置上的清单变了」，
///   浏览器那一档说的是「这一层变了」——屏上没有"桶变了"这句话，别照着它去找）。
/// - **工作预算**：`read_dir` 次数或 key 数超过 [`WalkBudget`] 就整次失败，
///   错误里点名是哪条上限、上限是多少、停在哪儿。绝不交回半个集合——半个集合正是这个函数
///   存在的理由要消灭的那个东西（少掉的 key 在下游是删除候选）。
/// - **失败一律往上抛**：`read_dir` 失败（ACL 拒绝、路径过长）会抛出带目录名的 `io::Error`——
///   一个读不了的目录**不会**被当成文件收进集合（实测 `is_dir()` 对 ACL 拒绝的目录仍然返回 true，
///   所以下探它才看得见"读不了"这件事），`canonicalize` 失败也是抛（口径：
///   跳过交出的是不完整的集合，而这份集合在下游是删除候选）。
/// - **`exclude` 按实测的 rclone 语义匹配**，见 [`parse_excludes`] 与 [`rule_matches`]：
///   同一批字符串会喂给两套匹配器（这里，和 rclone 自己的 `--exclude` 启动参数），
///   两边答案不同就等于 `expected` 和桶不同，而差集的两个方向**不等价**：
///   少一条就是删除候选。语法超出我们实现的那一档（`**` / `[...]` / `{...}` / `?` / `\`）
///   一律**报错**，不静默不匹配。
pub fn normalize_expected(
    root: &Path,
    prefix: &str,
    exclude: &[String],
) -> Result<BTreeSet<String>, std::io::Error> {
    normalize_expected_with_budget(root, prefix, exclude, WalkBudget::default())
}

/// [`normalize_expected`] 的工作预算可变的那一份。默认入口用两个 `MAX_WALK_*` 常量，
/// 这个入口是给测试和"操作员想把上限调紧一点"留的。
pub fn normalize_expected_with_budget(
    root: &Path,
    prefix: &str,
    exclude: &[String],
    budget: WalkBudget,
) -> Result<BTreeSet<String>, std::io::Error> {
    // exclude 的模式先解析再走路：语法超出我们这一档的写法必须在**动文件系统之前**就说出来，
    // 而不是走到那一层才静默不匹配（静默不匹配 = 期望集合和桶不等，而这正是本函数要消灭的东西）。
    let rules = parse_excludes(exclude)?;
    let mut out = BTreeSet::new();
    // root 自己先进 seen：指回 root 的 junction 第一次就会被拦掉。
    let root_canon = std::fs::canonicalize(root)
        .map_err(|e| std::io::Error::new(e.kind(),
            format!("{}：这个 src 目录的真实路径解析不出来：{e}", root.display())))?;
    let mut seen: BTreeSet<PathBuf> = BTreeSet::from([root_canon.clone()]);
    // 预算的口径和远端 BFS 的 `max_requests` 一致：**上限之内一次都不许多发**。
    // 所以 root 这一层也是先记账再打开，不是打开完才发现超了。
    let mut opened = 1usize;
    if opened > budget.read_dirs {
        return Err(over_read_dirs_budget(root, opened, budget.read_dirs));
    }
    let root_entries = read_dir_sorted(root)?;
    let mut stack: Vec<Frame> = vec![Frame { canon: root_canon, entries: root_entries, next: 0 }];

    while let Some((path, ftype)) = next_entry(&mut stack) {
        // 分类只看父目录那一次枚举带回来的数据：这里没有"第二次 stat 失败就当成文件"的余地。
        if ftype.is_symlink() {
            continue;
        }
        let rel = path.strip_prefix(root).expect("walked from root")
            .to_string_lossy().replace('\\', "/");
        let is_dir = ftype.is_dir();
        if excluded(&rel, is_dir, &rules) {
            // 目录命中目录规则 = **整棵子树剪掉，一次 read_dir 都不做**（实测 rclone 就是这样：
            // `--exclude node_modules/` 连 `node_modules/deep/z.webp` 都进不了桶）。
            // 剪在 canonicalize / 记账之前：被排除的目录不该占工作预算，也不该进 `seen`——
            // 它本来就不会被推上去，数它一次和数它十次都不该改变结论。
            continue;
        }
        if is_dir {
            // canonicalize 失败也要说得出是哪一层：一棵树里可能有几千个目录，
            // 一句"访问不被允许"没法让操作员去查那一个。
            let canon = std::fs::canonicalize(&path)
                .map_err(|e| std::io::Error::new(e.kind(),
                    format!("{}：这一层的真实路径解析不出来：{e}", path.display())))?;
            // 环：这个目录就在当前分支上。
            if stack.iter().any(|f| f.canon == canon) {
                continue;
            }
            // 镜像：分支之外已经枚举过同一个目录。
            if !seen.insert(canon.clone()) {
                continue;
            }
            opened += 1;
            if opened > budget.read_dirs {
                return Err(over_read_dirs_budget(&path, opened, budget.read_dirs));
            }
            let entries = read_dir_sorted(&path)?;
            stack.push(Frame { canon, entries, next: 0 });
            continue;
        }
        out.insert(format!("{prefix}{rel}"));
        if out.len() > budget.keys {
            return Err(std::io::Error::other(format!(
                "本地扫描停止：期望集合的 key 数超过上限 {}（停在 {rel}）。半个集合在 prune 那边是删除候选，\
                 所以这里不给结论。要么这个 src 真的这么大（那就调 MAX_WALK_KEYS），要么树里有镜像链。",
                budget.keys
            )));
        }
    }
    Ok(out)
}

/// 上限报错要说清三件事：撞的是哪条上限、上限多少、停在哪个目录。
/// 只说"扫描失败"的话，操作员没法判断该调预算还是该去查目录树。
fn over_read_dirs_budget(dir: &Path, opened: usize, limit: usize) -> std::io::Error {
    std::io::Error::other(format!(
        "本地扫描停止：{opened} 层目录已经超过 read_dir 次数上限 {limit}（正要打开的是 {}）。\
         半个集合在 prune 那边是删除候选，所以这里不给结论。要么这个 src 真的这么大\
         （那就调 MAX_WALK_READ_DIRS），要么树里有镜像链。",
        dir.display()
    ))
}

/// 栈上的一个目录层：`entries` 是这一层读到的条目（路径 + 类型），`next` 是游标。
/// 用显式帧而不是递归，是因为分支栈要和帧同生同灭（这一层见底就得把栈顶弹掉）。
struct Frame {
    /// 这一层的目录身份（`fs::canonicalize` 之后）。
    canon: PathBuf,
    entries: Vec<(PathBuf, std::fs::FileType)>,
    next: usize,
}

/// 取出下一个要处理的条目；一层的条目见底就把那一层从分支栈上弹掉。
fn next_entry(stack: &mut Vec<Frame>) -> Option<(PathBuf, std::fs::FileType)> {
    loop {
        let frame = stack.last_mut()?;
        if frame.next >= frame.entries.len() {
            stack.pop();
            continue;
        }
        let next = frame.entries[frame.next].clone();
        frame.next += 1;
        return Some(next);
    }
}

/// 读一层目录：条目路径 + **父目录那次枚举给的类型**，按名字排过再交回来。
///
/// 类型必须在这儿取。`DirEntry::file_type()` 读的是 FindFirstFile 那条记录本身，
/// 而 `path.is_dir()` 要拿这个路径再开一次——那一次开不动（死链接、路径过长）就报 false，
/// 于是"分文件还是目录"这件事会把一个目录悄悄当成文件收进集合（实测死 junction 出过 `p/gone`）。
///
/// 排序是为了让扫描顺序是**这棵树**的函数，不是**文件系统枚举**的函数：实测本机 `read_dir`
/// 给的是不区分大小写的名字序（`["a_dir", "B_dir", ...]`），和字节序（`["B_dir", "a_dir"]`）不一样。
///
/// 目录条目本身读失败也要抛（`flatten()` 会把"少了一条"变成静默的缺 key，而缺 key 在下游是
/// 删除候选），错误里带上是哪一层——操作员要的是一眼能看懂的那一句。
fn read_dir_sorted(dir: &Path) -> Result<Vec<(PathBuf, std::fs::FileType)>, std::io::Error> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(ctx(dir))? {
        let entry = entry.map_err(ctx(dir))?;
        let ftype = entry.file_type().map_err(ctx(dir))?;
        entries.push((entry.path(), ftype));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

/// 把 io 错误包上"是哪一层目录出的事"。`kind` 原样留着（Task 5 想按 kind 说话就别拿掉）。
fn ctx(dir: &Path) -> impl Fn(std::io::Error) -> std::io::Error {
    let dir = dir.to_path_buf();
    move |e| std::io::Error::new(e.kind(), format!("{}：这一层读不下去：{e}", dir.display()))
}

/// 远端大小写敏感、本地 NTFS 不敏感，所以"只差大小写"的一组 key 只可能出现在远端，
/// 通常是一次改名的残留。gate 不据此失败，但要把它显示出来。
pub fn case_conflicts(keys: &BTreeSet<String>) -> Vec<Vec<String>> {
    let mut by_lower: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for k in keys {
        by_lower.entry(k.to_lowercase()).or_default().push(k.clone());
    }
    by_lower.into_values().filter(|g| g.len() > 1).collect()
}

/// 一条 `[defaults] exclude` 规则，按**实测的** rclone v1.75.1 语义解析（逐条证据：
/// `docs/superpowers/evidence/build-notes.md` §Task 4 修复轮（下半））。
/// 同一批字符串会喂给两套匹配器：这里，和 rclone 自己的 `--exclude` 启动参数
/// （spec §3.1.1 rule 1，`config/jobs.rs` 把它们拼进 rcd 的 argv）。两套答案不一样，
/// `expected` 就和桶不一样，而差集的两个方向**不等价**：少一条 = prune 的删除候选。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ExcludeRule {
    /// 去掉首部 `/` 和尾部 `/` 之后的模式（相对 src 根、正斜杠、按 `/` 切段来看）。
    pattern: String,
    /// 带前导 `/`：只从 src 根算起。实测 `--exclude "/b/*"` 一个都不挡，`--exclude "/a/b/*"` 挡两层。
    anchored: bool,
    /// 带尾部 `/`：**目录**规则。实测这是唯一能剪掉整棵子树的写法（`node_modules/`、`a/*/`、`*.db/`）。
    directory: bool,
}

/// 我们实现不了的写法，和 rclone 拿它们干什么（全部实测，v1.75.1，本地到本地）：
///
/// | 写法 | rclone | 这一档 |
/// |---|---|---|
/// | `*` | 匹配任意串，**不跨 `/`** | 实现（`*/old*` 只挡 `a/b/oldx.webp`） |
/// | 不带 `/` 的模式 | 在**任意一层**按名字对齐 | 实现（`old*` 挡 `oldtop.webp` 与 `a/b/oldx.webp`） |
/// | 带 `/` 的模式 | 同样在任意一层对齐，但只吃这一段深度 | 实现（`b/*` 挡 `a/b/keep.db`，不挡 `a/b/deep/deepkeep.webp`） |
/// | 前导 `/` | 锚定 src 根 | 实现 |
/// | 尾部 `/` | 命中目录 → **剪掉整棵子树、不再下探** | 实现 |
/// | 裸目录名（`node_modules`、`a/b`、`data.db`） | **什么都不挡**（review 的前提在这里被推翻） | 一致：不挡 |
/// | 大小写 | **敏感**（`NODE_modules/` 实测一个都不挡） | 一致：按字节比 |
/// | `**` | 跨 `/`（`a/**` 挡掉 `a/` whole 子树；`**/old*` 不挡根上的 `oldtop.webp`） | **不实现，报错** |
/// | `[ab]` | 字符类（`cover[ab].webp` 挡两条） | **不实现，报错** |
/// | `{a,b}` | 或（`{rootfile,top}.webp` 挡两条） | **不实现，报错** |
/// | `?` | 单个非 `/` 字符（`rootfil?.webp` 挡 `rootfile.webp`） | **不实现，报错** |
/// | `\` | 转义（`a\.b.webp` 挡 `a.b.webp`） | **不实现，报错** |
/// | 单独的 `/` | 命中 src 根 → 整棵树都不推 | **不实现，报错** |
/// | `[` / `{` 不配对 | rclone **启动就失败**：`CRITICAL: ... mismatched '[' and ']' in glob "x[1"` | 报错（面板这条更早、更贵：配置不通过） |
///
/// 为什么"报错"而不是"按字面量不匹配"：上表最后那一档里 rclone **真的**会少推对象，我们若按字面量
/// 处理就会把那些对象留在 `expected` 里 → gate 永远报一批传不上去的"缺图"（多挡一次检查的那一侧），
/// 而 `**` 那类还会在桶里留下没人删的孤儿。两种都是"静默给结论"，而本模块的口径是**给不出结论就别给**。
/// 报错不改变配置能写什么（controller 的裁定：不改匹配器之外的东西），只是把"两套匹配器对同一串
/// 给两个答案"这件事变成一次启动失败。能用的替代写法：`dir/`（剪整棵子树）或 `dir/*`（只挡一层）。
fn parse_excludes(patterns: &[String]) -> Result<Vec<ExcludeRule>, std::io::Error> {
    let mut out = Vec::with_capacity(patterns.len());
    for raw in patterns {
        let (anchored, rest) = match raw.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, raw.as_str()),
        };
        let (directory, body) = match rest.strip_suffix('/') {
            Some(body) => (true, body),
            None => (false, rest),
        };
        if let Some(why) = unsupported_syntax(body) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!(
                "exclude 里的「{raw}」用的是 rclone 认、而本面板的匹配器不认的写法：{why}。\
                 这里只实现 `*`（不跨 `/`）这一档，所以这一条模式会让两套匹配器对同一个字符串给出两个答案，\
                 扫描拒绝出结论。能等效的写法：剪掉整棵目录子树用 `目录名/`，只挡一层用 `目录名/*`；\
                 `**`、`[abc]`、`{{a,b}}`、`?`、`\\` 请改写成若干条 `*` 模式。"
            )));
        }
        if body.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!(
                "exclude 里的「{raw}」剥掉首尾的 `/` 以后什么都不剩。\
                 rclone 会把它当成 src 根那一个目录，也就是**整棵树都不推**；\
                 本面板不接受这种让期望集合和桶同时对不上号的写法，扫描拒绝出结论。"
            )));
        }
        out.push(ExcludeRule { pattern: body.to_string(), anchored, directory });
    }
    Ok(out)
}

/// 说清这条模式撞上了哪一档 rclone 语法（`**` / `[` `]` / `{` `}` / `?` / `\`），没有就返回 None。
fn unsupported_syntax(pattern: &str) -> Option<&'static str> {
    if pattern.contains("**") {
        return Some("`**` 会跨 `/`（实测 `a/**` 挡掉 `a/` 整棵子树，`**/old*` 挡掉任意深度 old 开头的文件）");
    }
    if pattern.contains('[') || pattern.contains(']') {
        return Some("`[abc]` 是字符类（实测 `cover[ab].webp` 同时挡掉 covera/coverb）");
    }
    if pattern.contains('{') || pattern.contains('}') {
        return Some("`{a,b}` 是或（实测 `{rootfile,top}.webp` 两条都挡）");
    }
    if pattern.contains('?') {
        return Some("`?` 匹配单个非 `/` 字符（实测 `rootfil?.webp` 挡掉 rootfile.webp）");
    }
    if pattern.contains('\\') {
        return Some("`\\` 是转义符，不是路径分隔符（实测 `a\\.b.webp` 挡掉 a.b.webp）");
    }
    None
}

/// 这条规则命中这个对象吗？`rel` 是**对象自己**相对 src 根的路径（正斜杠、无前导斜杠），
/// 目录也一样传它自己的路径（不带尾部斜杠）——实测 rclone 匹配的就是"对象自己的路径"，
/// 一个模式对上中间某层目录**不会**让那一层的内容被排除（`*/old*` 不挡 `a/oldbox/inner.webp`）。
///
/// 按 `/` 切段以后逐段比：段内用 [`glob_match`]（`*` 不跨 `/`，段里本来就没有 `/`）。
/// 段数必须对齐：我们没有 `**`，所以"任意一层"就是"对齐到末尾那几段"，而锚定就是"从头对齐且等长"。
fn rule_matches(rule: &ExcludeRule, rel: &str) -> bool {
    let pat: Vec<&str> = rule.pattern.split('/').collect();
    let text: Vec<&str> = rel.split('/').collect();
    if pat.len() > text.len() || (rule.anchored && pat.len() != text.len()) {
        return false;
    }
    let start = if rule.anchored { 0 } else { text.len() - pat.len() };
    pat.iter().zip(&text[start..]).all(|(p, t)| glob_match(p, t))
}

/// 文件按**文件规则**比，目录按**目录规则**比（`rule.directory == is_dir`）。
/// 这不是省事的近似，而是实测出来的分工：
/// - 不带尾部 `/` 的规则命中目录时 rclone **不剪子树**（`node_modules`、`a/b`、`data.db` 各实测零排除），
///   所以目录这里必须只认目录规则，否则我们会剪掉 rclone 真会推的那批 key → prune 的删除候选；
/// - 带尾部 `/` 的规则只对目录生效，靠"不下探"覆盖整棵子树，而不是靠逐条比对文件。
fn excluded(rel: &str, is_dir: bool, rules: &[ExcludeRule]) -> bool {
    rules.iter().any(|r| r.directory == is_dir && rule_matches(r, rel))
}

/// **单段**的 `*`-only glob：`*` 匹配任意串，但**不跨 `/`**（实测 rclone 的 `*` 就是 `[^/]*`）。
/// 调用方（[`rule_matches`]）已经按 `/` 切过段，所以这里的 `text` 段内不含 `/`；
/// 这也正是旧实现出错的地方——它把带 `*` 的模式整条喂进来，于是 `*` 吃了 `/`，
/// `*/old*` 把 `a/oldbox/inner.webp` 也挡住了，而 rclone 照样把它推进桶。
/// 逐段算法：按 `*` 切开，首段必须是前缀、末段必须是后缀、中间段按最左出现顺序找——
/// 段内没有 `/`，所以最左贪心就是对的（后面的 `*` 总能吸收多余的部分）。
/// rclone 还认 `**` / `[abc]` / `{a,b}` / `?` / `\`：见 [`parse_excludes`] 的表，那几档我们**不实现**，
/// 在解析阶段就报错，不进这里。
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let last = parts.len() - 1;
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !rest.starts_with(part) { return false; }
            rest = &rest[part.len()..];
        } else if i == last {
            if rest.len() < part.len() || !rest.ends_with(part) { return false; }
            rest = &rest[..rest.len() - part.len()];
        } else {
            match rest.find(part) {
                Some(at) => rest = &rest[at + part.len()..],
                None => return false,
            }
        }
    }
    true
}
