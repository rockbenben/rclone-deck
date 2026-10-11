//! rclone `rcd` 的 HTTP 客户端：面板与 rclone 之间唯一的通道。
//!
//! 全部语义都来自实测（`npm run probe:rc` 会把它们逐条重跑一遍），
//! 注释里标了"实测"的地方都是被真 rclone 验证过的行为，别按常理改。

// 递归列举 = 一次 `operations/list` 带上 `opt`（见 `ListShape::opt`），用在哪一侧归 `listing` 管，
// 客户端只负责把 `opt` 原样发出去 —— 两种形状都发得出去，这是量出来的事实：
// `opts`+`recursive` 那种拼法被 rclone **静默忽略**（旧注释据此写过"rc 的 list 不递归"，那是过度推广），
// 正确的拼法是容器键 `opt`（单数）+ 字段 `recurse`，而它**穿过本客户端用的表单编码照样生效**——
// `tools/rc-contract-probe.mjs` §4b 钉着这一条，带"拿掉 opt 只回单层"的对照组。
pub mod listing;
pub mod types;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::Client;
use serde_json::Value;
use thiserror::Error;
use types::*;

/// 一次 rc 调用的上限。rcd 就在回环上，30 秒还没回就是出了问题；
/// 重试策略不在这个客户端里（Task 9 定）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 列举那一档自己的上限。它是唯一会把一个前缀整个翻一遍的调用（远端一页 1000 条），
/// 用全局那档会把一次"大但正常"的列举报成故障。这一档是**假设值，未在真桶量过**：
/// 单层列举之后它撞上的概率低得多（本地量过：同一棵树单层 34 ms / 递归 247 ms，
/// 读数在 `operations_list` 头上那组里），
/// 真桶那一档要等第一次实测再收。
const LIST_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Error)]
pub enum RcError {
    #[error("rc 调用 {method} 返回 HTTP {status}：{body}")]
    Http { status: u16, method: String, body: String },
    #[error("rc 调用 {method} 失败：{message}")]
    Rc { method: String, message: String },
    #[error("无法连接 rcd：{0}")]
    Transport(String),
    /// 带上当时的上限秒数：面向操作员那句话要说"等了多久"，而界面不许自己记一个数
    /// （判定与读数都从后台来）。
    #[error("rcd 响应超时：{method}（等了 {budget_secs} 秒）")]
    Timeout { method: String, budget_secs: u64 },
}

