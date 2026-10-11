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

/// `ListEntry` 那一族的**加料版**：多带一个 `ModTime`，只给"两边都有、想比是不是同一份"那一档用。
///
/// 为什么不给 [`ListEntry`] 加字段：那一份是 gate 与浏览器两条链路的输入，`tests/gate.rs`
/// 与 `tests/frontend_browser.rs` 各自钉着它的形状，而 `ListShape::Tree` 明确不发 ModTime
/// （关掉它才不逐对象 HeadObject，见 `ListShape::Tree` 的注释里那次 90 秒超时）。
/// 这里另起一个类型，让"要不要付那份钱"成为调用点的选择，而不是全局改动。
///
/// `ModTime` 是 Option：这一档只对**两边都有名字**的那批对象发 stat，而 rc 在 stat 失败时
/// 会把键整个省掉（`operations/stat` 对不存在/读不到的对象 200 + 无 `item`）。拿不到不是"相同"，
/// 上层把它当成"无法判定"，绝不当成一致（见 `prune::diverged` 的口径）。
/// `operations/stat` 的 `item`，以及**同样形状**的 `EntryMeta`。
///
/// `Name` / `IsDir` 是撞名拒绝那句必需的（`browser_target_exists_refusal` 要说"改于何时"、
/// 而 `Name` 是那句里点的名），所以这两个字段留着，不因为"新内核只用 Path/Size/ModTime"
/// 就砍掉——砍掉会让 `rc_client.rs` 那两个证人（钉的是**线上字段名**）失去意义。
#[derive(Debug, Deserialize, Clone)]
pub struct EntryMeta {
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Size")]
    pub size: u64,
    #[serde(rename = "IsDir")]
    pub is_dir: bool,
    /// Option 是**新加**的那一半：`operations/stat` 对读不到的对象会 200 且不带 `ModTime`
    /// 这一键，而那一格必须落 `prune::Verdict::Unknown`（既不放过、也不指控），
    /// 不能因为"键缺了"就当成时间相同。
    #[serde(rename = "ModTime")]
    pub mod_time: Option<String>,
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
    /// 用了哪一种哈希算出来的（rclone 自己挑两侧都支持的那一种，见 `fs.HashSet.Overlap().GetOne()`）。
    /// **可能整个键缺失** —— 那就是说"没报"，不是"没有哈希可用"，两种要分得开
    /// （`Option` 而不是 `String::new()`：空串会被读成"用的是空哈希"）。
    #[serde(default, rename = "hashType")] pub hash_type: Option<String>,
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
