//! 每键一锁的登记表。三件事光看本模块会读错，写在门口：
//! 1. **锁的单位是"一棵树"，不是"一对目录的关系"**（裁定 R90 / spec I3）。键来自 `Scope::key()`，
//!    冲突判定按 `Scope::conflicts`：同一棵树的**上下两层**互相排斥（`r2:#img` 占着时
//!    `r2:#img/2026` 进不来，反过来也一样），而只是长得像的邻居（`r2:#img` 与 `r2:#imgprompt`）
//!    和另一朵云上的同名树（`qiniu:#a`）互不相干。为什么这条必须落在锁本身：浏览器与对齐
//!    **不共享任务**，它们唯一的关系就是"同一棵树的两个层级"，只会比等号的锁等于没有互斥。
//!    两个方向（`A→B` 与 `B→A`）交出的键是**同一组**（Task 7），所以"反向"从来不是这把锁的一种状态，
//!    也不需要"先拿正向、再查反向键"那一步。
//! 2. **`Scope` 只进到登记了 Scope 的那些登记为止**：`try_acquire(key, owner)` 交的是一个不透明
//!    键串，那条登记没有树可比，于是它**只**与键相等的申领冲突（详见 [`conflicts_with`]）。
//!    生产侧的写动作都走 `acquire_scopes`（任务那侧是 `runner/mod.rs` 的 `Runner::acquire`，
//!    浏览器那侧是它自己那七处写动作），`try_acquire` / `acquire_all` 的当下读者只剩测试。
//!    谁把某条生产路径退回那两个键串入口，那一路就同时退掉父子互斥——要保留那一格就交 `Scope`，
//!    不要改这里的 `Option`。
//! 3. **只能有一张共享表。** `clone()` 是安全的（共用同一个 `Arc`，互斥照常成立），
//!    但再造一个 `JobMutex::new()` 就没有任何互斥，而且**没有任何症状**——它不会报错，
//!    只会让两个任务同时跑。所以 Task 10 的 `locks` 命令必须读 managed state 里那个 `Runner`，
//!    绝不自建一个（Task 9 那边是单字段 + 构造一次 + `mutex()` 只读访问）。
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;

use crate::scope::Scope;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Busy {
    pub owner: String,
}

/// 登记表：**键 → 一条登记**。
///
/// 为什么不是 `tokio::sync::Mutex` + `OwnedMutexGuard`（这版计划原来那样写，是被裁定 R31 推翻的）：
/// 那种写法把"表锁"和"任务锁"合成了一把——`try_acquire` 成功后**整张表一直被锁着**，直到任务跑完。
/// 后果不是慢，是三个方向同时坏：
/// 1. 第二个任务 `try_acquire`（哪怕完全不同的键）会**卡在锁上等**，而不是拿到 `Busy` 被拒。
///    本模块的立身之本是"忙就说忙"，等待语义把它反成了"排队"。
/// 2. `holder()` / `snapshot()` 也要同一把锁 → 面板每秒轮询的"谁在跑"直接挂住，
///    传输期间界面停止更新（每秒那两条是 `get_jobs` → `blocking_owner` / `locks`，它们都读这张表；
///    `job_state` 读的是状态表，不走这里）。
/// 3. 一个 panic 的 guard 会把锁留在原地，而 `Drop` 里没法 await。
///
/// 所以表锁用 `std::sync::Mutex`，**只在"查+插"这几行里持有，绝不跨 `.await`**；
/// 任务之间的互斥靠**表里的成员关系**，不靠持锁。`Drop` 是同步的，正好能锁它。
///
/// 一条登记：**名字是给界面看的，token 才是所有权**（裁定 R34）。
/// 名字不能当所有权标识——Task 9 传的 `owner` 是 `job.name`，同一个任务再跑一次就是同一个名字，
/// 于是"按名字删"会允许一个**陈年的 guard**删掉**当下这次运行**的登记（R34 的 ABA，见 `Drop`）。
/// token 每次申领从进程内的单调计数器取，因此"谁删的"永远唯一，且不需要 `unsafe` 也不需要 uuid。
#[derive(Debug, Clone)]
struct Registration {
    owner: String,
    token: u64,
    /// 这条登记代表**哪棵树**（裁定 R90）。`None` = 申领方只交了一个不透明的键字符串
    /// （`try_acquire` 那条路），它手里没有"树"这个概念，于是谁也不能跟它比包含关系。
    /// 判据本体见 [`conflicts_with`]：**只有两边都是 `Some` 时才比 `Scope::conflicts`**。
    scope: Option<Scope>,
}