impl serde::Serialize for RcError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Error)]
pub enum ListError {
    #[error("该前缀在远端不存在")]
    NotFound,
    #[error("{0}")]
    Invariant(String),
    #[error(transparent)]
    Other(#[from] RcError),
}

impl serde::Serialize for ListError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// `_config` 白名单。实测未知键会 fail-open（rclone 返回 success=true 并静默忽略），
/// 所以只允许已被证明生效的键，多一个都不行。
/// `exclude` 这类 fs.Config flag 只能拼进 rcd 启动参数，塞进 `_config` 会被静默忽略（实测）。
const ALLOWED_CONFIG_KEYS: &[&str] = &["dryRun"];

pub struct RcClient {
    http: Client,
    base: String,
    /// Basic 头的值。故意不 derive Debug：密码不能跟着 `{rc:?}` 漏进日志或前端。
    auth: String,
    /// 发出去的 `operations/list` 次数，单调递增、永不重置。
    /// 调用方按"前后取差值"用，所以并发任务会把彼此的 LIST 算进同一段区间——
    /// 这只影响日志里的一个观测数字，不参与任何判定，故不加锁也不分区。
    list_requests: AtomicUsize,
}

/// 列举的两种形状。**要什么**由这个类型说，不由调用点临时拼 `opt`。
///
/// 为什么不是一个 `recurse: bool`：未识别的 rc 参数是 **fail-open** 的（§3 / 裁定 R51），
/// 拼错一个键名不会报错、不会回声，只会安静地给你一份默认答案 —— 就是这么踩过的：
/// 第一版按 `rclone lsf` 的旗标名发 `{"dirs":false,"files":true}`，两个键都不存在，
/// 回来的集合与默认逐字节相同（`tools/rc-contract-probe.mjs` §4d 把这条钉成了对照组）。
/// 真名是 `filesOnly` / `noModTime` / `noMimeType`，而且**每一档要哪几样是固定的**，
/// 所以把它们收进类型，而不是留给每个调用点去记。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListShape {
    /// 整棵树的**文件**：`recurse` + `filesOnly` + `noModTime` + `noMimeType`。
    ///
    /// 后两个不是顺手省的带宽：面板没有一处读 `ModTime` / `MimeType`（`ListEntry` 只有四个字段：
    /// `Path` 给集合差，`Size` 与 `IsDir` 给浏览器那一档），而让 rclone 去把那两样凑齐，在真桶上就是**每个对象一次
    /// `HeadObject`**——操作员报的那次「看差异」90 秒超时量的就是这个形状，`rcd.log` 里
    /// 全是 `Failed to read metadata: operation error S3: HeadObject`。
    /// 目录行则从一开始就是白要 —— 四个使用方拿到后第一件事都是 `!is_dir` 丢掉。
    Tree,
    /// 只列**这一层**：目录行留着（浏览器要靠它导航），但 `noModTime` + `noMimeType` 照关。
    ///
    /// 这一档以前一个 `opt` 都不发，理由是"大小与目录都要显示"。前半句站不住：
    /// `Size` 是列举本身给的（真桶实测：关掉那两样之后每个对象的 Size 一个不缺），
    /// 而"什么都不关"在对象存储上意味着逐对象去凑 metadata —— 单层也要付这份钱，
    /// 一个平铺的大前缀能把浏览器那一发拖过面板自己的闸。后半句是真的，所以
    /// `filesOnly` **不进这一档**：那一串会把目录行一起取消，浏览器就没有文件夹可点了。
    Level,
}

impl ListShape {
    /// 发出去的 `opt`（form 编码里的一个 JSON 串；`opt` 是单数，实测 §4b）。
    fn opt(self) -> Option<&'static str> {
        match self {
            ListShape::Tree =>
                Some("{\"recurse\":true,\"filesOnly\":true,\"noModTime\":true,\"noMimeType\":true}"),
            ListShape::Level => Some("{\"noModTime\":true,\"noMimeType\":true}"),
        }
    }
}

/// 一次调用的原始结果。状态码留着，因为有的调用方要按码分支（见 operations_list）。
struct RcResponse {
    status: u16,
    json: Value,
    text: String,
}

/// 哪些方法的 200 载荷里 `error` 是**数据**而不是故障（裁定 R35）。只有 `job/status`：
/// 它的响应体就是那条 job 记录本身，`error` 是"这次任务失败的原因"，成功的任务里它是空串。
/// 判据不许写成"`error` 为空就放行"那种通用规则——那会把 `core/stats` 之类真正的
/// 200+error 故障也一并放过去；放行的是**方法**，不是**空串**。
/// 实测（本机 v1.75.1 真 daemon）：
/// - 成功的 `job/status`：HTTP 200、`finished=true`、`success=true`，而 `"error": ""` 确实在载荷里；
/// - 失败的 job：HTTP 200、`success=false`、`error` 是非空原因（"directory not found"）；
/// - 真·未知 jobid：HTTP **500** + `"job not found"`（走下面的 `status >= 400` 分支，与此无关）。
fn error_is_payload(method: &str) -> bool {
    matches!(method, "job/status")
}

