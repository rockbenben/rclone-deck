use std::collections::BTreeSet;

use crate::config::jobs::Job;
use crate::mutex::MultiGuard;
use crate::note::ShortNote;
use crate::rc::ListError;
use crate::scope::Scope;

use super::tasklog::{Level, TaskLog};
use super::{mirror_side_refused_refusal, Inflight, RunError, Runner};

pub(super) struct Deletion<'a> {
    pub fs: &'a str,
    pub source: Option<&'a str>,
    pub files: &'a BTreeSet<String>,
    /// Approved empty directories in child-before-parent order.
    pub directories: &'a [String],
    pub touched: &'a [Scope],
}

#[derive(Default)]
pub(super) struct Deleted {
    pub files: usize,
    pub directories: usize,
}

pub(super) struct DeleteFailure {
    pub note: ShortNote,
    pub unknown: bool,
}

impl From<DeleteFailure> for RunError {
    fn from(failure: DeleteFailure) -> Self {
        Self::Invariant(format!("{} {}", failure.note.short, failure.note.detail))
    }
}

pub(super) fn valid_delete_name(name: &str) -> bool {
    !name.contains(['\\', '\0'])
        && name.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

pub(super) fn unknown_write(error: &ListError) -> bool {
    matches!(error, ListError::Other(e) if e.submission_is_unknown())
}

impl Deleted {
    fn failure(&self, log: &TaskLog, name: &str, reason: &str, unknown: bool) -> DeleteFailure {
        let outcome = if unknown { "删除结果未知" } else { "删除失败" };
        let mut detail = format!("{name:?} {outcome}。{reason} 日志位于 {}。请重新预览。", log.path().display());
        if let Err(e) = log.kv(Level::Error, outcome, &[
            ("文件名", format!("{name:?}")),
            ("已删除文件", self.files.to_string()),
            ("已删除目录", self.directories.to_string()),
            ("原因", reason.to_string()),
        ]) {
            detail.push_str(&format!(" 写入结果日志也失败了。{e}"));
        }
        DeleteFailure {
            note: ShortNote::with_detail(
                format!("已删除 {} 个文件、{} 个目录。{outcome}。", self.files, self.directories),
                detail,
            ),
            unknown,
        }
    }

    fn audit_failure(&self, log: &TaskLog, name: &str, error: std::io::Error) -> DeleteFailure {
        DeleteFailure {
            note: ShortNote::with_detail(
                format!("已删除 {} 个文件、{} 个目录。记录结果失败。", self.files, self.directories),
                format!("{name:?} 已删除。写入 {} 失败。{error} 请重新预览。", log.path().display()),
            ),
            unknown: false,
        }
    }
}

impl Runner {
    pub(super) async fn delete_mirror_files(
        &self, job: &Job, files: &BTreeSet<String>, touched: &[Scope], log: &TaskLog, guard: MultiGuard,
    ) -> Result<(Deleted, MultiGuard), DeleteFailure> {
        // Copy may have taken a long time. Recheck the safety gates without extending the allowlist.
        let (current, _, _) = self.listed_doomed(job).await.map_err(|error| {
            Deleted::default().failure(log, "删除前复核", &error.to_string(), false)
        })?;
        if current.empty_src || current.too_much {
            let reason = mirror_side_refused_refusal(&job.name, current.empty_src, current.too_much);
            return Err(Deleted::default().failure(log, "删除前复核", &reason, false));
        }
        // rclone may treat different spellings as the same file during copy.
        // Preserve the destination entry unless all source names now exist as listed.
        if current.pending > 0 {
            let reason = format!("复制后目标端仍缺少 {} 个源端文件名。", current.pending);
            return Err(Deleted::default().failure(log, "删除前复核", &reason, false));
        }
        // Do not pass a vanished spelling to a backend that could resolve it to a renamed file.
        if let Some(name) = files.iter().find(|name| current.doomed.binary_search(name).is_err()) {
            return Err(Deleted::default().failure(log, name, "该路径已不在复核后的删除候选中。", false));
        }
        self.delete_approved(Deletion {
            fs: &job.endpoint_dst().rclone_root(),
            source: Some(&job.endpoint_src().rclone_root()),
            files,
            directories: &[],
            touched,
        }, log, guard).await
    }

    pub(super) async fn record_write_result(
        &self, touched: &[Scope], result: Result<(), ListError>,
    ) -> Result<(), ListError> {
        if result.is_ok() || result.as_ref().is_err_and(unknown_write) {
            self.invalidate_writes(touched).await;
        }
        result
    }

    /// Delete only the exact paths carried by the approved plan.
    pub(super) async fn delete_approved(
        &self, plan: Deletion<'_>, log: &TaskLog, mut guard: MultiGuard,
    ) -> Result<(Deleted, MultiGuard), DeleteFailure> {
        let mut done = Deleted::default();
        let entries = plan.files.iter().map(|name| (name, false))
            .chain(plan.directories.iter().map(|name| (name, true)));
        if let Some((name, _)) = entries.clone().find(|(name, _)| !valid_delete_name(name)) {
            return Err(done.failure(log, name, "名单中存在无法安全解析的相对路径。", false));
        }
        let mut invalidated = false;
        for (name, directory) in entries {
            if let Some(source) = plan.source {
                let reason = match self.rcd.client.operations_stat(source, name).await {
                    Ok(None) => None,
                    Ok(Some(_)) => Some("源端已出现同名路径。".to_string()),
                    Err(e) => Some(format!("无法复核源端。{e}")),
                };
                if let Some(reason) = reason {
                    return Err(done.failure(log, name, &reason, false));
                }
            }

            let mut flight = Inflight::new(guard, &self.abandoned);
            let result = if directory {
                flight.synchronous_write(self.rcd.client.operations_rmdir(plan.fs, name)).await
            } else {
                flight.synchronous_write(self.rcd.client.operations_deletefile(plan.fs, name)).await
            };
            if result.is_ok() || result.as_ref().is_err_and(unknown_write) {
                self.invalidate_after_write(plan.touched, &mut invalidated).await;
            }
            if let Err(e) = result {
                return Err(done.failure(log, name, &e.to_string(), unknown_write(&e)));
            }

            guard = flight.disarm().expect("已收到本次删除答复");
            let action = if directory {
                done.directories += 1;
                "已删除空目录"
            } else {
                done.files += 1;
                "已删除文件"
            };
            if let Err(e) = log.kv(Level::Info, action, &[("路径", format!("{name:?}"))]) {
                return Err(done.audit_failure(log, name, e));
            }
        }
        Ok((done, guard))
    }
}