/// 一条**已登记**的锁与一次**待申领**的锁冲突吗（裁定 R90 / spec I3 的落点）。
///
/// **只有两边都是 Scope 时才比包含关系**，其余一律回到键相等。为什么这一条不能省掉 `Option`：
/// 包含关系要两个 `Scope` 值才算得出来，而登记表里躺着的可能只是一个不透明串
/// （`tests/job_mutex.rs` 里手写的 `"a=>b"`，以及 `runner_local`/`mirror_runner` 占键时交的
/// `Job::scope_keys()`——都是键，不是树）。
/// 那一侧没有树，硬要比就只能拆键串——`Scope::key()` 用的分隔符 `#` **不是**保留字符
/// （对象名里 `#` 合法，见 `scope.rs` 里 `key()` 的注释），拆出来的那两截不保证是一棵树，
/// 拿它去比包含就是拿一个猜测当判据。所以宁可只比等号：
/// - 键相等 → 一定冲突（`r2:#img` 占着，同键谁都进不来）；
/// - 有一边不是 Scope → **弱一级**：`try_acquire("r2:#img")` 的持有者挡不住
///   `acquire_scopes(r2:#img/2026)`，`Scope` 的持有者也挡不住 `try_acquire("r2:#img/2026")`。
///   两边都是 Scope 时父子互相排斥，这才是 I3 要的那一格（浏览器与对齐唯一的关系就是
///   "同一棵树的上下两层"，它们不共享任务，方向串与等号都串不到彼此）。
///
/// 这一档撑着哪些证人、"弱一级"在 Task 9/10 的浏览器接上写动作之前咬不咬得到真实写动作，逐条写在
/// 模块头第 2 条（那里是读者名单，这里不重抄一遍以免两处漂移）。证人：
/// `a_string_keyed_holder_is_weaker_than_a_scope_holder`（同一条判据的三个面，一面一句；
/// `Option` 那一档被谁拆掉会 panic，两条键串申领方那两面各自红在哪也记在那条的注释里）。
///
/// **这一句判据有三处读者，三处必须是同一把尺子**：拒绝侧的 `acquire_one`、一批之内的折叠
/// [`JobMutex::acquire_scopes`]（裁定 R91）、读侧的 [`JobMutex::blocking_owner`]（裁定 R92）。
/// 各写各的就会长出两套规矩——界面按键等号说"这行没锁"、锁按包含关系把人拒在门外，
/// 操作员看到的是一亮点下去就拒绝的按钮，那比读数难看更坏。
fn conflicts_with(held: Option<&Scope>, held_key: &str, req: Option<&Scope>, req_key: &str) -> bool {
    match (held, req) {
        (Some(h), Some(r)) => h.conflicts(r),
        _ => held_key == req_key,
    }
}

/// 进程内单调申领号。只用 `SeqCst` 取号，不参与任何同步语义。
static NEXT_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone)]
pub struct JobMutex {
    inner: Arc<Mutex<HashMap<String, Registration>>>,
}

