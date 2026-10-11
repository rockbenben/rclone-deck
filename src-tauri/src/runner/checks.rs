use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::config::jobs::{Job, Manifest};
use crate::endpoint::Endpoint;
use crate::gate::{self, GateReport};
use crate::rc::listing::{case_conflicts, list_files_recursive, normalize_actual, normalize_expected, normalize_relative};
use crate::rc::{ListError, RcClient};

use super::RunError;

/// 期望侧：扫本地目录，key = prefix + 相对路径。
pub fn expected_keys(job: &Job, exclude: &[String]) -> Result<BTreeSet<String>, RunError> {
    let root = Path::new(&job.src);
    if !root.is_dir() {
        return Err(RunError::Invariant(format!(
            "job {} 的 src 不是目录：{}", job.id, job.src)));
    }
    normalize_expected(root, &job.prefix(), exclude).map_err(|e| RunError::Io(e.to_string()))
}

/// 实际侧：递归列举。key 直接用 rclone 返回的 `Path`——它已经带了前缀，**不能再拼一次**。
pub async fn actual_keys(client: &RcClient, job: &Job) -> Result<BTreeSet<String>, RunError> {
    // 必须走 remote_for_list()（会 Err），不能走 infallible 的 remote_part()：
    // dst 指向本地目录时后者返回 ""，于是拿本地路径去 operations_list，rclone 真的能列本地盘，
    // 于是"远端实际集合"退化成"又一个本地扫描"，gate 永远绿、盘点永远没有孤儿。
    // 这正是本面板要拦的假绿，所以让它明确报错。
    let remote = job.remote_for_list().map_err(RunError::Config)?;
    let entries = list_files_recursive(client, &job.fs_for_list(), &remote).await?;
    Ok(normalize_actual(&entries))
}

/// 取**某一侧**的文件集合，名字相对该侧自己的根（spec §4.1 第 1 步）。
///
/// 与 `expected_keys` / `actual_keys` 的区别是刻意的，不是重复实现：那两个只服务闸门（对账），
/// 两侧都归一到桶全 key；这里两侧都退到各自的根，于是 `plan(src, dst).doomed` 直接就是
/// "rclone 会删掉的那个名字"。**别把其中一个拿去喂另一个。**
///
/// 本地侧仍走我们那套遍历（重解析点不进集合、失败一律往上抛——判据与实测都在
/// `normalize_expected` 的注释上），这里只多一件事：前缀传 `""`，因为对侧的桶前缀不该出现在
/// 本地文件的名字里。远端侧只能从 `Endpoint::Remote` 那一个分支进来（本地端点是另一个分支），
/// 所以 `operations/list` 物理上拿不到本地路径——不许改用 `fs_for_list()` 那种"不是 remote 就
/// 原样返回本地路径"的写法，那是 §5.1 那个假绿的入口。
pub async fn side_set(
    client: &RcClient,
    exclude: &[String],
    ep: &Endpoint,
) -> Result<BTreeSet<String>, RunError> {
    match ep {
        Endpoint::Local(root) => {
            if !root.is_dir() {
                return Err(RunError::Invariant(format!(
                    "{} 不是一个可读的目录，算不出这一侧的集合。对齐的两侧得有一侧是 remote、另一侧是这个目录。",
                    root.display())));
            }
            normalize_expected(root, "", exclude).map_err(|e| RunError::Io(e.to_string()))
        }
        Endpoint::Remote { fs, remote } => {
            // 裁定 R64：列举根（`remote=` 那一段）在桶里**还不存在**时 `operations/list` 回 404，
            // 这一档映射成"这一侧是空集"——"第一次把一个新子目录推上云"是真实用法而不是故障，
            // 而空集在两个方向上都不害人：远端是 dst 时 doomed 为空（什么都不删），
            // 远端是 src 时"src 空而 dst 非空"正好落进既有的空基准硬拒绝。
            //
            // 这条降级今天比 BFS 时代**更**安全：整棵树是一次调用，所以 404 只可能来自被问的那
            // 一段本身。以前逐层下探时"更深一层的 404"和"列举根的 404"长得一样，需要额外探针去
            // 分辨（那个探针也确实是这么写的）；现在没有"更深一层"这种请求，那个分辨没有必要了。
            // 反过来，代价也要说清：一次调用意味着**这一侧要么全看见、要么整个读成空**，
            // 半途截断这一档在形状上不存在了（R64 那一档的口径不变）。
            let entries = match list_files_recursive(client, fs, remote).await {
                Ok(entries) => entries,
                Err(ListError::NotFound) => return Ok(BTreeSet::new()),
                Err(e) => return Err(RunError::List(e)),
            };
            normalize_relative(remote, &entries).map_err(RunError::List)
        }
    }
}

