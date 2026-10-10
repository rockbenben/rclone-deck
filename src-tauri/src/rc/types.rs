use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct SubmitAck {
    #[serde(rename = "executeId")]
    pub execute_id: String,
    pub jobid: i64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct JobStatus {
    pub id: i64,
    pub group: String,
    pub finished: bool,
    pub success: bool,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub duration: f64,
}

/// `core/stats` 的完整字段集（实测枚举，spec §3.2）。
/// `listed` 留在结构体里是因为要解析它，但它**不会**出现在 `StatsView` 里。
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CoreStats {
    pub bytes: u64,
    pub checks: u64,
    pub deleted_dirs: u64,
    pub deletes: u64,
    pub elapsed_time: f64,
    pub errors: u64,
    pub eta: Option<u64>,
    pub fatal_error: bool,
    pub listed: u64,
    pub renames: u64,
    pub retry_error: bool,
    pub server_side_copies: u64,
    pub server_side_copy_bytes: u64,
    pub server_side_move_bytes: u64,
    pub server_side_moves: u64,
    pub speed: f64,
    pub total_bytes: u64,
    pub total_checks: u64,
    pub total_transfers: u64,
    pub transfer_time: f64,
    pub transfers: u64,
}

/// 唯一允许流向界面的 stats 形态。没有 `listed` 字段——
/// "界面不显示 listed" 靠类型系统保证，不靠前端自觉。
/// 之所以要有这个类型：`listed` 数的是两侧被扫过的条目（实测一次只显示 1 个文件在传
/// 时 listed=12237），它一旦被当成"待传数"，进度条就是错的。
#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatsView {
    pub transfers: u64,
    pub total_transfers: u64,
    pub checks: u64,
    pub total_checks: u64,
    pub bytes: u64,
    pub total_bytes: u64,
    pub speed: f64,
    pub eta: Option<u64>,
    pub errors: u64,
    pub deletes: u64,
    pub deleted_dirs: u64,
    pub retry_error: bool,
    pub fatal_error: bool,
    pub elapsed_time: f64,
}

impl CoreStats {
    pub fn view(&self) -> StatsView {
        StatsView {
            transfers: self.transfers,
            total_transfers: self.total_transfers,
            checks: self.checks,
            total_checks: self.total_checks,
            bytes: self.bytes,
            total_bytes: self.total_bytes,
            speed: self.speed,
            eta: self.eta,
            errors: self.errors,
            deletes: self.deletes,
            deleted_dirs: self.deleted_dirs,
            retry_error: self.retry_error,
            fatal_error: self.fatal_error,
            elapsed_time: self.elapsed_time,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ListEntry {
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Size")]
    pub size: u64,
    #[serde(rename = "IsDir")]
    pub is_dir: bool,
}

/// `operations/stat` 的 `item`。实测这个对象带 Path/Name/Size/MimeType/ModTime/IsDir，
/// 这里只收本面板要读的四个（Name/Size/IsDir/ModTime）。
/// `MimeType` 与 `Path` 今天没人读，所以不预先存——存了就成了没人验证、又没人删的字段。
#[derive(Debug, Deserialize, Clone)]
pub struct StatObject {
    #[serde(rename = "Name")] pub name: String,
    #[serde(rename = "Size")] pub size: u64,
    #[serde(rename = "IsDir")] pub is_dir: bool,
    #[serde(rename = "ModTime")] pub mod_time: String,
}

/// `operations/size`。实测 `{"bytes":3,"count":3,"sizeless":0}`；`sizeless` 今天没人读。
/// **注意**：这一档**受排除规则影响**（F11），所以它是"过滤后的用量"。
#[derive(Debug, Deserialize, Clone)]
pub struct Sizes {
    #[serde(rename = "bytes")] pub bytes: u64,
    #[serde(rename = "count")] pub count: u64,
}

/// `operations/check` 的真值在三个列表里，**不在 `error` 里**（F7：那里的 `error` 是数组）。
/// 所以这个结构**不读 `error`**：读了就要决定"空数组算不算失败"，而答案是"那不是失败也不是成功"。
#[derive(Debug, Deserialize, Clone, Default)]
pub struct CheckOutcome {
    #[serde(default, rename = "success")] pub success: bool,
    #[serde(default, rename = "missingOnSrc")] pub missing_on_src: Vec<String>,
    #[serde(default, rename = "missingOnDst")] pub missing_on_dst: Vec<String>,
    #[serde(default, rename = "differ")] pub differ: Vec<String>,
}

/// `operations/fsinfo`。能力位整张表收下来按需读。
/// 用 `HashMap<String, bool>` 而不是逐位建字段——那是替 rclone 的版本演进维护一张表；
/// 而且**缺位与 false 不是同一个值**（缺位 = 这个后端没报，false = 一句确定的话）。
/// 逐位建字段还会在编译期就说不准：`#[serde(default)]` 把缺位直接落成 `false`。
#[derive(Debug, Deserialize, Clone)]
pub struct FsInfo {
    #[serde(default, rename = "Features")] pub features: std::collections::HashMap<String, bool>,
    #[serde(default, rename = "Name")] pub name: String,
    #[serde(default, rename = "Root")] pub root: String,
}