/// 持有者。`Drop` 就是释放——runner 靠这个保证 panic 路径也不会把锁漏在原地。
///
/// **它管的是"这条持锁的 task 活着"，不是"那个 rclone job 活着"（裁定 R33，评审发现的 R31 副作用）。**
/// 不变量要说明白：**登记恰好在 guard 死时释放，所以 guard 必须活得比 rclone 的 job 长。**
/// 被否决的那版（guard 里存 `OwnedMutexGuard`）在这条上是**失败即关闭**的——它会漏锁，于是谁都跑不了；
/// 现在这版是**失败即敞开**：任何让持锁 future 结束的东西都顺手把键放了，包括计划里
/// `transfer` 连续读不到 `job/status` 之后返回 `TaskState::Unknown` 那条路——那时 rclone 那边
/// 的任务还在跑、还在删。所以 Task 9 **不许**在那条路上把 guard 丢掉，见它的 `abandoned` 字段。
///
/// `Debug` 不是装饰：Step 1 的测试对 `Result<LockGuard, Busy>` 用 `.unwrap_err()`，
/// 而那要求**成功分支**也 `Debug`（`unwrap_err` 的约束）。少了这一条 brief 里的代码编译不过
/// （`E0277`），执行期由实现者发现并补上。
#[must_use = "guard 就是那把锁：不当场接住它等于立刻释放这个键"]
#[derive(Debug)]
pub struct LockGuard {
    key: String,
    // 持有者名。R34 之后所有权比对改用 token，但这个名字**不是**死代码：`Drop` 的绊线分类
    // 走到绊线那一档就经 `drop_note` 无条件读它（只有**打印**被 cfg 掉，分类本身在所有构建里都跑；
    // `pub fn owner()` 的当下读者是证人测试，runner 的日志用的是申领时自己传进去的那个名字。
    // 这里挂过 `#[cfg_attr(not(debug_assertions), allow(dead_code))]` 来压
    // `cargo build --release` 的 "field is never read"（rustc 不把 derive 生成的读算作字段使用，
    // 绊线又被 cfg 掉了）。R39 的处置是**给它真读者**而不是把警告压掉；这条也照该纪律办：
    // release 侧的 dead_code 纪律保持完整，**不许**把 allow 加回来。
    owner: String,
    token: u64,
    inner: Arc<Mutex<HashMap<String, Registration>>>,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // 毒锁恢复：表锁的临界区只有 map 的查/插/删，不跑任何回调，中毒本身几乎不可能。
        // 但一旦发生，"面板从此拒绝所有任务"绝不是比"丢掉一条登记"更好的结局。
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        // 按**所有权**删：先看这个键上登记的是不是自己，不是就一动不动。
        // （这里原来写的是"说一声但不改变行为"，可那是注释在说谎——`remove` 是无条件执行的，
        //  真出现别人的登记挂在同一个键上时，第一个落地的 guard 会把**别人**的登记删掉，
        //  于是两个任务同时握着同一对目录：正是本项目的起点事故。R33 改成所有权门控。）
        // 比对的是 **token**，不是名字（裁定 R34）：名字在"同一个任务再跑一次"时是重复的，
        // 按名字认会让一条陈年 guard 删掉当下这次运行的登记——键被放开、rclone 还在删，
        // 反向任务此刻能进来，正是起点事故。token 每次申领唯一，这条路彻底断掉。
        // 分类判定（`drop_note`）只做**话术**，删除动作的门槛仍是下面那个 `reg.token == self.token`
        // 卫兵，一个字都没改；把名字当成删除依据就是 R34 否掉的那条 ABA。
        let note = match map.get(&self.key) {
            Some(reg) if reg.token == self.token => {
                map.remove(&self.key);
                None
            }
            Some(reg) => {
                // 绊线：本 guard 还活着，可键上登记的不是自己，**不动那条登记**。
                // R39-2 把"不同名"叫成结构性 bug 是错的（已否），但那次修正自己**把结论
                // 写死成了"每一档都不是 bug"**——那是**枚举出来的**可达性：今天 `insert` 在 `Busy`
                // 返回之后不可达，登记离开表只有 owning guard 的 `Drop` 和 `force_release` 两条路，
                // 所以"这一档全是恢复"在当前形状下成立。可坏的方向也是错的：将来谁加了第三条移除
                // 路径（比如按键删除的清理），绊线唯一的输出就是一句斩钉截铁的"非 bug"，
                // 正是那句被否掉的"狼来了"的镜像。
                // R43：判据换成免费的 **token 顺序**（登记从不覆盖 + 计数器单调 ⇒
                // `force_release` 之后重新申领的 token 一定**大于**陈年 guard 的 token）。
                // 更大 = 恢复（保留原来按名字分的两句），**更小 = 现有形状构造不出来**，
                // 只有真缺陷才能造出来 ⇒ 那是本模块唯一诚实的结构性 bug 信号，必须留着。
                // 判定抽成 `drop_note` 是为了能被**见证**（mutex.rs 的 `mod tests`）：
                // 消息只去 stderr，线上没有断言，把两个分支折成一行打印原先无人能察觉；
                // 不捕获 stderr 做断言（需要 panic hook 或子进程，代价大于收益，评审同意的取舍）。
                drop_note(&reg.owner, reg.token, &self.owner, self.token)
                    .map(|note| (reg.owner.clone(), reg.token, note))
            }
            None => None,
        };
        // 打印挪到表锁**之外**：先把要印的串拷进局部值，放下 `MutexGuard`，再 `eprintln!`。
        // 理由是机器特定且真实的：本机 console `QuickEdit=1`，持表锁时一次被挡住的 stderr 写
        // 会把整张登记表冻住——连 `blocking_owner()` / `snapshot()`（面板每秒轮询的那两条）也一起挂。
        drop(map);
        // release 构建里 `eprintln!` 被 cfg 掉、没人再读 `note`，显式喂它一次：
        // 否则 `cargo check --all-targets --release` 报 unused variable，而 CI 跑的是 debug 那一档的
        // `clippy --all-targets`、也**没有** `deny(warnings)` —— release 那条警告没人会看见，所以这里
        // 用一次消费来自证，而不是 `allow(dead_code)`。
        let _ = &note;
        // 只在 debug 构建里说话：release 是 GUI 子系统程序，stderr 没有落点；
        // 更要紧的是 `eprintln!` 写失败会 panic，而在 `Drop` 里 panic（还可能叠在另一段
        // 展开之上）会把一次可恢复的展开变成进程中止——这条路径就在删除动作旁边。
        #[cfg(debug_assertions)]
        if let Some((registered_owner, registered_token, note)) = note {
            eprintln!(
                "mutex: 释放键 {:?} 时登记表里是 {:?}(#{})，本 guard 是 {:?}(#{})——{}",
                self.key, registered_owner, registered_token, self.owner, self.token, note
            );
        }
    }
}

