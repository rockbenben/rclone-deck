//! "人已经看过这份清单"这件事的登记处（从 runner 抽出，裁定 R71）。
//!
//! 为什么抽出来：对齐与浏览器要共用**同一份**机制——单次消费的凭证、动手前重列、逐条比对。
//! 留在 runner 里且按 job id 索引，浏览器就只能另写一份确认逻辑，那正是 spec I2 禁止的形状。
//! 键换成 `Scope` 之后，"往 imgprompt 里写了一个文件"能作废掉整桶那份预览，
//! 而方向串那套做不到（它只认同一对任务）。
use std::collections::{BTreeMap, BTreeSet};

use tokio::sync::Mutex;

use crate::scope::Scope;

/// 这份凭证**授权做什么**。放在凭证里而不是放在参数里，是因为界面上只有一个"动手"入口（R78）：
/// token 是唯一能说清"刚才看的是哪件事"的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewIntent {
    Mirror { job_id: String },
    /// "删掉这一层看到的 N 个"（R75）：`prefix` 用于动手前重列比对。
    DeleteVisible { fs: String, prefix: String },
    /// 删除明确点名的那一个/那几个对象。
    DeleteNames { fs: String, names: Vec<String> },
    /// 清空整个前缀（只在排除规则数为 0 时发得出来，R74）。
    Purge { fs: String, prefix: String },
    /// 删一个**空**目录。清单上只有一个名字：那个目录本身。
    /// 空不空在发凭证之前判过一遍，动手之前再判一遍（`runner::browser::run_rmdir`）。
    Rmdir { fs: String, remote: String },
    /// 改名 / 移动（源与目标各是 `(fs, remote)`）。
    Move { src: (String, String), dst: (String, String) },
}

pub struct StoredPreview {
    pub intent: PreviewIntent,
    /// 确认时那份清单（相对各自前缀的对象名）。动手前的比对用它。
    ///
    /// 这里不是"没人核对的第二副本"——它的读者就是动手前那一步逐条比对：把重列出来的删除候选
    /// 和这份存起来的对一遍，不一致就放弃实删（spec §4.1 第 3 步、§6 那一行）。
    /// 存的是**内容**而不是"当时有多少个"，因为要判的是"是不是同一批"，光有数比对不出多了哪个少了哪个。
    /// 用 `BTreeSet`：与 `prune::plan` 交出的有序集合同形，比对就是集合相等。
    ///
    /// **`None` 是第三种状态，不是空集**：它说"这个人没看过任何一份名单，他批准的是这一趟"
    /// （对齐那一路可以直接这样确认）。空集说的是"他看过一份名单，那份名单是空的"，
    /// 两者在动手那一步走的是不同的判据 —— 所以这里用 `Option` 而不是用空集骗守卫。
    /// 需要一份真名单才能动手的那五路浏览器动作，在 `browser_run` 里对 `None` **拒绝执行**，
    /// 不是"当成空的下发一遍"。
    pub doomed: Option<BTreeSet<String>>,
    /// 这份清单**关于哪些位置**；空集合 = 永远作废不掉，**禁止**。写动作落进其中任何一个，凭证就作废。
    /// 对齐的预览是**两侧**都算（源变了删除候选也会变），所以这个集合不能只放目标端。
    /// "非空"这一条由 `PreviewRegistry::issue` 在边界上拦住，理由与证人写在那一句的文档上。
    pub scopes: Vec<Scope>,
}

#[derive(Default)]
pub struct PreviewRegistry {
    inner: Mutex<BTreeMap<String, StoredPreview>>,
    /// 人在**只读盘点**里看过的那份删除候选（2026-10-09）。
    ///
    /// 它**不是凭证**：没有 token、不能授权任何动作、消费它也不会跑任何东西。
    /// 它只回答一件事——"人看过哪一份清单"，供对齐动手前那道逐条比对当基准。
    /// 有了它，「看差异 → 让两边对齐 → 直接确认」就不用再点一次「重新预览」：
    /// 盘点算的 `plan(local, remote).doomed` 与预览算的**是同一个东西**（同一个 `plan` 内核），
    /// 重新列一遍只是为了"看一眼"，而人已经看过了。
    ///
    /// **键必须带方向**：键是 `src>dst` 而不是无序集合。同一对位置两个方向的 `doomed`
    /// 互为补集，拿反方向那份当基准会判错（多半每次都拒，但那是撞对的）。
    /// 顺序进键这件事在构造上就堵死了，不靠调用方的自觉。
    ///
    /// **不按 `invalidate` 清空**：写动作让树变了，那正该由动手前那次重列比对出来并**拒绝**，
    /// 而不是悄悄退到"没看过清单"那一档（更弱）。这一张表是缓存不是授权，清掉它等于降级。
    /// 大小有上限：超过 `SEEN_CAP` 的不记 —— 那种规模的比例守卫本来就几乎必然拒绝，
    /// 而在内存里留住几十万条名字是这个进程付不起的（对照：`last_summaries` 只留计数是刻意的）。
    seen: Mutex<BTreeMap<String, BTreeSet<String>>>,
}