impl RcResponse {
    /// 把状态码折叠成 RcError：这是公开 `call` 的口径。
    fn into_value(self, method: &str) -> Result<Value, RcError> {
        if self.status >= 400 {
            return Err(self.error_for(method));
        }
        // 200 带 `error` 有两种完全不同的意思，不能一律当故障（裁定 R35，实测）：
        // - **`job/status` 的 `error` 是载荷**。成功任务带 `"error": ""`；失败任务带
        //   `"error": "directory not found"` 且 `success=false`（都是 200）。真故障走 500，
        //   上面 `status >= 400` 已经接住。
        // - 其余方法（`sync/copy`、`core/stats`……）200 里带 `error` 才是"rc 包了一层错"。
        // 一律折叠的后果很具体：Task 9 每一次轮询都会被判成"读不到状态"，重试耗尽进 `Unknown`，
        // 而 rclone 明明好好答了——一个跑成功的 copy 会被报成"状态说不准"；也正是 Task 7
        // 当初不得不改轮 `job/list` 才能把活干完的原因。
        if !error_is_payload(method) {
            // rc 偶尔会用 200 包一个 error（版本差异），一样当失败处理。
            if let Some(err) = self.json.get("error").and_then(Value::as_str) {
                return Err(RcError::Rc { method: method.to_string(), message: err.to_string() });
            }
        }
        Ok(self.json)
    }

    /// rc 的错误规矩得很：JSON 里带 `error`。不带就说明说话的不是 rclone
    /// （端口上挂着别的进程、代理 502……），这时把状态码和原始 body 交出去，
    /// 不伪装成"rc 报了一个错"。
    fn error_for(&self, method: &str) -> RcError {
        match self.json.get("error").and_then(Value::as_str) {
            Some(message) => RcError::Rc {
                method: method.to_string(),
                message: message.to_string(),
            },
            None => RcError::Http {
                status: self.status,
                method: method.to_string(),
                body: self.text.clone(),
            },
        }
    }
}

impl RcClient {
    pub fn new(port: u16, user: &str, pass: &str) -> Self {
        Self::from_base_url(format!("http://127.0.0.1:{port}"), user, pass)
    }

    /// base_url 单独开口是为了测试打桩；生产代码只用 `new`（只可能打回环）。
    /// 凭据只进 Basic 头，绝不进 URL——URL 会出现在 reqwest 的错误话术里。
    pub fn from_base_url<S: Into<String>>(base: S, user: &str, pass: &str) -> Self {
        let token = STANDARD.encode(format!("{user}:{pass}"));
        Self {
            http: Client::builder().timeout(REQUEST_TIMEOUT).build().expect("reqwest client"),
            base: base.into(),
            auth: format!("Basic {token}"),
            list_requests: AtomicUsize::new(0),
        }
    }

    /// 已发出的 LIST 次数。用途见字段注释：调用方取差值，写进任务日志。
    pub fn list_request_count(&self) -> usize {
        self.list_requests.load(Ordering::Relaxed)
    }

    pub async fn call(&self, method: &str, params: &[(&str, String)]) -> Result<Value, RcError> {
        self.send(method, params).await?.into_value(method)
    }

    /// 提交异步任务。`dry_run` 为 true 时附带 `_config={"dryRun":true}`。
    pub async fn submit_async(
        &self,
        method: &str,
        params: &[(&str, String)],
        dry_run: bool,
    ) -> Result<SubmitAck, RcError> {
        // `_config` 由 dry_run 这个布尔决定，不接受调用方自备：
        // 否则 dry_run=false 的一次"真跑"可以被一个 _config 悄悄改成干跑——
        // 任务照样 success=true，只是一个字节都没传。
        if params.iter().any(|(k, _)| *k == "_config") {
            return Err(RcError::Rc {
                method: method.to_string(),
                message: "submit_async 自己决定 _config，params 里不许传：要干跑就 dry_run=true".into(),
            });
        }
        let mut owned: Vec<(&str, String)> = params.to_vec();
        owned.push(("_async", "true".to_string()));
        if dry_run {
            // 这是我们唯一主动发送的 _config。即便如此它照样要过 send() 里的白名单——
            // 白名单是发送前的闸门，不是这段字面量的装饰。
            let cfg = serde_json::json!({ "dryRun": true });
            owned.push(("_config", cfg.to_string()));
        }
        let v = self.call(method, &owned).await?;
        // 状态码必须已经处理完才轮得到 jobid：实测非法 _config 是 400 且 body 里没有 jobid。
        let jobid = v.get("jobid").and_then(Value::as_i64).ok_or_else(|| RcError::Rc {
            method: method.to_string(),
            message: format!("响应里没有 jobid：{v}"),
        })?;
        let execute_id = v.get("executeId").and_then(Value::as_str).unwrap_or_default().to_string();
        Ok(SubmitAck { execute_id, jobid })
    }

