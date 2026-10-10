use std::collections::BTreeSet;
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