/// 把"键上登记的不是本 guard"分类成绊线的一句话。**只做话术，不碰任何状态**。
///
/// 三档，第一判据是 **token 顺序**（裁定 R43），名字只在恢复档里用来分两句：
/// - `registered_token == self_token` → `None`：本 guard 就是那条登记（owning 一侧落地），无话可说。
/// - `registered_token > self_token` → **正常恢复**。登记离开登记表只有两条路：owning guard 自己的
///   `Drop`（那条命中上面一档）与 `force_release`。登记从不覆盖、计数器单调，所以键上现在挂的这条
///   只会是本 guard 之后**重新申领**的，token 必然更大。两句措辞按名字分：
///   - **同名** = 键被放掉后**同一个任务**又跑了一次，陈年 guard 只是落地晚了；
///   - **不同名** = 键被放掉后被**另一个任务**接手，本 guard 是那条旧的。
/// - `registered_token < self_token` → **结构性 bug**。这条在本模块的形状下**构造不出来**：
///   更小的号只可能在更早的时刻发出，而它此刻正挂在键上、又不是自己，就只能有人把晚那条删掉后
///   把旧那条**写回去**了，或者新增了一条未预期的移除/覆盖路径。这是绊线**唯一**诚实的缺陷信号，
///   所以它必须存在；前两档那种"非 bug"的断言只许出现在真恢复的形状上。
///
/// 名字比对只用于**分类**，绝不用于删除（删除门槛是 `Drop` 里的 token 卫兵，R34 的 ABA）。
/// `Drop` 只在非自己那档调用它，所以那条 `None` 早退是给判定留的完整形状，也是证人 3 的靶子。
fn drop_note(registered_owner: &str, registered_token: u64, self_owner: &str, self_token: u64) -> Option<&'static str> {
    if registered_token == self_token {
        return None;
    }
    if registered_token < self_token {
        return Some("结构性 bug：键上登记的 token 比本 guard 还旧，单调计数器且登记从不覆盖的形状下这条路构造不出来（要么登记被删后又写回旧值，要么新增了未预期的移除路径）");
    }
    if registered_owner == self_owner {
        Some("键已被 force_release、随后同名再跑（本任务又开了一次，陈年 guard 晚落地）：正常恢复，非 bug，未改写登记")
    } else {
        Some("键已被 force_release、随后被另一个任务接手：正常恢复，非 bug，未改写登记")
    }
}

impl LockGuard {
    /// 正被持有的键。读它的**只有证人测试**：runner 需要这对目录的键时自己算
    /// （`Job::write_scopes()` 交的是 Scope，键由 `Scope::key()` 出），日志里也没有从 guard 取键的那一行。
    /// 旧注释声称"runner 把它写进日志，运维才知道正在跑的那条锁的是哪对目录"——那不是事实。
    pub fn key(&self) -> &str { &self.key }

    /// 持有者名字。照实说：**当下的读者只有证人测试**（`job_mutex.rs` 拿它和 `holder()`、
    /// `Busy.owner` 对，钉"三个出口报的是同一个串"）。界面那句「正在跑：谁」既不是这个方法也不是
    /// `holder()`，是读侧那条查询 `Runner::blocking_owner` 的读数（填进 `JobView.lockedBy`，裁定 R92）
    /// ——别把这里读成"界面在用它"。
    /// 名字只是**标签**，不是所有权凭证：删除判定只认 token（见 `Drop`）。
    pub fn owner(&self) -> &str { &self.owner }
}

/// 一批键的持有证明。**语义与 `LockGuard` 完全一致**：登记恰好在它死时释放，
/// 所以它也**必须活得比 rclone 的 job 长**（子项目 1 的 R33 那条不变量在这里同样成立，
/// 而且更严——`purge` 异步跑的时候如果 guard 先死，浏览器就能对同一棵树再发起一次写）。
#[must_use = "这批锁就是这些键：不当场接住它等于立刻释放它们"]
#[derive(Debug)]
pub struct MultiGuard {
    guards: Vec<LockGuard>,
}

impl MultiGuard {
    /// 排序后的键。当下的读者只有证人测试（`tests/job_mutex.rs` 用它核对"拿到了什么"）；
    /// Task 7 的 runner 日志是**计划中**的读者（那行还没写）。
    pub fn keys(&self) -> Vec<String> {
        self.guards.iter().map(|g| g.key().to_string()).collect()
    }
}

impl JobMutex {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// 仍然是 `async fn`：Task 9 在整个生命周期里 await 它，接口不该因为内部不再真异步而变。
    /// 语义是**尝试**，不是等待——拿不到就立刻 `Busy`。
    ///
    /// 这条路径登记的键是**不透明串**（`scope: None`），所以它只与"键相等"的申领互斥，
    /// 父子那一格比不出来（裁定 R90，判据与后果见 [`conflicts_with`]）。
    /// 生产写动作请走 [`JobMutex::acquire_scopes`]。
    pub async fn try_acquire(&self, key: &str, owner: &str) -> Result<LockGuard, Busy> {
        self.acquire_one(key, None, owner)
    }