    pub async fn job_status(&self, jobid: i64) -> Result<JobStatus, RcError> {
        // 参数名必须是 jobid；传 group 会得到 400 Didn't find key "jobid"（实测）。
        let v = self.call("job/status", &[("jobid", jobid.to_string())]).await?;
        serde_json::from_value(v).map_err(|e| RcError::Rc {
            method: "job/status".into(), message: format!("解析 job/status 响应失败：{e}") })
    }

    pub async fn core_stats(&self, group: &str) -> Result<CoreStats, RcError> {
        // 与 job/status 相反：core/stats 认的是 group=job/N（实测）。
        let v = self.call("core/stats", &[("group", group.to_string())]).await?;
        serde_json::from_value(v).map_err(|e| RcError::Rc {
            method: "core/stats".into(), message: format!("解析 core/stats 响应失败：{e}") })
    }

    /// 列一个前缀，形状由 [`ListShape`] 定（`Tree` = 整棵树的文件；`Level` = 只这一层，
    /// 目录行与全字段都留着）。两种都是量出来的形状：
    /// `tools/rc-contract-probe.mjs` §4b 钉递归开关的拼法（单数 `opt`、键名 `recurse`、
    /// **form 编码也生效**），§4d 钉 `filesOnly` / `noModTime` / `noMimeType` 三个真名，
    /// 以及"不存在的键会被静默忽略"那条对照。另有一组耗时读数：同一棵
    /// 5,010 对象的树，递归回 5,210 条 / 247 ms，单层回 210 条 / 34 ms；
    /// 操作员自己的 vault 根上是 6,110 条 vs 2 条。
    ///
    /// 两个使用方要的不是同一件事，所以形状是参数而不是常量：
    /// - 集合差与盘点（`listing.rs`）要**整棵树**——一次递归列举取代逐层 BFS 是量过才换的
    ///   （10,020 个对象 / 200 个目录：203 个请求 2894 ms → 1 个请求 59 ms，两侧集合逐字节相同）。
    /// - 浏览器那一面要**一层**：它是个两栏文件浏览器，把整棵子树摊成"这一层"既是慢的原因，
    ///   也让「删掉这一层看到的 N 个」这句话和它实际动的范围不是一回事。
    ///
    /// 别复制这个函数来做另一档（上面那条注释说的就是这个）：条件化 `opt` 那一行。
    pub async fn operations_list(&self, fs: &str, remote: &str, shape: ListShape) -> Result<Vec<ListEntry>, ListError> {
        // remote="." 不是"当前目录"而是 404；结尾斜杠会拿到空列表（都是实测）。
        // 两条都在发请求之前掐掉。桶根是 ""，不是 "."。
        if remote == "." {
            return Err(ListError::Other(RcError::Rc {
                method: "operations/list".into(),
                message: "remote 不能是 \".\"：rclone 会返回 404 directory not found；云端根用 \"\"".into(),
            }));
        }
        let remote = remote.trim_matches('/');
        // 记账点在客户端侧那唯一一次早退（`remote="."`）与尾部斜杠折叠**之后**、真正 `send` 之前：没发出去的请求不能计数，
        // 否则日志里的 LIST 数比账单上的多。
        self.list_requests.fetch_add(1, Ordering::Relaxed);
        let mut params = vec![("fs", fs.to_string()), ("remote", remote.to_string())];
        if let Some(opt) = shape.opt() {
            params.push(("opt", opt.to_string()));
        }
        // 列举给一档自己的超时：它是唯一会把远端**整个前缀**翻一遍的调用，
        // 而其余调用是几百毫秒的事。全局那档仍然管着别的（`REQUEST_TIMEOUT`）。
        let res = self.send_with_timeout("operations/list", &params, LIST_TIMEOUT)
            .await.map_err(ListError::Other)?;

        // 目录不存在就是 404（实测）。按状态码认，不按 rclone 的英文措辞认——
        // 措辞随后端和版本变，"directory not found" 只是本地后端今天的样子。
        if res.status == 404 {
            return Err(ListError::NotFound);
        }
        // 404 之后一律走 into_value：那是这个文件唯一的失败口径，也是 call() 用的那一个。
        // 自己重写一遍 `>= 400` 会漏掉它那条"rc 偶尔用 200 包一个 error（版本差异）"的
        // 分支——而 operations_list 恰恰是最不能漏的地方，见下面 list 的取法。
        let json = res.into_value("operations/list").map_err(ListError::Other)?;
        // list 必须真的存在、而且真的是个数组。缺键或非数组都不是"空目录"：
        // 空目录的表示是 `{"list": []}`（实测）。把上面两种读成 Ok(vec![])，
        // 就等于让 `listing` 那一趟"整棵树一次读完"把"这个目录没看见"当成"这个目录是空的"——
        // 门禁会报"没有缺图"、盘点会报"没有孤儿"，而它们是静默跳过它的。
        // fail-closed：宁可让任务带着错误停下，也不报一个没人验证过的 success。
        let list = json.get("list").and_then(Value::as_array).cloned().ok_or_else(|| {
            ListError::Other(RcError::Rc {
                method: "operations/list".into(),
                message: format!("响应里没有一个 list 数组（不能当作空目录）：{json}"),
            })
        })?;
        // 返回条目的 Path 已经含前缀，不能再拼一次（实测）。这里原样交出去。
        serde_json::from_value(Value::Array(list)).map_err(|e| {
            ListError::Other(RcError::Rc {
                method: "operations/list".into(),
                message: format!("解析 list 失败：{e}"),
            })
        })
    }