/// 两侧的时间戳**必须先归一到同一个可比形状**再比，否则比较的是两串不同格式的字节。
///
/// 云端那一侧是 rc 给的 RFC3339；本地那一侧是 `SystemTime`，直接 `to_string()` 出来的是
/// `2026-10-11 07:36:23.1234567 +08:00` 那一族。两串按字典序排毫无意义 —— 连"哪个是更晚"
/// 都问不出来，而这一层的产物是要给人当"往哪边合"的依据的。
///
/// 统一到 **UTC、毫秒精度** 的 RFC3339。两个口径都是刻意选的：
/// - **UTC**：本地时间带偏移量而云端不一定带，字典序只在统一偏移后才等于时间序。
/// - **毫秒**：秒以下留着（文件在同���秒内被改两次是真实情况），但不留到纳秒 ——
///   跨文件系统与后端时纳秒精度本来就不可靠，留着只会造出"这两个不一样"的假差异。
///
/// **归一不出来就返回 `None`**（那是 `prune::Verdict::Unknown` 那一档），不拿原串去比。
pub fn comparable_mod_time(t: chrono::DateTime<chrono::Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// 一个 Unix 时刻（秒 + 毫秒）→ 与远端同一形状的 RFC3339。
///
/// **两侧都必须走这里**（`tests/gate.rs` 有一条证人钉着）：本地那一侧早先自己拼了一个
/// `DateTime::from_timestamp(secs, 0)`，亚秒位被截到整秒，而远端那一侧留着毫秒 ——
/// 同一个时刻于是归一化成两串不同的字面量，一个**完全没动过**的文件被报成"云端那份更新"。
/// 那个洞不是精度之争，是**两边必须落在同一个刻度**这件事被漏掉了。
pub fn normalize_stamp(secs: i64, millis: u32) -> String {
    chrono::DateTime::from_timestamp(secs, millis.saturating_mul(1_000_000))
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default()
}

/// rc 交回来的 ModTime → 同一形状。**认不出来返回 `None`**，那一格于是落
/// `prune::Verdict::Unknown`：大小对得上、时间读不出，就既不放过也不指控。
///
/// 上游某个后端给的时间格式不在 `SystemTime` 能解析的那一族时，原串会被交出去，
/// 而它和本地那一侧的归一化串**永远不等** —— 于是每一对文件都被报成"云端更新"，
/// 那是一份全错的清单，比不报更坏。所以这里失败就是失败。
pub fn normalize_remote_time(iso: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(iso).ok().map(|t| {
        // 与本地那一侧**同一个** `normalize_stamp`：两条路都从 `(秒, 毫秒)` 出发，
        // 于是输出形状由一处决定（本地那一侧是 `duration_since(UNIX_EPOCH)`）。
        normalize_stamp(t.timestamp(), t.timestamp_subsec_millis())
    })
}

/// 某一侧的元数据，**只对 `names` 里那些**去拿（大小 + 修改时间）。
///
/// 为什么不是整棵树一次拿回来：`ListShape::Tree` 明确不发 ModTime，因为那意味着
/// 真桶上**每个对象一次 HeadObject** —— 操作员报过的那次「看差异」90 秒超时，量到的就是
/// 那个形状（`ListShape::Tree` 的注释里记着，`rcd.log` 里全是 `Failed to read metadata`）。
/// 而这里要的恰恰是 ModTime，所以只能**按需单个问**：交集通常远小于整棵树，
/// 真实图库里绝大多数文件两边一模一样，根本不该付那份钱。
///
/// `names` 是**交集**（两边都有的那些），差集那两半不归这里 —— 它们由 `prune::plan` 负责，
/// 而且那边只要名字就够了。**exclude 不在这里重跑**：`names` 已经由 `side_set` 过了一遍
/// exclude（那才是匹配器的唯一实现），这里再判一次就是第二套口径。
///
/// 本地侧走文件系统自己读，不问 rclone：那是本地盘，`std::fs` 一次 stat 就够，
/// 而绕一圈 rc 反而把本地路径交给了后台。
pub async fn side_meta(
    client: &RcClient,
    ep: &Endpoint,
    names: &BTreeSet<String>,
) -> Result<BTreeMap<String, crate::prune::FileMeta>, RunError> {
    let mut out = BTreeMap::new();
    match ep {
        Endpoint::Local(root) => {
            for name in names {
                let p = root.join(name);
                // 读不到（ACL、路径过长、重解析点）的那一条**不进映射**，于是它在 `diverged`
                // 里因为缺项而不出现 —— 那是"没看见"，不是"两边一样"，两者的区别见
                // `prune::Verdict::Unknown` 的注释。绝不用 0 字节顶替，那等于凭空说它没改。
                let Ok(m) = std::fs::metadata(&p) else { continue };
                if !m.is_file() { continue; }
                // 亚秒位**必须**带进去：本地这一侧截到整秒、远端那一侧留着毫秒，
// 同一个时刻就会在 `newer_of` 里比出"不一样" —— 于是一个**完全没动过的文件**
// 被报成"云端那份更新（按上传会冲掉它）"。那是凭空造一个方向，比方向缺更坏。
// （`SystemTime` → `DateTime` 的亚秒精度按毫秒取即可：跨文件系统与后端时纳秒本就不可靠，
//  但**两边截到同一个刻度**是硬要求，不是精度之争。）
let mod_time = m.modified().ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    // 走 `normalize_stamp`（不是自己拼 `from_timestamp`）：那个洞就是两边
                    // 各拼各的导致的 —— 本地截整秒、远端留毫秒。证人钉的就是"两侧同源"。
                    .map(|d| normalize_stamp(d.as_secs() as i64, d.subsec_millis()));
                out.insert(name.clone(), crate::prune::FileMeta {
                    name: name.clone(), size: m.len(), mod_time,
                });
            }
        }
        Endpoint::Remote { fs, remote } => {
            let root = remote.trim_matches('/');
            for name in names {
                let full = if root.is_empty() { name.clone() } else { format!("{root}/{name}") };
                // 云端这一侧的 ModTime 本来就是 RFC3339，但要**过一遍归一化**才敢和本地那侧
                // 比：rc 给的精度与偏移量在真桶上并不统一（实测 v1.75.1 的 S3 后端给的是
                // `…Z`，local 后端给的是带偏移量的），不归一就是两串不同格式的字典序。
                // `if let` 而非 `match`：只有拿到 item 那一支有事要做，别的都是"不出现"。
                if let Some(m) = client.operations_stat(fs, &full).await? {
                    out.insert(name.clone(), crate::prune::FileMeta {
                        name: name.clone(), size: m.size,
                        mod_time: m.mod_time.as_deref().and_then(normalize_remote_time),
                    });
                }
                // stat 回 200 且没有 item：这一刻它不在那儿。集合是刚才列出来的，
                // 这一刻又不在——中间有人动了。不当"相同"（那是一次放行），
                // 也不当"不同"（那是凭空的指控），只是**不出现**。
            }
        }
    }
    Ok(out)
}

pub fn gate_report(expected: &BTreeSet<String>, actual: &BTreeSet<String>) -> GateReport {
    gate::diff(expected, actual, case_conflicts(actual))
}

pub fn require_local_dir_manifest(job: &Job) -> Result<(), RunError> {
    let manifest = job.manifest.as_ref().ok_or_else(|| RunError::Invariant(format!(
        "gate 任务 {} 缺 manifest 段", job.id)))?;
    match manifest {
        Manifest::LocalDir => Ok(()),
        Manifest::D1 => Err(RunError::Invariant(format!(
            "manifest {manifest:?} 属子项目 2，本子项目只实现 localdir"))),
        Manifest::File(_) => Err(RunError::Invariant(format!(
            "manifest {manifest:?} 还没实现，本子项目只实现 localdir"))),
    }
}