    /// 查一次冲突、插一条登记。**表锁只在"扫+插"这几行里持有，绝不跨 `.await`**（R31 那条纪律一字未改）。
    ///
    /// 冲突判定从"查这一个键"改成"扫一遍表"，`try_acquire` 的**结果**没有变：表里键唯一，
    /// 而它那个请求 `scope = None` 只会按等号命中（见 [`conflicts_with`]），能命中的还是原来
    /// `map.get(key)` 那一条。多花的是一次 O(持有数) 的扫描，持有数是个位数（同时在跑的传输条数）。
    ///
    /// 多条登记同时冲突时报出的是 **token 最小的那条**（最早登记的）。这不是修饰：
    /// `HashMap` 的迭代顺序是随机的，不选一个确定的判据，`Busy.owner` 就每次跑换一个名字说，
    /// 而"排序让失败原因可预测"那句（见 `acquire_all` / `acquire_scopes` 头上）当场作废。
    fn acquire_one(&self, key: &str, scope: Option<Scope>, owner: &str) -> Result<LockGuard, Busy> {
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let clash: Option<String> = map.iter()
            .filter(|(held_key, reg)| conflicts_with(reg.scope.as_ref(), held_key, scope.as_ref(), key))
            // `min_by_key`：多条同时冲突时报**最早**登记的那条，理由见头上那段。
            .min_by_key(|(_, reg)| reg.token)
            .map(|(_, reg)| reg.owner.clone());
        if let Some(busy_owner) = clash {
            return Err(Busy { owner: busy_owner });
        }
        let token = NEXT_TOKEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        map.insert(
            key.to_string(),
            Registration { owner: owner.to_string(), token, scope },
        );
        drop(map); // 表锁到此为止，绝不跟着任务活一辈子
        Ok(LockGuard {
            key: key.to_string(),
            owner: owner.to_string(),
            token,
            inner: Arc::clone(&self.inner),
        })
    }

    /// 一次拿齐多把键，**全有或全无**。
    ///
    /// 先排序再去重（`dedup` 只对有序序列有效，所以顺序不能反过来）：两侧在同一棵树上时
    /// 一次请求会带两遍同一把键，不去重就是自己撞自己。
    ///
    /// 排序**不是**防死锁的兜底——`try_acquire` 从不等待，拿不到就立刻回 `Busy`，
    /// "持有并等待"这个条件根本不成立。排序是为了让失败原因稳定可预测：`Busy` 只带 `owner`
    /// 不带键名，所以"可预测"说的是**报出谁的持有**，不是报出哪个键。
    pub async fn acquire_all(&self, keys: &[String], owner: &str) -> Result<MultiGuard, Busy> {
        let mut ordered: Vec<&String> = keys.iter().collect();
        ordered.sort();
        ordered.dedup();
        let mut got: Vec<LockGuard> = Vec::new();
        for k in ordered {
            match self.try_acquire(k, owner).await {
                Ok(g) => got.push(g),
                // 已经拿到的那些在这里随 `got` 一起 drop —— 部分持有不留残骸。
                Err(busy) => return Err(busy),
            }
        }
        Ok(MultiGuard { guards: got })
    }