    pub async fn core_version(&self) -> Result<String, RcError> {
        let v = self.call("core/version", &[]).await?;
        Ok(v.get("version").and_then(Value::as_str).unwrap_or("unknown").to_string())
    }

    /// 列配置里的远端名。**只此一个 `config/*`**：`config/dump` 与 `config/get` 的响应里含明文
    /// 凭据（F4），列出来一次就等于把密钥写进界面与日志。回来的名字**不带冒号**（实测）。
    ///
    /// **这条注释的写法纪律**（不是风格问题，是有证人在盯）：解释"为什么不调用那三个方法"时，
    /// 方法名**不许写成带双引号的字面量** —— `core/command`、`config/dump`、`config/get` 这样可以，
    /// 把整条名字用双引号包住就不行。tests/rc_client.rs 的
    /// `the_three_forbidden_methods_never_appear_as_a_call_target` 扫的正是带引号的那种形状
    /// （方法名永远以字符串字面量传给 call / send / submit_async，那才是调用位置），
    /// 而裸词扫会把这种正当的说明文字判成缺陷。裁定 I4。
    pub async fn config_listremotes(&self) -> Result<Vec<String>, RcError> {
        let v = self.call("config/listremotes", &[]).await?;
        Ok(v.get("remotes").and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default())
    }

    /// 只有 `fs` + `remote` 两个参数的对象级方法共用这一条出口（实测）。
    /// 404 在这里只有一种含义：**那个名字不在**（F10：源不存在是 404；撞同名不是 404，是 200）。
    /// 500 一律走 `into_value` 那条口径响亮失败——`purge` 打在没了的前缀上是 500（F12），
    /// 那一档**绝不能**被读成"已经空了"。
    async fn object_call(&self, method: &str, fs: &str, remote: &str) -> Result<(), ListError> {
        let params = object_params(fs, remote);
        let res = self.send(method, &params).await.map_err(ListError::Other)?;
        if res.status == 404 { return Err(ListError::NotFound); }
        res.into_value(method).map_err(ListError::Other)?;
        Ok(())
    }

