use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::config::jobs::{Job, Manifest};
use crate::endpoint::Endpoint;
use crate::gate::{self, GateReport};
use crate::prune::FileMeta;
use crate::rc::listing::{case_conflicts, list_files_recursive, normalize_actual, normalize_expected};
use crate::rc::{ListError, RcClient};

use super::RunError;

/// 期望侧：扫本地目录，key = prefix + 相对路径。
pub fn expected_keys(job: &Job, exclude: &[String]) -> Result<BTreeSet<String>, RunError> {
    let root = Path::new(&job.src);
    if !root.is_dir() {
        return Err(RunError::Invariant(format!(
            "「从哪里来」不是一个可读的目录：{}。对账要用这个本地目录里的文件清单；到「设置」里的「改这条」换成别的目录。",
            root.display())));
    }
    normalize_expected(root, &job.prefix(), exclude).map_err(|e| RunError::Io(e.to_string()))
}

/// 实际侧：从任务根列举，然后还原为对账报告使用的桶内 key。
pub async fn actual_keys(client: &RcClient, job: &Job) -> Result<BTreeSet<String>, RunError> {
    job.remote_for_list().map_err(RunError::Config)?;
    let entries = list_files_recursive(client, &job.endpoint_dst().rclone_root(), "").await?;
    let prefix = job.prefix();
    Ok(normalize_actual(&entries).into_iter().map(|name| format!("{prefix}{name}")).collect())
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
                    "{} 不是一个可读的目录，算不出这一侧的集合。对齐的两侧得有一侧是 remote、另一侧是这个目录。\
                     它可能被移走或改名了；要换成别的目录，到「设置」里的「改这条」改这一条，\
                     或在「文件任务」上点「新建临时任务」临时换一个。",
                    root.display())));
            }
            normalize_expected(root, "", exclude).map_err(|e| RunError::Io(e.to_string()))
        }
        Endpoint::Remote { root } => {
            // 裁定 R64：任务根在桶里还不存在时 `operations/list` 回 404，
            // 这一档映射成"这一侧是空集"——"第一次把一个新子目录推上云"是真实用法而不是故障，
            // 而空集在两个方向上都不害人：远端是 dst 时 doomed 为空（什么都不删），
            // 远端是 src 时"src 空而 dst 非空"正好落进既有的空基准硬拒绝。
            //
            // 这条降级今天比 BFS 时代**更**安全：整棵树是一次调用，所以 404 只可能来自被问的那
            // 一段本身。以前逐层下探时"更深一层的 404"和"列举根的 404"长得一样，需要额外探针去
            // 分辨（那个探针也确实是这么写的）；现在没有"更深一层"这种请求，那个分辨没有必要了。
            // 反过来，代价也要说清：一次调用意味着**这一侧要么全看见、要么整个读成空**，
            // 半途截断这一档在形状上不存在了（R64 那一档的口径不变）。
            remote_names(client, root).await
        }
    }
}

async fn remote_names(client: &RcClient, root: &str) -> Result<BTreeSet<String>, RunError> {
    match list_files_recursive(client, root, "").await {
        Ok(entries) => Ok(entries.into_iter().map(|entry| entry.path).collect()),
        Err(ListError::NotFound) => Ok(BTreeSet::new()),
        Err(e) => Err(RunError::List(e)),
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

pub(super) fn local_metadata(root: &Path, names: &BTreeSet<String>) -> BTreeMap<String, FileMeta> {
    let mut out = BTreeMap::new();
    for name in names {
        let Ok(metadata) = std::fs::metadata(root.join(name)) else { continue };
        if !metadata.is_file() { continue; }
        let mod_time = metadata.modified().ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|time| normalize_stamp(time.as_secs() as i64, time.subsec_millis()));
        out.insert(name.clone(), FileMeta { name: name.clone(), size: metadata.len(), mod_time });
    }
    out
}

pub(super) async fn inventory_remote(
    client: &RcClient, root: &str, local: &BTreeSet<String>,
) -> Result<(BTreeSet<String>, Option<BTreeMap<String, FileMeta>>), RunError> {
    // On backends such as S3, asking for ModTime may add a transaction per
    // listed object. Require an explicit fast-metadata capability and unique names.
    let batch = local.len() > 1 && client.operations_fsinfo(root).await.is_ok_and(|info| {
        info.features.get("SlowModTime") == Some(&false)
            && info.features.get("DuplicateFiles") == Some(&false)
    });
    if !batch {
        return Ok((remote_names(client, root).await?, None));
    }
    let entries = match client.operations_list_metadata(root).await {
        Ok(entries) => entries,
        Err(ListError::NotFound) => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    let mut names = BTreeSet::new();
    let mut metadata = BTreeMap::new();
    for entry in entries.into_iter().filter(|entry| !entry.is_dir) {
        if !names.insert(entry.path.clone()) {
            metadata.remove(&entry.path);
            continue;
        }
        if !local.contains(&entry.path) { continue; }
        if let (Some(size), Some(mod_time)) = (
            entry.size.as_u64(), entry.mod_time.as_str().and_then(normalize_remote_time),
        ) {
            metadata.insert(entry.path.clone(), FileMeta { name: entry.path, size, mod_time: Some(mod_time) });
        }
    }
    Ok((names, Some(metadata)))
}

pub(super) async fn inventory_remote_metadata(
    client: &RcClient, root: &str, names: &BTreeSet<String>, listed: Option<BTreeMap<String, FileMeta>>,
) -> Result<BTreeMap<String, FileMeta>, RunError> {
    let batched = listed.is_some();
    let mut out = listed.unwrap_or_default();
    for name in names {
        if out.contains_key(name) { continue; }
        match client.operations_stat(root, name).await {
            Ok(Some(item)) if !item.is_dir => {
                out.insert(name.clone(), FileMeta {
                    name: name.clone(), size: item.size,
                    mod_time: item.mod_time.as_deref().and_then(normalize_remote_time),
                });
            }
            Ok(_) => {}
            Err(_) if batched => {} // This name is reported as unknown.
            Err(e) => return Err(e.into()),
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