    /// 一次拿齐多棵树的锁，**全有或全无**（裁定 R90）。
    ///
    /// 它是 `acquire_all` 的 Scope 版，差别只在**冲突判定**：`acquire_all` 收 `&[String]`，
    /// 手里没有任何树的信息，只能比等号；于是"对齐占着 `decklocal:#img`"与"浏览器要写
    /// `decklocal:#img/2026`"这两次写**互不相干**——而它们唯一的关系就是同一棵树的上下两层，
    /// I3 整条讲的就是这一格。等号互斥在这种形状里等于没有互斥，所以锁必须认识 Scope。
    ///
    /// Task 4 那三条纪律**一条都没搬走**，只是把比较对象换成键：
    /// 1. **全有或全无**——第一次冲突就返回，已经拿到的那些随 `got` 一起 drop，不留残骸；
    /// 2. **先排序再折叠**（`dedup` 只对有序序列有效，顺序反过来就折不掉）——两侧落在同一棵树上时
    ///    一次请求会带两遍同一把键，不折就是自己撞自己；折叠现在管的不只等号那一格，见下一段；
    /// 3. **排序不是为了防死锁**——这条路径与 `try_acquire` 一样从不等待，拿不到立刻 `Busy`，
    ///    "持有并等待"这个条件根本不成立。排序是为了让**失败原因可预测**：`Busy` 只带 `owner`
    ///    不带键名，所以"可预测"说的是报出谁的持有（多条同时冲突时由 `acquire_one` 取最早那条），
    ///    不是报出哪个键。这句在换进 Scope 之后仍然为真。
    ///
    /// **一批里同一棵树只登记极大那一层**（裁定 R91）：`img` 与 `img/backup` 同批出现时子层被折掉，
    /// 表里只留父层那一条。这一步**取代**了原来那道只按精确键去重的 `dedup_by`——等号是包含关系的
    /// 退化情形（一棵树是自己的祖先），一趟排序后的折叠把"重复的键"与"父子两层"两件事一起办完，
    /// 不必再叠第二趟。这和 `acquire_all` 用 `dedup` 守的是同一条纪律（"不去重就是自己撞自己"），
    /// 这里只是把比较从等号扩到包含。为什么不在这条路上放行父子、也不让它撞出 `Busy`：
    /// - **撞在一起的形状是锁在说谎**：父层先进表，子层随即在表里看见它自己 ⇒
    ///   `Busy { owner: 它自己 }`，而且**永久**如此——那句话报的"正在跑的人"就是来申领的这条任务，
    ///   界面上「正在跑：它自己」与它自己的行是同一行，除了 `force_release` 没有任何出路，
    ///   重启之前它一次也跑不了。一把锁拒绝一种形状的方式是说谎，这比多拦或少拦都坏。
    /// - **折叠不减弱任何一格**：同一根之下"互为祖先"是一条链（同为 `img/backup/2026` 祖先的两个
    ///   前缀必然互为祖先），所以任何会被子层挡住的申领也一定被父层挡住。要写 `img/backup` 的浏览器
    ///   purge、要写 `img` 整棵的对齐、要写 `img` 的**反向任务**、更深一层的复制——撞的都是同一条
    ///   登记，`Busy` 报的都是同一个人；反面半边没松：邻居（`imgprompt`）与另一朵云照常进得来。
    /// - **配置层不加拒绝**（"拒而不猜"那条路子这里用不上）：会写的任务里只有 `kind = copy` 交得出
    ///   这种集合——`sync` 被"恰好一侧是 remote"那条校验挡住（两侧根必属不同类，成不了父子）；
    ///   `gate` 与盘点虽然走的是同一把锁（`run_gate`/`run_inventory` 也调 `acquire`），但它们只列不写，
    ///   父子同批时折到祖先只是少登记一条，改不出任何后果；`prune` 这个 kind 已经不存在（裁定 R58）。
    ///   再说删除：删除只可能来自 `sync`，而它到不了这个形状；`[defaults] flags` 的白名单
    ///   `ALLOWED_FLAGS` 里没有任何删除类 flag（不在名单上的一律拒），copy 报出删除还会被判成违约
    ///   （`runner::transfer_invariant_violation`）⇒ 父→子的复制只会**增加**文件，最坏是往自己肚子里
    ///   多写一份，不是"删掉别人的东西"那一档事故。为一个只会增加文件的形状在 parse 侧发明一条拒绝，
    ///   代价是操作员读一句他验证不了的"不许"，收益是零。
    /// - 这个形状会从真实里长出来：Task 9/10 的浏览器交的是**用户在树上点出来的两个位置**
    ///   （"把这个目录复制到它自己的某个子目录"就是一次父子请求），那边只会走 `acquire_scopes`。
    ///   证人：`a_batch_with_both_layers_of_one_tree_registers_the_ancestor_only`。
    pub async fn acquire_scopes(&self, scopes: &[Scope], owner: &str) -> Result<MultiGuard, Busy> {
        // 排序与折叠的比较对象是 `Scope::key()`，不是 `Scope` 自己：登记表是以键为索引的，
        // 而 `Scope` 的 `Ord`（派生顺序是 `root` 再 `prefix`）与 `root#prefix` 的字典序
        // 在 `#` 这个字符上不等价（`#` 排在 digits/letters 之前），照 `Scope` 排就会让
        // "登记的顺序"与"键的顺序"两说一套。
        let mut ordered: Vec<(String, &Scope)> = scopes.iter().map(|s| (s.key(), s)).collect();
        ordered.sort_by(|a, b| a.0.cmp(&b.0));
        // 一趟折叠：留下极大那些层，折掉"已经被留下的某条包住"的那些（重复的那一份是同一种形状的
        // 退化情形）。判据用的是 [`conflicts_with`] 本身，与拒绝侧、读侧同一把尺子，不在这里长第二套。
        // 排序让这一步只需要问"已留下的有没有包住本条"：`is_ancestor` 成立时祖先的键正是后代键的
        // **字符串前缀**（同根，`p` 与 `p/…`），前缀在字典序里必然在前 ⇒ 本条是已留那条的祖先
        // 这种情况构造不出来，所以对称的 `conflicts_with` 在这里够用，不必判方向。
        // 判据里那句 `kept.0 == entry.0`（键串相等）留着是为了**不改变形状**：`Scope::key()` 不保证一对一
        // （条件与后果写在 `scope.rs` 的 `key()` 头上），真撞键时这张表以键为索引、同键只能落一条登记，
        // 折成一条与 R90 之前那道精确键去重**同一种结果**；不折的话会留下两个 guard 共用同一把键，
        // 第二个 `Drop` 因 token 不匹配踩到"键已被 force_release"那句读数上。撞键会让另一棵树没人挡
        // （误放），那是 R90 起就在的一格、不是这一趟折叠带来的，也不在这里关。
        let mut maximal: Vec<(String, &Scope)> = Vec::with_capacity(ordered.len());
        for entry in ordered {
            let nested = maximal.iter()
                .any(|kept| kept.0 == entry.0 || conflicts_with(Some(kept.1), &kept.0, Some(entry.1), &entry.0));
            if nested {
                continue; // 同一棵树的下层（或重复的那一份）：折掉，登记只留极大那一层
            }
            maximal.push(entry);
        }
        let mut got: Vec<LockGuard> = Vec::new();
        for (key, scope) in maximal {
            match self.acquire_one(&key, Some(scope.clone()), owner) {
                Ok(g) => got.push(g),
                // 已经拿到的那些在这里随 `got` 一起 drop —— 部分持有不留残骸。
                Err(busy) => return Err(busy),
            }
        }
        Ok(MultiGuard { guards: got })
    }