    /// `srcFs/srcRemote/dstFs/dstRemote` 四个参数，一个都不能少、也不能换名（F2 那族拼写错误：
    /// 把 `srcRemote` 写成别的，rc 静默丢弃，"复制这一个"就变成"复制整个远端"）。
    /// 因此这条出口的参数名由 tests/rc_client.rs 按**上线字节**逐字钉死，不按常量表钉。
    async fn two_sided_call(&self, method: &str, src: (&str, &str), dst: (&str, &str)) -> Result<(), ListError> {
        let params = [
            ("srcFs", src.0.to_string()), ("srcRemote", src.1.to_string()),
            ("dstFs", dst.0.to_string()), ("dstRemote", dst.1.to_string()),
        ];
        let res = self.send(method, &params).await.map_err(ListError::Other)?;
        if res.status == 404 {
            // 源不见了 = 界面上点中的那个东西已经没了（面板每秒刷一次，中间被人删掉是真会发生的）。
            return Err(ListError::NotFound);
        }
        res.into_value(method).map_err(ListError::Other)?;
        Ok(())
    }

    /// 单文件复制。**撞同名不会报错也不会拒绝**（F10：rc 回 200，然后按"大小 + modtime"
    /// 决定跳过还是盖掉），所以调用方必须在写之前自己做存在性检查（I2c）。
    pub async fn operations_copyfile(&self, src: (&str, &str), dst: (&str, &str)) -> Result<(), ListError> {
        self.two_sided_call("operations/copyfile", src, dst).await
    }

    pub async fn operations_movefile(&self, src: (&str, &str), dst: (&str, &str)) -> Result<(), ListError> {
        self.two_sided_call("operations/movefile", src, dst).await
    }

    pub async fn operations_deletefile(&self, fs: &str, remote: &str) -> Result<(), ListError> {
        self.object_call("operations/deletefile", fs, remote).await
    }

    /// 实测**幂等**：对已存在的目录再建一次还是 200。所以"建目录"不需要先问在不在。
    pub async fn operations_mkdir(&self, fs: &str, remote: &str) -> Result<(), ListError> {
        self.object_call("operations/mkdir", fs, remote).await
    }

    /// 删空目录。**只对空目录成功**（F14，rc 的保证），非空是 500 走 `into_value` 那条口径、
    /// 原文里带绝对路径——那是操作员自己的路径，可以给他看，但这里不改写、不猜测。
    pub async fn operations_rmdir(&self, fs: &str, remote: &str) -> Result<(), ListError> {
        self.object_call("operations/rmdir", fs, remote).await
    }

    /// 清空整个前缀，**异步提交**：同步那一档在本地 2000 个对象是 376 ms（F15），真桶上没有这个数，
    /// 而 `send` 的 30 s 超时会把"还在删"报成"失败了"。
    /// 调用方必须轮 `job_status`，并且**持锁的 guard 要活得比 job 长**（子项目 1 的 R33）。
    pub async fn operations_purge_async(&self, fs: &str, remote: &str) -> Result<SubmitAck, RcError> {
        let params = object_params(fs, remote);
        self.submit_async("operations/purge", &params, false).await
    }

    /// 问一个名字在不在——覆盖拒绝（I2c）唯一的依据，所以三种回答必须分得开：
    /// 200 + `item` 对象 = 在；200 + `item: null` = 不在；非 2xx = **不知道**（当错误抛出）。
    /// 把"不知道"读成"不在"，就等于在最关键的那一步上放行了一次写入。
    pub async fn operations_stat(&self, fs: &str, remote: &str) -> Result<Option<EntryMeta>, ListError> {
        let params = object_params(fs, remote);
        let res = self.send("operations/stat", &params).await.map_err(ListError::Other)?;
        let json = res.into_value("operations/stat").map_err(ListError::Other)?;
        match json.get("item") {
            None | Some(Value::Null) => Ok(None),
            Some(v) => serde_json::from_value::<EntryMeta>(v.clone()).map(Some).map_err(|e| {
                ListError::Other(RcError::Rc {
                    method: "operations/stat".into(),
                    message: format!("解析 stat 的 item 失败：{e}（不能当成「没有这个对象」）"),
                })
            }),
        }
    }