/// 记住一份"看过"的删除候选，超过这个条数就不记（见 `seen` 的字段文档）。
const SEEN_CAP: usize = 50_000;

/// 有方向的键：`src>dst`。方向进键，见 `PreviewRegistry::seen`。
fn dir_key(src: &Scope, dst: &Scope) -> String {
    format!("{}>{}", src.key(), dst.key())
}

/// 一次性凭证。沿用子项目 1 的做法：随机数、不落盘、不进日志（`prev-` 前缀也是原来的形状）。
///
/// `pub(crate)`：配置写那张表（`config/edit.rs` 的 `WriteRegistry`）用的是**同一个**生成器，
/// 凭证的算法全仓只许有一份。两张表本身**没有**合并：那张表的 `StoredPreview::scopes`
/// 非空是 `issue()` 在边界上钉死的断言（空 scopes 的凭证作废不掉），而一次配置写
/// 没有"关于哪两棵树"可填——填一个假 Scope 去骗守卫，比分开两张表更坏。
pub(crate) fn new_token() -> String {
    use rand::RngExt;
    format!("prev-{:032x}", rand::rng().random::<u128>())
}

impl PreviewRegistry {
    /// 登记一份清单，交出一次性凭证。
    ///
    /// **`stored.scopes` 不许为空**，这一条在边界上钉死（`assert!` 而不是 `Result`）：
    /// 空集合的那份凭证**永远作废不掉** —— `invalidate` 对空 `scopes` 的 `any()` 恒假，
    /// `supersede` 又只被另一个空集合顶掉，于是写动作落在哪儿都跟它不相干，它会一直躺在登记表上
    /// 等着被消费成一次实删。那是 fail-open 的形状，而它恰好是"忘了带 Scope"的调用方最容易长出来的
    /// 样子：这一层的读者不止对齐 —— `PreviewIntent` 除 `Mirror` 外那**五个**变体都要从浏览器侧发
    /// （删可见的 / 删点名的 / 清空前缀 / 删空目录 / 移动），`issue` 是它们共同的入口，所以规矩必须长在这里，
    /// 不能只长在调用方的自觉上。
    /// panic 在这里是对的：空 scopes 不是操作员能撞上的运行条件（界面上发不出"关于零个位置"的预览），
    /// 而是**编程错误** —— 报错给开发者、当场停，比悄悄登记一份免死的凭证便宜。
    /// 证人：`tests/preview_registry.rs` 的
    /// `an_empty_scope_set_is_refused_at_issue_because_it_can_never_be_voided`。
    pub async fn issue(&self, stored: StoredPreview) -> String {
        assert!(
            !stored.scopes.is_empty(),
            "PreviewRegistry::issue 收到空 scopes 的 StoredPreview：这样的凭证作废不掉（规矩见 StoredPreview::scopes）"
        );
        let token = new_token();
        self.inner.lock().await.insert(token.clone(), stored);
        token
    }

    /// 取走即消费：一次确认只能 authorize 一次动作，`get` 那种"看一眼还在不在"的形状
    /// 会让同一个 token 被反复试（探测路径）。
    pub async fn consume(&self, token: &str) -> Option<StoredPreview> {
        self.inner.lock().await.remove(token)
    }

    /// **写**动作用它作废：任何与被写过的前缀有交集的清单都不再作数。
    pub async fn invalidate(&self, touched: &[Scope]) {
        let mut map = self.inner.lock().await;
        map.retain(|_, stored| !stored.scopes.iter().any(|s| touched.iter().any(|t| s.conflicts(t))));
    }

    /// **发新预览**用它顶掉旧的：只按"scope 集合完全相同"删。
    /// 读操作不改变任何东西，所以它只能顶掉同一件事的上一份清单。
    pub async fn supersede(&self, scopes: &[Scope]) {
        let want: BTreeSet<String> = scopes.iter().map(|s| s.key()).collect();
        let mut map = self.inner.lock().await;
        map.retain(|_, stored| {
            let have: BTreeSet<String> = stored.scopes.iter().map(|s| s.key()).collect();
            have != want
        });
    }

    /// 记下一份"人在只读盘点里看过"的删除候选（**方向敏感**）。
    /// 超上限的那一份不记，理由见 `seen` 的字段文档。
    pub async fn remember_seen(&self, src: &Scope, dst: &Scope, doomed: BTreeSet<String>) {
        if doomed.len() > SEEN_CAP {
            return;
        }
        self.seen.lock().await.insert(dir_key(src, dst), doomed);
    }

    /// 取"人看过的那一份"当基准。**不消费** —— 它是缓存不是授权，取过还能再取。
    pub async fn peek_seen(&self, src: &Scope, dst: &Scope) -> Option<BTreeSet<String>> {
        self.seen.lock().await.get(&dir_key(src, dst)).cloned()
    }
}