    /// 这批位置**正被谁挡着**——读侧的查询（裁定 R92）。没有就 `None`。
    ///
    /// 它存在的理由是"读侧与拒绝侧不许有两把尺子"：`acquire_scopes` 按 `Scope` 的包含关系拒绝，
    /// 而界面那句「正在跑：谁」原来是拿 `Job::scope_keys()` 逐条去登记表里找**键等号**
    /// （`commands.rs::get_jobs` 那句 `find_map(|key| locks.get(key))`）。于是"占着 `r2:#img` 的对齐"
    /// 与"要写 `r2:#img/2026` 的那一行"这一格里，按钮是亮的、点下去拿到 `Busy`：显示说没锁而动作被拒，
    /// 操作员只能拿一次失败去发现这件事。读侧的过宽（把邻居读成占着）与过窄一样是缺陷，所以两侧
    /// 共用判据不是为了省事，是因为只有一个答案才是可核对的。
    ///
    /// 与拒绝侧同形的三处，逐条对齐 [`acquire_one`]：
    /// - **判据**：[`conflicts_with`]，键串登记（`scope: None`）仍然只按等号命中——那面不对称
    ///   在读写两侧是同一个形状（`blocking_owner` 的请求侧永远带树，因为它是从 `Scope` 进来的）；
    /// - **多条同时命中时报谁**：token 最小（最早登记）那条，`HashMap` 的迭代顺序是随机的，
    ///   不选一个确定判据的话界面那行的名字每次轮询换一个；
    /// - **一批的语义**：只要有一棵被挡着就算被挡，与 `acquire_scopes` 的全有或全无一致。
    ///
    /// 不是 `async`：临界区只有这张表的扫描，与 `acquire_one` 同样绝不跨 `.await`（R31 那条纪律）。
    /// 证人：`the_read_side_names_the_holder_on_both_layers_of_a_tree`（父子两个方向 + 邻居/另一朵云
    /// 的反面半边）、`the_read_side_reports_the_earliest_registration_when_two_clash`（那条确定性选择），
    /// 生产形状那一格在 `tests/runner_local.rs` 的 `a_held_tree_blocks_the_tree_inside_it_but_not_a_neighbour`。
    pub fn blocking_owner(&self, scopes: &[Scope]) -> Option<String> {
        let requested: Vec<(String, &Scope)> = scopes.iter().map(|s| (s.key(), s)).collect();
        let map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        map.iter()
            .filter(|(held_key, reg)| requested.iter()
                .any(|(req_key, req_scope)|
                    conflicts_with(reg.scope.as_ref(), held_key, Some(req_scope), req_key)))
            // 与 `acquire_one` 同一个 `min_by_key`：报最早登记的那条，理由见那边头上那段。
            .min_by_key(|(_, reg)| reg.token)
            .map(|(_, reg)| reg.owner.clone())
    }

    /// 返回**持有者名字**：界面与日志只关心"谁在跑"，token 是内部所有权凭证，不外泄。
    pub async fn holder(&self, key: &str) -> Option<String> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
            .get(key).map(|r| r.owner.clone())
    }

    /// 人工确认"那条 job 真的已经不在了"之后的解锁口，返回被清掉的持有者名。
    /// 存在的理由是：Task 9 在状态未知时会把 guard 存起来长期占住这个键，那时**唯一的**恢复手段
    /// 若是 `std::mem::forget(guard)`，就会留下一个进程内永远洗不掉的幽灵持有者
    /// （`snapshot()` 每秒都报一个人在跑，直到重启）。给一个显式、可审计、会写日志的出口，
    /// 比让后来的人去用 forget 兜好。子项目 1 里**不接界面按钮**，先作为 runner 内部与测试的口子。
    /// 只清"**登记的就是这个名字**"的那条。传错名字 = 不动，返回 `None`。
    /// 为什么要 `expected_owner`：`JobMutex` 是 `Clone`、被到处递，只有键名的话
    /// 任何持有它的人都能一条 await 放开**当下正在跑**的任务的登记（起点事故的形状）。
    /// 加上名字至少要求调用方说出"我以为我放的是谁"，那既能写进日志，也能被测试钉住。
    pub async fn force_release(&self, key: &str, expected_owner: &str) -> Option<String> {
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match map.get(key) {
            Some(reg) if reg.owner == expected_owner => map.remove(key).map(|r| r.owner),
            _ => None,
        }
    }

    pub async fn snapshot(&self) -> BTreeMap<String, String> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
            .iter().map(|(k, v)| (k.clone(), v.owner.clone())).collect()
    }
}