    /// **故意没有生产读者**（评审 I-5 那一格）：F16 量到它对前缀不作答——问 `remote="sub"`、
    /// 问一个根本不存在的前缀，反斜杠 / 正斜杠 / 具名 alias 三种本地 fs 形状回的都是**整棵树**的
    /// 字节数与个数，而且全是 HTTP 200，没有一个错误可接。面板这一侧"这一层有多少用量"来自
    /// `runner::browser::usage_of`（按已经列到的那一份求和），不走这里。
    /// 现在的读者只有 `tests/rc_client.rs` 那几处：它钉的是**上线的字节形状**，那仍是契约的一部分，
    /// 所以这个方法留着，删它等于删掉那份契约的证人。
    pub async fn operations_size(&self, fs: &str, remote: &str) -> Result<Sizes, ListError> {
        let params = object_params(fs, remote);
        let res = self.send("operations/size", &params).await.map_err(ListError::Other)?;
        if res.status == 404 { return Err(ListError::NotFound); }
        let json = res.into_value("operations/size").map_err(ListError::Other)?;
        serde_json::from_value(json).map_err(|e| ListError::Other(RcError::Rc {
            method: "operations/size".into(), message: format!("解析 size 失败：{e}（不能当成「0 个对象」）") }))
    }

    /// **这里没有"按 Range 读对象内容"的那条出口**：那条路需要 rclone 的 `operations/cat`
    /// （或 `get`/`download`/`open`/`read` 里的任一个），而本机 v1.75.1 的 rc 接口上这五个
    /// 方法都没注册（`couldn't find method`，两个独立 daemon 各问一次同答）。曾经写过一版
    /// `operations_cat_prefix`，界面上每一格都报"看不了"—— 要读云端对象内容只剩整份
    /// `copyto` 那一条，而那是另一件事。理由与实测记在 `runner::browser` 那一节。
    pub async fn operations_fsinfo(&self, fs: &str) -> Result<FsInfo, RcError> {
        let v = self.call("operations/fsinfo", &[("fs", fs.to_string())]).await?;
        serde_json::from_value(v).map_err(|e| RcError::Rc {
            method: "operations/fsinfo".into(), message: format!("解析 fsinfo 失败：{e}") })
    }

    pub async fn operations_check(&self, src: (&str, &str), dst: (&str, &str)) -> Result<CheckOutcome, RcError> {
        let params = [
            ("srcFs", src.0.to_string()), ("srcRemote", src.1.to_string()),
            ("dstFs", dst.0.to_string()), ("dstRemote", dst.1.to_string()),
        ];
        let v = self.call("operations/check", &params).await?;
        serde_json::from_value(v).map_err(|e| RcError::Rc {
            method: "operations/check".into(), message: format!("解析 check 失败：{e}") })
    }

    /// 所有请求的唯一出口：先过 `_config` 白名单，再发。
    async fn send(&self, method: &str, params: &[(&str, String)]) -> Result<RcResponse, RcError> {
        self.send_with_timeout(method, params, REQUEST_TIMEOUT).await
    }

    /// 与 [`Self::send`] 同一件事，只是上限由调用点给。列举那一档用它（`LIST_TIMEOUT`）：
    /// 唯一会把整个前缀翻一遍的调用，套全局那档会把一次"大但正常"的列举报成故障。
    /// 走 `RequestBuilder::timeout`（覆盖 client 上那个），不为此再建一个 client：
    /// 两个 client 就是两份连接池，而这里要区分的只是"等多久"。
    async fn send_with_timeout(
        &self,
        method: &str,
        params: &[(&str, String)],
        timeout: Duration,
    ) -> Result<RcResponse, RcError> {
        enforce_config_allowlist(method, params)?;
        let mut form = String::new();
        for (k, v) in params {
            if !form.is_empty() { form.push('&'); }
            form.push_str(&urlencoded(k));
            form.push('=');
            form.push_str(&urlencoded(v));
        }
        let res = self.http
            .post(format!("{}/{method}", self.base))
            .header("authorization", &self.auth)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form)
            .timeout(timeout)
            .send()
            .await
            .map_err(|e| if e.is_timeout() {
                RcError::Timeout { method: method.to_string(), budget_secs: timeout.as_secs() }
            } else {
                RcError::Transport(e.to_string())
            })?;
        let status = res.status().as_u16();
        let text = res.text().await.map_err(|e| RcError::Transport(e.to_string()))?;
        let json = serde_json::from_str(&text).unwrap_or(Value::String(text.clone()));
        Ok(RcResponse { status, json, text })
    }
}

/// `fs` + `remote` 这一对参数的**唯一**构造点，`object_call`、`operations_stat`、`operations_size`、
/// `operations_purge_async` 四个调用点共用它（实测：这四处上线的就是这两个名字，没有第三个）。
/// 为什么只留一处：每抄一份字面量，就多一处能**独立**打错的地方，而 rc 对不认识的字面量是
/// **静默丢弃**（实测 F2）——把 `remote` 写成 `remot`，`operations_stat` 不会报错，它只会去问
/// 那个远端的**根对象**，于是 I2c 的覆盖预检带着一份错的 Size/ModTime 放行一次写入。
/// 合并之后打错只可能发生在这一处，而这一处的上线字节由 tests/rc_client.rs 逐字钉在**每个调用点**上：
/// `deletefile`/`operations_stat`/`operations_size` 三条断言同一条期望串，
/// `mkdir`/`rmdir`/`purge` 各断自己那条全串（`purge` 后面多一段 `_async=true`）。
/// **合并的只是参数的构造，回答的口径没有合并**：`object_call` 与 `operations_size` 各自保留
/// 404 那一档，`operations_stat` 故意没有 404 那一档（实测 F13：按名字问不到是 200 + `item: null`）。
fn object_params(fs: &str, remote: &str) -> [(&'static str, String); 2] {
    [("fs", fs.to_string()), ("remote", remote.to_string())]
}

/// 任何要发出去的 `_config` 都必须在请求离开前过白名单。
/// 闸门放在 `send`（每条请求的出口）而不是 `submit_async` 里拼字面量的那一行，
/// 是因为 `call` 是公开出口、谁都能带一个 `_config` 进来，而 rclone 对未知键
/// fail-open、对非法 JSON 只给一个 body 里没有 jobid 的 400（均实测）。
/// 两种情况都在本地拒绝，一次请求都不发。
fn enforce_config_allowlist(method: &str, params: &[(&str, String)]) -> Result<(), RcError> {
    let rejected = |message: String| RcError::Rc { method: method.to_string(), message };
    for (key, value) in params {
        if *key != "_config" {
            continue;
        }
        let cfg: Value = serde_json::from_str(value)
            .map_err(|e| rejected(format!("_config 不是合法 JSON（rclone 只会回 400）：{e}")))?;
        let Some(map) = cfg.as_object() else {
            return Err(rejected(format!("_config 必须是一个 JSON 对象，当前是 {cfg}")));
        };
        for k in map.keys() {
            if !ALLOWED_CONFIG_KEYS.contains(&k.as_str()) {
                return Err(rejected(format!(
                    "_config 的键 {k} 不在白名单 {ALLOWED_CONFIG_KEYS:?} 内：\
                     未知键 rclone 会 fail-open（照样返回 success=true 并静默忽略），不能发"
                )));
            }
        }
    }
    Ok(())
}

/// application/x-www-form-urlencoded 的转义：只留 RFC 3986 的 unreserved 字符，
/// 其余按 %XX。路径里的 `/` 和 JSON 里的 `{` 都会被编掉（测试就是按编出来的字节断言的）。
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