impl Default for JobMutex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::drop_note;

    /// 证人 1：**force_release 之后同名再跑**必须被分类成正常恢复。
    /// 实参是"大 token 在前"（登记表里 4 > 本 guard 3），这正是 R43 的恢复档判据：`force_release`
    /// 之后重新申领必然抽到更大的号。
    /// 破坏一次：把 `drop_note` 里 `registered_owner == self_owner` 的分支翻掉（或把同名话术
    /// 换成接手话术）→ 这条红。断言的是**这一档专属**的措辞，相邻变异不算证住。
    #[test]
    fn same_name_after_release_is_normal_recovery_not_a_bug() {
        let note = drop_note("R2 图床上传", 4, "R2 图床上传", 3)
            .expect("同名再跑是绊线档，必须有分类行");
        assert!(note.contains("同名再跑"), "实际话术：{note}");
        assert!(note.contains("正常恢复"), "实际话术：{note}");
        assert!(!note.contains("结构性"), "正常恢复不许指控成结构性缺陷：{note}");
    }

    /// 证人 2：**force_release 之后被另一个任务接手**同样是正常恢复——R39-2 旧话术把它叫成
    /// "结构性 bug（不该发生）"是错的：登记离开表的唯一出口是 `force_release`，而本模块自己的
    /// 测试 `a_stale_guard_never_frees_a_newer_owner_on_the_same_key` 就走在这条路上（也是接手）。
    /// 同样是大 token 在前（2 > 1）⇒ 落在恢复档。
    /// 破坏一次：把两个分支折成同一行打印（丢掉接手措辞）、或把不同名话术改回 bug 指控 → 这条红。
    #[test]
    fn different_name_after_release_is_also_normal_recovery() {
        let note = drop_note("job-2", 2, "job-1", 1)
            .expect("不同名接手是绊线档，必须有分类行");
        assert!(note.contains("另一个任务接手"), "实际话术：{note}");
        assert!(note.contains("正常恢复"), "实际话术：{note}");
        assert!(!note.contains("结构性"), "不同名接手也是正常恢复，不许指控：{note}");
    }

    /// 证人 3：**本 guard 就是那条登记**（token 相同）⇒ 无话可说，返回 `None`。
    /// 破坏一次：删掉 `registered_token == self_token` 的提前返回 → 这条红。
    /// （`Drop` 只在非自己那档调用它，所以这条早退是判定域的完整形状，不是死分支。）
    #[test]
    fn the_owning_guard_gets_no_note() {
        assert_eq!(drop_note("job-1", 1, "job-1", 1), None);
        // token 相同时名字不同也不该有话——owning 一侧落地就是无话，分类轮不到名字。
        assert_eq!(drop_note("别的名字", 1, "job-1", 1), None);
    }

    /// 证人 4（R43 新增，守第三档）：**登记的 token 比本 guard 还旧** ⇒ 绊线唯一诚实的
    /// 结构性 bug 信号。上一版两档话术都断言"键已被 force_release、非 bug"，而这个形状在
    /// 单调计数器 + 从不覆盖下**构造不出来**（重新申领只会抽到更大的号），所以让它沉默 = 让
    /// 将来新增的第三条移除路径永远只汇报"非 bug"。
    /// 前两名证人的实参都是大 token 在前，因此自动落在恢复档；这条把两侧换过来。
    /// 破坏一次：把 `registered_token < self_token` 的方向翻掉（或删掉这一档）→ 这条红
    /// （话术会退成恢复档那句"非 bug"）。
    #[test]
    fn an_older_registration_than_the_guard_is_a_structural_bug() {
        let same_name = drop_note("job-1", 1, "job-1", 9)
            .expect("登记不是自己就必须有话，这一档是缺陷信号");
        assert!(same_name.contains("结构性"), "键上挂着更旧的登记却被放过了：{same_name}");
        assert!(!same_name.contains("非 bug"), "缺陷信号不许同时说'非 bug'：{same_name}");

        // 名字不参与这一档的判定：换个名字也还是结构性 bug，不是接手。
        let other_name = drop_note("job-2", 1, "job-1", 9)
            .expect("同上，这一档与名字无关");
        assert!(other_name.contains("结构性"), "不同名不许把这一档洗成正常接手：{other_name}");
    }
}
