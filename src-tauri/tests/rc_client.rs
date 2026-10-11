use rclone_deck_lib::rc::listing::list_files_recursive;
use rclone_deck_lib::rc::{types::*, ListError, ListShape, RcClient, RcError};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(s: &MockServer) -> RcClient {
    RcClient::from_base_url(s.uri(), "u", "p")
}

/// 单层 LIST 的响应体。形状与 `rc_listing.rs` 那份夹具同一份（那边直接给 `ResponseTemplate`，
/// 这里只给 JSON 值，好复用本文件已有的 `mount_json`，不再多一个挂载口）。
fn list_resp(entries: &[(&str, bool)]) -> serde_json::Value {
    let list: Vec<serde_json::Value> = entries.iter().map(|(p, is_dir)| serde_json::json!({
        "Path": p,
        "Name": p.rsplit('/').next().unwrap_or(p),
        "Size": 1,
        "IsDir": is_dir,
    })).collect();
    serde_json::json!({ "list": list })
}

async fn mount_json(s: &MockServer, p: &str, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path(p))
        .and(header("authorization", "Basic dTpw"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(s)
        .await;
}

/// 挂一个**指定状态码** + 指定 JSON 体的响应。`mount_json` 只会回 200，而"非 2xx 怎么被读"
/// 正是 `deletefile_404_...` / `deletefile_500_...` 那一族证人的全部题目（它们写在
/// `// ---- 修复轮 1 · Finding 2` 那一节里），所以这里需要一个能给码的挂载口。
/// 不给 `mount_json` 加默认参数：那会改动 200 那一档的既有证人。
async fn mount_status_json(s: &MockServer, p: &str, status: u16, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path(p))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(s)
        .await;
}

/// 客户端实际发到某个端点上的 form 字节。参数名一类的契约只能在这里钉。
async fn body_sent_to(s: &MockServer, p: &str) -> String {
    let hit = s
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.url.path() == p)
        .unwrap_or_else(|| panic!("{p} 根本没被请求"));
    String::from_utf8_lossy(&hit.body).into_owned()
}

#[tokio::test]
async fn sends_basic_auth_and_form_body() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sync/copy"))
        .and(header("authorization", "Basic dTpw"))
        .and(body_string_contains("srcFs=aaa"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({"executeId": "e1", "jobid": 7})))
        .mount(&s).await;
    let ack = client(&s)
        .submit_async("sync/copy", &[("srcFs", "aaa".into()), ("dstFs", "bbb".into())], false)
        .await.unwrap();
    assert_eq!(ack.jobid, 7);
    assert_eq!(ack.execute_id, "e1");
}

#[tokio::test]
async fn dry_run_is_the_only_config_key_we_ever_send() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/sync/copy"))
        .and(body_string_contains("%7B%22dryRun%22%3Atrue%7D")) // urlencoded {"dryRun":true}
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({"executeId": "e1", "jobid": 1})))
        .expect(1)
        .mount(&s).await;
    client(&s).submit_async("sync/copy", &[("srcFs", "a".into())], true).await.unwrap();
}

// ---- 裁定 R35：`job/status` 的 `error` 是载荷，不是故障 ----
// 这四条是补来的证人。Task 3 原有的夹具里**没有 `error` 这个键**，所以对真 daemon 会全线塌掉的
// 那个形状，27 条测试一条都看不见；probe 读的是原始 JSON，也看不见。

/// 真 rclone 的**成功** `job/status`：HTTP 200、`finished=true`、`success=true`，
/// 而 `"error": ""` 实实在在地在载荷里（量到的键集是 10 个：
/// duration endTime error executeId finished group id output startTime success）。
/// 把这个当失败 = Task 9 每一次轮询都"读不到状态"，重试耗尽后进 `Unknown`——
/// 一次跑成功的 copy 会被界面报成"状态说不准"。
#[tokio::test]
async fn an_empty_error_field_on_a_successful_job_status_is_not_a_failure() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/job/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 1, "group": "1/2",
            "startTime": "2026-09-29T14:00:00+08:00",
            "endTime": "2026-09-29T14:00:01+08:00", "duration": 1.0,
            "success": true, "finished": true, "error": "",
            "executeId": "e1", "output": ""
        })))
        .mount(&s).await;
    let st = client(&s).job_status(1).await.expect("成功任务的 error 是空串，不是故障");
    assert!(st.finished && st.success, "{st:?}");
    assert_eq!(st.error, "", "{st:?}");
}

/// 任务**真失败**时：HTTP 200、`success=false`、`error` 是非空原因（实测 "directory not found"）。
/// 必须作为 `Ok(JobStatus)` 交回去，让 Task 9 映射成 `TaskState::Failed { error }`——
/// 当传输故障的话，操作员看到的是"说不准"，而不是"这个目录找不到"。
#[tokio::test]
async fn a_failed_job_status_comes_back_as_payload_not_as_a_transport_error() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/job/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 4, "group": "4/5", "duration": 0.2,
            "success": false, "finished": true, "error": "directory not found",
            "executeId": "e1", "output": ""
        })))
        .mount(&s).await;
    let st = client(&s).job_status(4).await.expect("失败的 job 也要读得回来");
    assert!(!st.success && st.finished, "{st:?}");
    assert_eq!(st.error, "directory not found", "{st:?}");
}

/// 反向半边：**别把放行做成通用规则**。非载荷方法（`core/stats`）带 200 + 非空 `error`
/// 仍然是故障——放行判据是"这个方法属于 error 即载荷的那一类"，不是"这条 error 很短/很空"。
/// 载荷体是**完整可解析**的 stats（外加一个 error 键）：若把 `error_is_payload` 写成通用放行，
/// `into_value` 会跳过折叠、`core_stats` 又能把这份体解析成功 → 这条就变红。
/// （brief 原来的体只有 `{"error": ...}`，那种体在 `from_value::<CoreStats>` 处永远解析失败，
///  于是无论怎么改 `error_is_payload` 它都是红以外的绿——不是活的证人；见 task-7-report 的"brief 缺陷"。）
#[tokio::test]
async fn a_wrapped_error_on_a_non_payload_method_still_fails() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/core/stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "error": "group not found",
            "bytes": 6, "checks": 1, "deletedDirs": 0, "deletes": 0, "elapsedTime": 0.26,
            "errors": 0, "eta": null, "fatalError": false, "listed": 2, "renames": 0,
            "retryError": false, "serverSideCopies": 0, "serverSideCopyBytes": 0,
            "serverSideMoveBytes": 0, "serverSideMoves": 0, "speed": 0.0, "totalBytes": 6,
            "totalChecks": 1, "totalTransfers": 1, "transferTime": 0.002, "transfers": 1
        })))
        .mount(&s).await;
    assert!(client(&s).core_stats("job/9").await.is_err(),
        "core/stats 的 200+error 必须仍算失败");
}

/// 真故障的形状仍然走状态码（500 + "job not found"），与上面两条区分开才成立。
#[tokio::test]
async fn a_genuinely_unknown_jobid_still_fails_by_status_code() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/job/status"))
        .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
            "error": "job not found", "path": "job/status", "status": 500
        })))
        .mount(&s).await;
    match client(&s).job_status(999).await {
        Err(RcError::Rc { message, .. }) => assert!(message.contains("job not found"), "{message}"),
        other => panic!("500 必须还是故障，实际 {other:?}"),
    }
}

#[tokio::test]
async fn rc_level_error_becomes_rcerror_rc() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/job/status"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "Didn't find key \"jobid\" in input", "path": "job/status", "status": 400 })))
        .mount(&s).await;
    match client(&s).job_status(1).await.unwrap_err() {
        RcError::Rc { message, .. } => assert!(message.contains("jobid"), "{message}"),
        other => panic!("expected Rc, got {other:?}"),
    }
}

#[tokio::test]
async fn transport_failure_becomes_transport_error() {
    // 端口上没人听：不能 panic，也不能报成"任务失败"
    let c = RcClient::from_base_url("http://127.0.0.1:1", "u", "p");
    match c.call("rc/noop", &[]).await.unwrap_err() {
        RcError::Transport(_) | RcError::Timeout { .. } => {}
        other => panic!("expected Transport/Timeout, got {other:?}"),
    }
}

#[tokio::test]
async fn missing_jobid_in_a_200_is_an_error_not_a_panic() {
    // 实测：_config 非法时 rclone 返回 400 且响应里没有 jobid。
    // 反过来 200 但没 jobid 也必须当失败，不能 unwrap。
    let s = MockServer::start().await;
    mount_json(&s, "/sync/copy", serde_json::json!({})).await;
    let err = client(&s).submit_async("sync/copy", &[("srcFs", "a".into())], false).await.unwrap_err();
    assert!(matches!(err, RcError::Rc { .. }), "{err:?}");
}

#[tokio::test]
async fn core_stats_parses_every_measured_field() {
    let s = MockServer::start().await;
    mount_json(&s, "/core/stats", serde_json::json!({
        "bytes": 6, "checks": 1, "deletedDirs": 0, "deletes": 0, "elapsedTime": 0.26,
        "errors": 0, "eta": null, "fatalError": false, "listed": 2, "renames": 0,
        "retryError": false, "serverSideCopies": 0, "serverSideCopyBytes": 0,
        "serverSideMoveBytes": 0, "serverSideMoves": 0, "speed": 0.0, "totalBytes": 6,
        "totalChecks": 1, "totalTransfers": 1, "transferTime": 0.002, "transfers": 1
    })).await;
    let st = client(&s).core_stats("job/1").await.unwrap();
    assert_eq!(st.listed, 2);
    assert_eq!(st.transfers, 1);

    // 这条断言就是 spec §9 的"界面不出现 listed"
    let json = serde_json::to_string(&st.view()).unwrap();
    assert!(!json.contains("listed"), "StatsView leaked listed: {json}");
    assert!(json.contains("\"transfers\":1"));
}

#[tokio::test]
async fn operations_list_maps_404_to_not_found() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/operations/list"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": "error in ListJSON: directory not found", "status": 404 })))
        .mount(&s).await;
    let err = client(&s).operations_list("r2:img", "nope", ListShape::Tree).await.unwrap_err();
    assert!(matches!(err, ListError::NotFound), "{err:?}");
}

#[tokio::test]
async fn operations_list_not_found_keys_on_the_status_not_the_wording() {
    // "directory not found" 只是本地后端今天的措辞。S3/R2 与新版 rclone 换个说法，
    // 按措辞匹配的写法就会悄悄退化成 Other(Rc)——Task 4 的 BFS 就把"远端没有这个前缀"
    // 当成一次普通失败来处理。实测的是状态码 404，认的就只能是状态码。
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/operations/list"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "error": "Bucket or key not found: ideahub/covers", "status": 404 })))
        .mount(&s).await;
    let err = client(&s).operations_list("r2:img", "ideahub/covers", ListShape::Tree).await.unwrap_err();
    assert!(matches!(err, ListError::NotFound), "{err:?}");
}

#[tokio::test]
async fn operations_list_rejects_dot_remote_before_sending() {
    // remote="." 会 404（实测）。这是调用方的错，在客户端就拦掉，
    // 不要浪费一次请求再去猜原因。
    let s = MockServer::start().await;
    match client(&s).operations_list("r2:img", ".", ListShape::Tree).await.unwrap_err() {
        ListError::Other(RcError::Rc { message, .. }) => assert!(message.contains('.'), "{message}"),
        other => panic!("expected a client-side rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn operations_list_strips_trailing_slash_before_sending() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/operations/list"))
        .and(body_string_contains("remote=a%2Fb"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"list": []})))
        .expect(1)
        .mount(&s).await;
    client(&s).operations_list("r2:img", "a/b/", ListShape::Tree).await.unwrap();
}

#[tokio::test]
async fn operations_list_parses_entries() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/list", serde_json::json!({"list": [
        {"Path": "ideahub/covers/ab.webp", "Name": "ab.webp", "Size": 12,
         "MimeType": "image/webp", "ModTime": "2026-09-28T23:00:00+08:00", "IsDir": false},
        {"Path": "ideahub/covers/sub", "Name": "sub", "Size": 0,
         "MimeType": "inode/directory", "ModTime": "2026-09-28T23:00:00+08:00", "IsDir": true}
    ]})).await;
    let list = client(&s).operations_list("r2:img", "ideahub/covers", ListShape::Tree).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].path, "ideahub/covers/ab.webp");
    assert!(list[1].is_dir);
}

#[tokio::test]
async fn core_version_reads_back_the_actual_binary() {
    let s = MockServer::start().await;
    mount_json(&s, "/core/version", serde_json::json!({"version": "v1.75.1"})).await;
    assert_eq!(client(&s).core_version().await.unwrap(), "v1.75.1");
}

// ---------------------------------------------------------------- 契约补钉
// 上面 11 条是计划给的。下面这些钉的是计划正文之外、实测确认过、而且
// "顺手改一改就会静默坏掉"的契约：_config 白名单（拦在发送之前）、
// 状态码先看再取 jobid、_config 归 dry_run 布尔管、参数名（jobid vs group 是反的）、
// 桶根的 remote=""、404 认状态码不认措辞、非 rc 的 HTTP 故障、只打回环、
// _config 的键名上线时必须是 rclone 的权威拼写、
// 凭据不进 URL 也不进错误话术、错误送进界面时是一句话、
// "LIST 失败"永远不许被读成"这个目录是空的"。

#[tokio::test]
async fn refuses_an_unknown_config_key_before_any_request() {
    // 实测：_config 里的未知键是 fail-open——rc 返回 success=true 并静默忽略它。
    // 一个 typo 就能让任务"成功"在错误设置下，所以拒绝必须发生在请求发出之前，
    // 而不是指望服务端报错。走 call()：那是绕过 submit_async 的原始出口。
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({"executeId": "e1", "jobid": 1})))
        .expect(0)
        .mount(&s).await;
    let err = client(&s)
        .call("sync/copy", &[("srcFs", "a".into()), ("_config", r#"{"exclude":["*.tmp"]}"#.into())])
        .await
        .unwrap_err();
    match err {
        RcError::Rc { message, .. } => {
            assert!(message.contains("exclude"), "错误要点名被拒的键：{message}");
            assert!(message.contains("dryRun"), "错误要说出允许的键：{message}");
        }
        other => panic!("expected Rc, got {other:?}"),
    }
    // expect(0) 要等 MockServer drop 才校验，这里当场确认：一个字节都没发出去
    assert!(s.received_requests().await.unwrap().is_empty(), "白名单必须拦在发送之前");
}

#[tokio::test]
async fn a_malformed_config_is_refused_before_any_request() {
    // 同上一条同一个理由，但走的是"根本不是一个对象"的形态：
    // rc 收到非法 _config 只会给一个 400（实测 body 里连 jobid 都没有），
    // 与其去猜那句 400，不如在本地就拒绝。
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(0)
        .mount(&s).await;
    for bad in ["not json at all", r#"[{"dryRun":true}]"#, "null"] {
        let err = client(&s)
            .call("sync/copy", &[("srcFs", "a".into()), ("_config", bad.into())])
            .await
            .unwrap_err();
        match err {
            RcError::Rc { message, .. } => {
                assert!(message.contains("_config"), "错误要说是 _config 的问题：{message}");
            }
            other => panic!("expected Rc for {bad:?}, got {other:?}"),
        }
    }
    assert!(s.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn submit_async_does_not_let_the_caller_smuggle_a_config() {
    // dry_run 是唯一的开关。params 里塞 _config 能把一次"真跑"偷偷变成干跑：
    // success=true、一个字节都没传，而这正是删除类流程最害怕的一种"成功"。
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({"executeId": "e1", "jobid": 1})))
        .expect(0)
        .mount(&s).await;
    let err = client(&s)
        .submit_async(
            "sync/copy",
            &[("srcFs", "a".into()), ("_config", r#"{"dryRun":true}"#.into())],
            false,
        )
        .await
        .unwrap_err();
    match err {
        RcError::Rc { message, .. } => {
            assert!(message.contains("dry_run"), "错误要指出正确的开关：{message}");
        }
        other => panic!("expected Rc, got {other:?}"),
    }
    assert!(s.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn submit_checks_http_status_before_reaching_for_jobid() {
    // 实测：_config 非法 JSON 时 rclone 直接 400，body 里没有 jobid。
    // 提交路径必须先认状态码、把 rc 自己的话术交出去，而不是掉进"没有 jobid"那条。
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/sync/copy"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "json: cannot unmarshal string into Go value of type map",
            "path": "sync/copy", "status": 400 })))
        .mount(&s).await;
    let err = client(&s)
        .submit_async("sync/copy", &[("srcFs", "a".into())], false)
        .await
        .unwrap_err();
    match err {
        RcError::Rc { message, .. } => {
            assert_eq!(message, "json: cannot unmarshal string into Go value of type map");
        }
        other => panic!("expected Rc carrying the server's words, got {other:?}"),
    }
}

#[tokio::test]
async fn a_real_run_sends_no_config_but_still_marks_async() {
    // dry_run=false 就是真跑：既不能偷偷带 dryRun（那样任务会"成功"但什么都没传），
    // 也不能丢 _async（丢了 submit 就会把整个传输同步阻塞在这一次 HTTP 调用上）。
    let s = MockServer::start().await;
    mount_json(&s, "/sync/copy", serde_json::json!({"executeId": "e1", "jobid": 3})).await;
    client(&s).submit_async("sync/copy", &[("srcFs", "a".into())], false).await.unwrap();
    let body = body_sent_to(&s, "/sync/copy").await;
    assert!(body.contains("_async=true"), "提交必须是异步的：{body}");
    assert!(!body.contains("_config"), "dry_run=false 不许带 _config：{body}");
}

#[tokio::test]
async fn job_status_sends_jobid_and_core_stats_sends_group() {
    // 实测两个端点的参数名是反的：job/status 只认 jobid（给它 group 会 400
    // Didn't find key "jobid" in input），core/stats 只认 group。
    // 两个名字都钉住，别让后来人把它们"统一"成一个。
    let s = MockServer::start().await;
    mount_json(&s, "/job/status",
        serde_json::json!({"id": 42, "group": "job/42", "finished": true, "success": true,
                           "duration": 0.5})).await;
    // core/stats 的字段解析已由 core_stats_parses_every_measured_field 覆盖；
    // 这条只关心发出去的参数名，响应给成错误即可。
    Mock::given(method("POST")).and(path("/core/stats"))
        .respond_with(ResponseTemplate::new(500).set_body_json(
            serde_json::json!({"error": "response not the point here", "status": 500})))
        .mount(&s).await;

    let c = client(&s);
    // 写出类型不是为了好看：JobStatus 就是 Task 7 轮询要拿到的那个形状。
    let st: JobStatus = c.job_status(42).await.unwrap();
    assert_eq!(st.group, "job/42");
    assert!(st.finished && st.success);
    assert!(c.core_stats("job/42").await.is_err(), "core/stats 的响应不是这条要测的");

    let js = body_sent_to(&s, "/job/status").await;
    assert!(js.contains("jobid=42"), "job/status 必须发 jobid=，实际：{js}");
    assert!(!js.contains("group="), "job/status 不接受 group（实测 400），实际：{js}");
    let cs = body_sent_to(&s, "/core/stats").await;
    assert!(cs.contains("group=job%2F42"), "core/stats 的参数是 group=，实际：{cs}");
    assert!(!cs.contains("jobid="), "core/stats 不是 job/status，实际：{cs}");
}

#[tokio::test]
async fn operations_list_sends_an_empty_remote_for_the_bucket_root() {
    // remote="" 才是桶根，"." 是 404。这条钉住别把"根"翻译成 "."，
    // 也顺便把发出去的 form 字节整体定死（fs 与 remote 怎么切不能偷偷变）。
    // 尾上那串是整棵树那一档的 form 编码（`opt={"recurse":true,"filesOnly":…}`）：它是"一次
    // 调用拿整棵树的文件"这条通路的全部依赖，量过的两格是探针 §4b（递归键的拼法）与 §4d
    // （三个真开关 + 一组假名对照）。写死完整字节，是因为键名少一个字母、或者容器键写成复数
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/operations/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"list": []})))
        .mount(&s).await;
    client(&s).operations_list("r2:img", "", ListShape::Tree).await.unwrap();
    assert_eq!(
        body_sent_to(&s, "/operations/list").await,
        "fs=r2%3Aimg&remote=&opt=%7B%22recurse%22%3Atrue%2C%22filesOnly%22%3Atrue%2C%22noModTime%22%3Atrue%2C%22noMimeType%22%3Atrue%7D");
}

/// 单层那一档发的是"只关 metadata"那一串：既不是整棵树那一串，也**不是什么都不发**。
/// 这是上面那条的对照组，两档各守一半边：
/// * `recurse` 不许出现在这一档 —— 进了这一层只拿这一层，浏览器一屏要的是目录加文件；
/// * `filesOnly` 也不许出现 —— 目录行是"能不能点进去"的唯一来源，把它关掉浏览器就没有文件夹；
/// * `noModTime`/`noMimeType` 则**必须**在：`ListEntry` 只有 Path/Name/Size/IsDir 四个字段，
///   这两个值在反序列化那一层就掉了，而在对象存储上要 rclone 去凑齐它们，代价是逐对象确认。
///   以前这一档"一个 opt 都不发"，理由是"大小与目录都要显示"——`Size` 是列举本身给的，
///   那个理由撑不住，撑得住的只有前面那两句。
/// 把两档合成一串、或在这一档顺手加上 `filesOnly`，这条红；整串删掉也红（回到逐对象那一档）。
#[tokio::test]
async fn operations_list_for_one_level_keeps_dirs_but_drops_metadata() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/operations/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"list": []})))
        .mount(&s).await;
    client(&s).operations_list("r2:img", "", ListShape::Level).await.unwrap();
    let body = body_sent_to(&s, "/operations/list").await;
    // 顺序是有意的：两条"不许有"排在钉字节之前，这样每一档破坏都能**因自己的理由**变红
    // （有人把整棵树那一串挪过来 → 先红在 `recurse`；有人顺手加上 `filesOnly` → 先红在目录那半）。
    assert!(!body.contains("recurse"), "单层那一档不许带递归开关：{body}");
    assert!(!body.contains("filesOnly"), "单层那一档不许把目录行关掉：{body}");
    assert_eq!(body, "fs=r2%3Aimg&remote=&opt=%7B%22noModTime%22%3Atrue%2C%22noMimeType%22%3Atrue%7D",
        "单层那一档的字节不是这一串：{body}");
}

/// 每一个动作列的范围，必须等于它实际动的范围。
///
/// 判据是结构上的：把 `runner/browser.rs` 按方法切开，逐个方法看它用的是哪一档列举——
/// `layer_files`（这一层）还是 `prefix_files`（整个前缀），以及两个包装各自把 `recurse`
/// 定死成了什么。找固定串不算判据：把 `false` 改成 `true` 而句子不动，那种断言一条都不会红。
///
/// 为什么这条值钱：`operations/purge` 删的是整棵前缀，`deletefile` 删的是名单里那几个。
/// 拿单层那份清单去当 purge 的"会删多少"，界面上报的数比真删的数小 —— 而这个面上
/// 最贵的一种说错话就是报小。反过来，删除那一侧若用递归，"看到的 N 个"就悄悄变成整棵子树。
#[test]
fn each_browser_action_lists_the_range_it_actually_touches() {
    let read = |rel: &str| -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("读不到 {rel}（{path:?}）：{e}"))
    };
    // 按 4 空格缩进的方法头把源文本切成 `名字 -> 方法体（到下一个方法头之前）`。
    // 缩进更深的那些行也会命中 `"\n    "`，所以按"这一行是不是以 `fn` 开头"再筛一次。
    let methods = |src: &str| -> Vec<(String, String)> {
        let mut heads: Vec<(usize, String)> = Vec::new();
        for (at, _) in src.match_indices("\n    ") {
            let start = at + "\n    ".len();
            let end_of_line = src[start..].find('\n').map(|j| start + j).unwrap_or(src.len());
            let line = src[start..end_of_line].trim_end();
            let is_head = ["pub async fn ", "pub(crate) async fn ", "async fn ", "pub fn ", "pub(crate) fn ", "fn "]
                .iter().any(|p| line.starts_with(p));
            if !is_head { continue; }
            let Some(f) = line.find("fn ") else { continue };
            let name = line[f + "fn ".len()..].split('(').next().unwrap_or("").trim().to_string();
            if !name.is_empty() { heads.push((start, name)); }
        }
        heads.iter().enumerate().map(|(k, (at, name))| {
            let end = heads.get(k + 1).map(|(next, _)| *next).unwrap_or(src.len());
            (name.clone(), src[*at..end].to_string())
        }).collect()
    };
    let body = |ms: &[(String, String)], name: &str| -> String {
        ms.iter().find(|(n, _)| n == name).map(|(_, b)| b.clone())
            .unwrap_or_else(|| panic!("browser.rs 里找不到方法 {name}——改名了就把这条证人一起改"))
    };

    let browser = read("src-tauri/src/runner/browser.rs");
    let ms = methods(&browser);

    // 两个包装各自把 recurse 定死；这是全部判据的根。
    assert!(body(&ms, "layer_files").contains("listed_files(fs, prefix, ListShape::Level)"),
        "layer_files 不再是单层：{}", body(&ms, "layer_files"));
    assert!(body(&ms, "prefix_files").contains("listed_files(fs, prefix, ListShape::Tree)"),
        "prefix_files 不再是整棵前缀：{}", body(&ms, "prefix_files"));
    // 显示给人看的那一份也是单层。
    assert!(body(&ms, "browser_list").contains("operations_list(fs, prefix, ListShape::Level)"),
        "浏览器显示的不是单层：{}", body(&ms, "browser_list"));

    // 删除那一族用单层：「看到的 N 个」就是这一层的 N 个。
    for name in ["browser_preview_delete", "run_delete_visible"] {
        let b = body(&ms, name);
        assert!(b.contains("layer_files("), "{name} 不再列这一层：{b}");
        assert!(!b.contains("prefix_files("), "{name} 混进了整棵前缀的清单（范围会比按钮上说的大）：{b}");
    }
    // 清空那一族用整棵前缀：purge 删的就是整棵，报小的数就是假读数。
    for name in ["browser_preview_purge", "run_purge"] {
        let b = body(&ms, name);
        assert!(b.contains("prefix_files("), "{name} 不再列整棵前缀（会删的比报的多）：{b}");
        assert!(!b.contains("layer_files("), "{name} 混进了单层清单：{b}");
    }

    // 集合差/盘点那一侧仍然要整棵树：它要的是一份完整快照，不是一层。
    let listing = read("src-tauri/src/rc/listing.rs");
    assert!(listing.contains("operations_list(fs, remote, ListShape::Tree)"),
        "集合差/盘点不再列整棵树了：{listing}");
}

#[tokio::test]
async fn operations_list_a_200_wrapping_an_error_is_a_failure_not_an_empty_dir() {
    // 这个文件里唯一的失败口径是 RcResponse::into_value，它对"rc 用 200 包一个 error"
    // （版本差异，实测存在）有专门的分支。operations_list 不能是绕过它的那个方法：
    // 一旦绕过，这个响应就变成 Ok(vec![])——对 Task 4 的 BFS 而言
    // "LIST 调用失败了"和"这个目录下没有东西"就成了同一个值。
    // 后果不是措辞问题：一个列举失败的目录会被静默跳过，任务照样 success=true，
    // 门禁报"没有缺图"、清理报"没有孤儿"，而真实情况是它压根没看见——
    // 正是 spec 的 fail-closed 规则要防的那类静默少干活。
    let s = MockServer::start().await;
    mount_json(&s, "/operations/list",
        serde_json::json!({"error": "some rclone-side failure"})).await;
    let err = client(&s).operations_list("r2:img", "ideahub/covers", ListShape::Tree).await.unwrap_err();
    match err {
        ListError::Other(RcError::Rc { method, message }) => {
            assert_eq!(method, "operations/list", "错误要点名是哪个调用坏了");
            assert_eq!(message, "some rclone-side failure", "要带上服务端的原话");
        }
        other => panic!("expected Other(Rc) carrying the server's words, got {other:?}"),
    }
}

#[tokio::test]
async fn operations_list_a_200_without_a_list_key_is_a_failure_not_an_empty_dir() {
    // 缺 list 键同样不能当成"这里没有东西"。同上一条一个理由，只是坏在形状上：
    // 一个键名改了、或响应根本不是你期待的那个端点的响应，都必须是一次 Err。
    let s = MockServer::start().await;
    mount_json(&s, "/operations/list", serde_json::json!({"unexpected": "shape"})).await;
    let err = client(&s).operations_list("r2:img", "ideahub/covers", ListShape::Tree).await.unwrap_err();
    match err {
        ListError::Other(RcError::Rc { method, message }) => {
            assert_eq!(method, "operations/list");
            assert!(message.contains("list"), "错误要说出缺的是哪个键：{message}");
            assert!(message.contains("unexpected"), "错误要带上那份对不上形状的实际响应：{message}");
        }
        other => panic!("expected Other(Rc), got {other:?}"),
    }
}

#[tokio::test]
async fn operations_list_a_200_whose_list_is_not_an_array_is_a_failure() {
    // list 是字符串或对象时，"解析不出来"不等于"空目录"。
    // 错误里要留下实际收到的东西，否则排查只能猜。
    for shape in [
        serde_json::json!("not an array"),
        serde_json::json!({"0": {"Path": "ideahub/covers/ab.webp"}}),
    ] {
        let s = MockServer::start().await;
        mount_json(&s, "/operations/list", serde_json::json!({"list": shape})).await;
        let err = client(&s).operations_list("r2:img", "ideahub/covers", ListShape::Tree).await.unwrap_err();
        match err {
            ListError::Other(RcError::Rc { method, message }) => {
                assert_eq!(method, "operations/list");
                assert!(message.contains("list"), "错误要说出坏的是 list：{message}");
                assert!(message.contains(&shape.to_string()),
                    "错误要带上 list 实际的值：{message}");
            }
            other => panic!("expected Other(Rc) for {shape}, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn http_failure_without_an_rc_error_body_becomes_http() {
    // rc 的错误长得很规矩（JSON 里带 error）。不带 error 的 4xx/5xx 就不是 rc 在说话，
    // 而是端口上挂着别的东西 / 代理出错——这种情况把状态码和原始 body 交出去，
    // 不要伪装成"rc 报了一个错"。
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/job/status"))
        .respond_with(ResponseTemplate::new(500).set_body_bytes(
            "<html>something that is not rclone</html>"))
        .mount(&s).await;
    match client(&s).job_status(1).await.unwrap_err() {
        RcError::Http { status, body, .. } => {
            assert_eq!(status, 500);
            assert!(body.contains("not rclone"), "{body}");
        }
        other => panic!("expected Http, got {other:?}"),
    }
}

#[tokio::test]
async fn credentials_stay_out_of_the_url_and_the_error_text() {
    // 凭据只进 Basic 头。一旦有人把它拼进 base_url，reqwest 的错误话术就会把
    // URL 连密码一起送到日志/前端界面上。这里连 Display 带 Debug 一起验。
    let c = RcClient::from_base_url("http://127.0.0.1:1", "rcuser", "s3cret-pass");
    let err = c.call("rc/noop", &[]).await.unwrap_err();
    let text = format!("{err} // {err:?}");
    assert!(!text.contains("s3cret-pass"), "错误话术泄露了 rcd 密码：{text}");
    assert!(!text.contains("rcuser"), "错误话术泄露了 rcd 用户名：{text}");
}

#[tokio::test]
async fn the_production_constructor_dials_only_the_loopback() {
    // rc 只绑回环上它自己那一个号，--rc-no-auth 是禁区——所以客户端能连的地址
    // 也必须只有回环。reqwest 的失败话术里带着那个 URL，正好拿来看一眼。
    let err = RcClient::new(1, "u", "p").call("rc/noop", &[]).await.unwrap_err();
    let text = format!("{err}");
    assert!(text.contains("http://127.0.0.1:1/rc/noop"), "new() 必须打回环，实际：{text}");
}

#[tokio::test]
async fn errors_reach_the_ui_as_their_display_string() {
    // tauri 的 invoke 只能把一个 Err 送成字符串，这两个 Serialize impl 就是那条通道：
    // 界面看到的必须是一句话，不是 {"Rc":{"method":…,"message":…}} 这种结构体转储。
    let rc = RcError::Rc { method: "sync/copy".into(), message: "boom".into() };
    assert_eq!(serde_json::to_string(&rc).unwrap(), "\"rc 调用 sync/copy 失败：boom\"");

    let http = RcError::Http { status: 502, method: "job/status".into(), body: "bad gateway".into() };
    assert!(serde_json::to_string(&http).unwrap().contains("HTTP 502"), "{http}");

    // `Invariant` 走的是 `#[error("{0}")]` 的直通道：界面要拿到那句人话本身，
    // 不是 `Invariant("...")` 这种 Debug 形状（R56 第 2 条同一族）。
    let inv = serde_json::to_string(&ListError::Invariant("坐标系对不上".into())).unwrap();
    assert_eq!(inv, "\"坐标系对不上\"", "拒绝出结论的那句话要原样到得了界面：{inv}");
}

/// 把客户端自己编的那一段 x-www-form-urlencoded 解回来。只解 `%XX`：
/// `urlencoded()` 保留 RFC 3986 的 unreserved 字符、其余一律 `%XX`，
/// 所以 `&` 一定是编掉的，按 `&` 拆参数是安全的（不存在未编码的分隔符）。
fn pct_decode(s: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(s.len());
    let mut it = s.bytes();
    while let Some(b) = it.next() {
        if b != b'%' {
            out.push(b);
            continue;
        }
        let hi = it.next().and_then(|c| (c as char).to_digit(16));
        let lo = it.next().and_then(|c| (c as char).to_digit(16));
        // 残缺的转义原样带着走，让它去把 JSON 解析弄失败：这条要的是
        // "上线的字节长什么样"，不是一个宽容的解码器。
        match (hi, lo) {
            (Some(h), Some(l)) => out.push((h * 16 + l) as u8),
            _ => out.extend_from_slice(b"%"),
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `_config` 的键名契约。白名单 `ALLOWED_CONFIG_KEYS` 是**逐字节**比较的
/// （`ALLOWED_CONFIG_KEYS.contains(&k.as_str())`），而 rclone 那侧是 Go 的 encoding/json
/// 往 `fs.Config` 上落：匹配对大小写宽松，对**分隔符**不宽松——`dry_run` 落不进 `DryRun`
/// 字段，而未知键是 fail-open（实测：success=true 加静默忽略，见 `npm run probe:rc`）。
/// 于是把白名单里的那一条连发送的字面量一起改写成 `dry_run`，客户端自己**看不出问题**
/// （两份拼写自洽），rclone 却把它当一个不认识的键丢掉：一次干跑被报成真跑，
/// 而这正是删除类流程最害怕的那种"成功"。所以钉的是**上线的字节**等于 rclone 的权威
/// 拼写（lowerCamelCase，无分隔符），不是常量表自己长得自洽。
/// 破坏方式（必须变红）：`src-tauri/src/rc/mod.rs` 里把 `submit_async` 发的
/// `json!({ "dryRun": true })` 连 `ALLOWED_CONFIG_KEYS` 一起改成 `dry_run`。
/// 加一个允许进 `_config` 的键，要同时改 `ALLOWED_CONFIG_KEYS` 和这里的期望值。
/// 期望那一份现在是**集合**（顺序无关）：`serde_json::Map` 走 `BTreeMap` 迭代序，
/// 按顺序写的 `Vec` 期望会在加第二条键时先自己红一次，而那不是回归。
#[tokio::test]
async fn every_allowed_config_key_arrives_on_the_wire_in_camel_case() {
    let s = MockServer::start().await;
    mount_json(&s, "/sync/copy", serde_json::json!({"executeId": "e1", "jobid": 1})).await;
    client(&s).submit_async("sync/copy", &[("srcFs", "a".into())], true).await.unwrap();
    let body = body_sent_to(&s, "/sync/copy").await;

    let encoded = body
        .split('&')
        .find_map(|part| part.strip_prefix("_config="))
        .unwrap_or_else(|| panic!("dry_run=true 的提交里没有一个 _config 参数：{body}"));
    let cfg: serde_json::Value = serde_json::from_str(&pct_decode(encoded))
        .unwrap_or_else(|e| panic!("_config 上线时不是一个 JSON 对象：{encoded}（{e}）"));
    let keys: Vec<&str> = cfg
        .as_object()
        .unwrap_or_else(|| panic!("_config 上线时不是一个对象：{cfg}"))
        .keys()
        .map(String::as_str)
        .collect();

    for k in &keys {
        // 首字母小写 + 只有字母数字：`dry_run`（带分隔符，rclone 会静默丢掉）和
        // `DryRun`（今天能靠 Go 的大小写宽松落上，但它不是权威拼写）都过不了这一关。
        let canonical = !k.is_empty()
            && k.chars().next().unwrap().is_ascii_lowercase()
            && k.chars().all(|c| c.is_ascii_alphanumeric());
        assert!(
            canonical,
            "_config 的键 {k:?} 不是 rclone 的权威拼写（lowerCamelCase、无分隔符）：{body}"
        );
    }
    // 顺序无关的集合比较：`serde_json::Map` 的迭代序今天只有一个键所以看不出来，但它是
    // `BTreeMap` 序——白名单加第二条键时，按顺序写的期望值会"测试自己先红"。
    let got: std::collections::BTreeSet<&str> = keys.iter().copied().collect();
    let want: std::collections::BTreeSet<&str> = ["dryRun"].into_iter().collect();
    assert_eq!(got, want, "上线的 _config 键集合必须正好是白名单那一份：{body}");
}

// ---- Task 9 Step 0：LIST 计数器（日志里那个 `list_requests` 的来源）----

/// 计数器数的是**真发出去的** LIST：整棵树一次调用，所以这里必须正好数到 1。
/// 这条同时也是"列举没有退回逐层下探"的哨兵——变成 2 就意味着有人在按目录发请求。
#[tokio::test]
async fn list_request_count_is_one_call_for_the_whole_tree() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/list", list_resp(&[("a", false), ("sub", true)])).await;
    let c = client(&s);
    assert_eq!(c.list_request_count(), 0);
    list_files_recursive(&c, "r2:img", "ideahub").await.unwrap();
    assert_eq!(c.list_request_count(), 1, "整棵树该是一次 LIST，多出来的就是逐层下探回来了");
}

/// 客户端侧就掐掉的调用不许计数：`remote == "."` 根本没发请求，
/// 计了就会让日志里的 LIST 数比实际发出去的多。
#[tokio::test]
async fn list_request_count_ignores_calls_rejected_before_sending() {
    let s = MockServer::start().await;
    let c = client(&s);
    assert!(c.operations_list("r2:img", ".", ListShape::Tree).await.is_err());
    assert_eq!(c.list_request_count(), 0);
}

// ---- Task 6：浏览器（Task 8–13）要用的对象级具名方法 ----
// 参数名逐字钉死。理由在 spec §2 的 F2：rc 对不认识的参数**静默丢弃**，
// 拼错 `srcRemote` 不会报错，只会把"复制这一个文件"变成"复制整个远端"。
// 所以断言的对象是上线的字节，不是"我们代码里写了这个名字"。

/// `fs` + `remote` 这一档上线字节的**唯一**期望值。钉它的三条证人：
/// `deletefile_sends_fs_and_remote_and_nothing_else`、`stat_sends_the_same_two_names_as_deletefile`、
/// `size_sends_the_same_two_names_as_deletefile`。之所以共用一条串而不是各抄一条：这三个方法在
/// `rc/mod.rs` 里共用同一个构造点（`object_params`），期望值跟着分成三份，就等于允许实现分成三份。
/// 另外三个也走 `object_params` 的调用点没并进来，因为它们发的字节不同：
/// `mkdir_and_rmdir_send_fs_and_remote` 的 `remote` 是 `dir`，
/// `purge_is_submitted_async_so_a_long_delete_does_not_race_the_http_timeout` 尾部多一段 `_async=true`。
const FS_AND_REMOTE_BODY: &str = "fs=deckb%3A&remote=dir%2Fa.webp";

/// F9/F12 实测原文：远端名不在配置里时 rc 回 **500 且点名**。
/// `deletefile_500_...`、`copyfile_500_...`、`size_500_...` 三条证人共用这一份夹具体，
/// 断言只看核心短语 `didn't find section`——括号里那个名字是 rc 拼的，`nope` 是夹具给的远端名。
const UNKNOWN_REMOTE_ERROR: &str = "didn't find section in config file (\"nope\")";

#[tokio::test]
async fn copyfile_sends_exactly_the_documented_four_parameters() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/copyfile", serde_json::json!({})).await;
    client(&s).operations_copyfile(("decka:", "pick.webp"), ("deckb:", "in/pick.webp")).await.unwrap();
    assert_eq!(
        body_sent_to(&s, "/operations/copyfile").await,
        "srcFs=decka%3A&srcRemote=pick.webp&dstFs=deckb%3A&dstRemote=in%2Fpick.webp");
}

#[tokio::test]
async fn movefile_sends_the_same_four_names_as_copyfile() {
    // 两个方法共用一条构造路径，所以共用的那四个名字必须各自被钉住：
    // 只钉 copyfile 的话，把 movefile 拼成 sync/move 那套参数（`srcRemote` 写成别的）不会红。
    let s = MockServer::start().await;
    mount_json(&s, "/operations/movefile", serde_json::json!({})).await;
    client(&s).operations_movefile(("decka:", "a.webp"), ("deckb:", "box/a.webp")).await.unwrap();
    assert_eq!(
        body_sent_to(&s, "/operations/movefile").await,
        "srcFs=decka%3A&srcRemote=a.webp&dstFs=deckb%3A&dstRemote=box%2Fa.webp");
}

#[tokio::test]
async fn deletefile_sends_fs_and_remote_and_nothing_else() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/deletefile", serde_json::json!({})).await;
    client(&s).operations_deletefile("deckb:", "dir/a.webp").await.unwrap();
    assert_eq!(body_sent_to(&s, "/operations/deletefile").await, FS_AND_REMOTE_BODY);
}

#[tokio::test]
async fn mkdir_and_rmdir_send_fs_and_remote() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/mkdir", serde_json::json!({})).await;
    mount_json(&s, "/operations/rmdir", serde_json::json!({})).await;
    client(&s).operations_mkdir("deckb:", "dir").await.unwrap();
    client(&s).operations_rmdir("deckb:", "dir").await.unwrap();
    assert_eq!(body_sent_to(&s, "/operations/mkdir").await, "fs=deckb%3A&remote=dir");
    assert_eq!(body_sent_to(&s, "/operations/rmdir").await, "fs=deckb%3A&remote=dir");
}

/// `operations_stat` 与 `operations_size` 是当时两个**自建 `params`** 的读取方法，所以两个名字
/// 各钉一次，不靠 `deletefile` 那一条替它们说话。缺这一格的后果不是"少一条测试"：`remote` 拼成
/// `remot` 时 rc 静默丢弃它（F2），`stat` 于是回答那个远端的**根对象**而不是报错，
/// 而 I2c 的覆盖预检正是拿这个答案决定放不放行这次写——Size 与 ModTime 都是错的。
/// 响应侧由 `stat_reads_the_measured_wire_fields_including_mod_time` 钉，这一条钉的是上线的字节。
#[tokio::test]
async fn stat_sends_the_same_two_names_as_deletefile() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/stat", serde_json::json!({"item": {
        "Path": "dir/a.webp", "Name": "a.webp", "Size": 4242, "IsDir": false,
        "ModTime": "2026-10-01T18:10:48.4796855+08:00" }})).await;
    let st = client(&s).operations_stat("deckb:", "dir/a.webp").await.unwrap();
    assert!(st.is_some(), "先证这一趟往返真的走完了（`item` 读回来一个对象），字节断言才有的放矢：{st:?}");
    assert_eq!(body_sent_to(&s, "/operations/stat").await, FS_AND_REMOTE_BODY);
}

/// 同上一条的钉法，落在 `operations_size` 上。`size_and_check_read_the_measured_keys_...` 读的是
/// 回来的字段，这一条读的是上线的字节——两面各一次，缺哪一面都留一格缝。
#[tokio::test]
async fn size_sends_the_same_two_names_as_deletefile() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/size", serde_json::json!({"bytes": 7, "count": 3, "sizeless": 0})).await;
    let z = client(&s).operations_size("deckb:", "dir/a.webp").await.unwrap();
    assert_eq!((z.bytes, z.count), (7, 3));
    assert_eq!(body_sent_to(&s, "/operations/size").await, FS_AND_REMOTE_BODY);
}

#[tokio::test]
async fn purge_is_submitted_async_so_a_long_delete_does_not_race_the_http_timeout() {
    // F15：2000 个对象本地同步 376 ms，但真桶上没有这个数；`send` 的 30 s 超时会把
    // "还在删"报成"失败了"。异步 + 轮询才是这条通路的形状。
    let s = MockServer::start().await;
    mount_json(&s, "/operations/purge", serde_json::json!({"executeId": "e1", "jobid": 9})).await;
    let ack = client(&s).operations_purge_async("deckb:", "dir").await.unwrap();
    assert_eq!((ack.jobid, ack.execute_id.as_str()), (9, "e1"));
    assert_eq!(body_sent_to(&s, "/operations/purge").await, "fs=deckb%3A&remote=dir&_async=true");
}

#[tokio::test]
async fn listremotes_is_the_only_config_method_the_client_can_call() {
    let s = MockServer::start().await;
    // 实测形状：名字**不带冒号**（`{"remotes":["shapea","shapeb"]}`），拼 `fs` 时要自己补。
    mount_json(&s, "/config/listremotes", serde_json::json!({"remotes": ["decka", "deckb"]})).await;
    assert_eq!(client(&s).config_listremotes().await.unwrap(), vec!["decka".to_string(), "deckb".to_string()]);
}

#[tokio::test]
async fn stat_reads_the_measured_wire_fields_including_mod_time() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/stat", serde_json::json!({"item": {
        "Path": "a.webp", "Name": "a.webp", "Size": 4242, "MimeType": "image/webp",
        "ModTime": "2026-10-01T18:10:48.4796855+08:00", "IsDir": false }})).await;
    let st = client(&s).operations_stat("deckb:", "a.webp").await.unwrap().expect("item 在那里");
    assert_eq!((st.name.as_str(), st.size, st.is_dir), ("a.webp", 4242, false));
    // `mod_time` 现在是 Option（键缺就是 `Unknown` 那一档，见 `prune::Verdict`），
    // 所以这里读的是"拿到的那个值里有没有这一天"，而不是"字段在不在"。
    assert!(st.mod_time.as_deref().unwrap_or_default().contains("2026-10-01"),
        "撞名拒绝的话术要说'改于何时'：{:?}", st.mod_time);
    // 键整个缺掉时必须是 None，绝不能被读成空串——空串会让 `newer_of` 以为"时间是空字符串"，
    // 于是每一对都被判成"云端更新"，交出一份全错的清单。
    let no_time = MockServer::start().await;
    mount_json(&no_time, "/operations/stat", serde_json::json!({"item": {
        "Path": "a.webp", "Name": "a.webp", "Size": 4242, "IsDir": false }})).await;
    let st2 = client(&no_time).operations_stat("deckb:", "a.webp").await.unwrap().expect("item 在那里");
    assert!(st2.mod_time.is_none(), "没有 ModTime 这一键时是 None，不是空串：{:?}", st2.mod_time);
}

#[tokio::test]
async fn a_missing_object_is_none_while_a_broken_remote_is_still_an_error() {
    // F13：不存在是 **200 + item:null**；F9：远端名打错是 **500 且点名**。
    // 把后者也读成"不存在"，就等于在最关键的那一步（I2c 的存在性预检）上放行了——
    // 一个配置错误会变成一个"可以放心写"的答案。
    let s = MockServer::start().await;
    mount_json(&s, "/operations/stat", serde_json::json!({"item": null})).await;
    assert!(client(&s).operations_stat("deckb:", "gone.webp").await.unwrap().is_none());

    let bad = MockServer::start().await;
    Mock::given(method("POST")).and(path("/operations/stat"))
        .respond_with(ResponseTemplate::new(500).set_body_json(
            serde_json::json!({"error": "didn't find section in config file (\"nope\")"})))
        .mount(&bad).await;
    let err = client(&bad).operations_stat("nope:", "a.webp").await.unwrap_err();
    assert!(err.to_string().contains("didn't find section"), "500 必须响亮失败并点名：{err}");
    assert!(!err.to_string().contains("object not found"), "{err}");
}

// ---- 修复轮 1 · Finding 2：写路径上的 404 与 500 是两个答案 ----
// 这一族钉的是 `object_call` / `two_sided_call` / `operations_size` 里那一行状态码为 404 时
// 返回 `ListError::NotFound` 的分支。补钉之前没有任何夹具给 `deletefile`/`copyfile`/`movefile`/
// `rmdir`/`mkdir`/`size` 挂过 404 或 500，所以那一行写成"400 及以上都算 NotFound"时整套件全绿。
// 危害是具体的：`NotFound` 在界面上就是"那一侧没有东西"，而"配置里的远端名打错了"回的是 **500 且点名**
// （实测 F9/F12）——一个配置错误于是变成一个"可以放心写"的答案（I2c 的预检就是这么被放行的）。
// 判据只认状态码，不认 rclone 的英文（见 `rc/mod.rs` 里 `operations_list` 那条注释）：
// 所以下面每条夹具里的 body 都只是道具，它不许影响读法。
// `operations_stat` 不在这一族里：它**故意没有** 404 那一档（实测 F13：按名字问不到是 200 + item:null），
// 非 2xx 一律响亮失败那一格由 `a_missing_object_is_none_while_a_broken_remote_is_still_an_error` 钉着。

#[tokio::test]
async fn deletefile_404_is_our_not_found_not_rclones_words() {
    let s = MockServer::start().await;
    // body 用 F10 实测到的那句（源不存在 → 404 `object not found`），但读法由状态码决定。
    mount_status_json(&s, "/operations/deletefile", 404,
        serde_json::json!({"error": "object not found"})).await;
    let err = client(&s).operations_deletefile("deckb:", "dir/a.webp").await.unwrap_err();
    assert!(matches!(err, ListError::NotFound), "404 必须是 NotFound：{err:?}");
    assert!(!err.to_string().contains("object not found"),
        "NotFound 是我们自己的话术，rc 那句反话不许被端上来：{err}");
}

#[tokio::test]
async fn deletefile_500_is_not_read_as_absent_and_keeps_rclones_words() {
    let s = MockServer::start().await;
    mount_status_json(&s, "/operations/deletefile", 500,
        serde_json::json!({"error": UNKNOWN_REMOTE_ERROR})).await;
    let err = client(&s).operations_deletefile("nope:", "dir/a.webp").await.unwrap_err();
    assert!(!matches!(err, ListError::NotFound),
        "一个配置错误不许被读成'那一侧没有东西'：{err:?}");
    assert!(matches!(err, ListError::Other(RcError::Rc { .. })),
        "500 走 into_value 那一条口径：{err:?}");
    assert!(err.to_string().contains("didn't find section"), "必须原样带上 rc 的点名：{err}");
}

#[tokio::test]
async fn copyfile_404_is_our_not_found_not_rclones_words() {
    let s = MockServer::start().await;
    // two_sided_call 与 object_call 是两条出口，404 那一档各写了一遍，所以各要一对证人。
    mount_status_json(&s, "/operations/copyfile", 404,
        serde_json::json!({"error": "object not found"})).await;
    let err = client(&s).operations_copyfile(("decka:", "dir/a.webp"), ("deckb:", "in/pick.webp")).await.unwrap_err();
    assert!(matches!(err, ListError::NotFound), "源不见了（面板点中后被人删掉）必须是 NotFound：{err:?}");
    assert!(!err.to_string().contains("object not found"), "同上，话术里不许有 rc 的原文：{err}");
}

#[tokio::test]
async fn copyfile_500_is_not_read_as_absent_and_keeps_rclones_words() {
    let s = MockServer::start().await;
    mount_status_json(&s, "/operations/copyfile", 500,
        serde_json::json!({"error": UNKNOWN_REMOTE_ERROR})).await;
    let err = client(&s).operations_copyfile(("nope:", "dir/a.webp"), ("deckb:", "in/pick.webp")).await.unwrap_err();
    assert!(!matches!(err, ListError::NotFound), "源那一侧的配置错误不许读成'源没了'：{err:?}");
    assert!(matches!(err, ListError::Other(RcError::Rc { .. })), "500 走 into_value 那一条口径：{err:?}");
    assert!(err.to_string().contains("didn't find section"), "必须原样带上 rc 的点名：{err}");
}

#[tokio::test]
async fn size_404_is_our_not_found_not_rclones_words() {
    let s = MockServer::start().await;
    // `operations_size` 的 404 那一档不在 `object_call` 里，是自己写的一行，所以它单独要一对证人。
    mount_status_json(&s, "/operations/size", 404,
        serde_json::json!({"error": "object not found"})).await;
    let err = client(&s).operations_size("deckb:", "box").await.unwrap_err();
    assert!(matches!(err, ListError::NotFound), "404 必须是 NotFound：{err:?}");
    assert!(!err.to_string().contains("object not found"), "同上：{err}");
}

#[tokio::test]
async fn size_500_is_not_read_as_absent_and_keeps_rclones_words() {
    let s = MockServer::start().await;
    mount_status_json(&s, "/operations/size", 500,
        serde_json::json!({"error": UNKNOWN_REMOTE_ERROR})).await;
    let err = client(&s).operations_size("nope:", "box").await.unwrap_err();
    assert!(!matches!(err, ListError::NotFound), "500 不许被读成'那里是空的'：{err:?}");
    assert!(matches!(err, ListError::Other(RcError::Rc { .. })), "500 走 into_value 那一条口径：{err:?}");
    // 这一条同时钉住"500 不会被读成 0 个对象"：错误在 `Sizes` 解析之前就抛出来了，
    // `bytes`/`count` 根本没有被编出来，界面拿到的只能是 rc 那句点名，而不是一个看起来很干净的 0。
    assert!(err.to_string().contains("didn't find section"), "必须原样带上 rc 的点名：{err}");
}

#[tokio::test]
async fn size_and_check_read_the_measured_keys_and_never_treat_an_empty_error_array_as_failure() {
    // F7：`check` 的 `error` 是**数组**（`[]` 在 JS 里 truthy），真值是三个列表 + `success`。
    let s = MockServer::start().await;
    mount_json(&s, "/operations/size", serde_json::json!({"bytes": 7, "count": 3, "sizeless": 0})).await;
    let z = client(&s).operations_size("deckb:", "").await.unwrap();
    assert_eq!((z.bytes, z.count), (7, 3));

    mount_json(&s, "/operations/check", serde_json::json!({
        "success": true, "error": [], "missingOnSrc": ["only-dst.webp"], "missingOnDst": [], "differ": [] })).await;
    let c = client(&s).operations_check(("decka:", ""), ("deckb:", "")).await.unwrap();
    assert!(c.success, "空数组不是失败：{:?}", c);
    assert_eq!(c.missing_on_src, vec!["only-dst.webp".to_string()]);
    assert!(c.differ.is_empty(), "`differ: []` 是'没有差异'：{:?}", c.differ);
}

/// 上面那条只读**回来的字段**，没有一条断言 `operations/check` **上线的字节**。
/// 实测（Task 6 破坏 (a2)）：把 `operations_check` 里的 `srcRemote` 写成 `srcremote`，
/// 那 44 条全绿——rc 对不认识的参数静默丢弃（F2），"比对这一个前缀"就变成"比对整个远端"，
/// 而且没有一句报错。同一条危害、同一种钉法，所以这里补一条字节证人。
#[tokio::test]
async fn check_sends_the_same_four_names_as_copyfile() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/check", serde_json::json!({"success": true})).await;
    client(&s).operations_check(("decka:", "sub"), ("deckb:", "box/sub")).await.unwrap();
    assert_eq!(
        body_sent_to(&s, "/operations/check").await,
        "srcFs=decka%3A&srcRemote=sub&dstFs=deckb%3A&dstRemote=box%2Fsub");
}

#[tokio::test]
async fn fsinfo_keeps_absent_feature_bits_absent_instead_of_reading_them_as_false() {
    // I5/F6 的前提：能力位是开放集合，`Copy` 缺位与 `Copy=false` 是两件事
    // （缺位是"这个后端没报"，false 才是一句确定的话）。
    let s = MockServer::start().await;
    mount_json(&s, "/operations/fsinfo", serde_json::json!({
        "Features": {"Move": true}, "Name": "alias", "Root": "//?/C:/x"})).await;
    let info = client(&s).operations_fsinfo("decka:").await.unwrap();
    assert_eq!(info.features.get("Move"), Some(&true));
    assert_eq!(info.features.get("Copy"), None, "缺位不许被读成 false");
}

/// `operations_fsinfo` 发的只有 `fs` 一个参数（实测：它问的是整个后端，没有按名字问这一档）。
/// 全串相等因此同时钉住两件事：名字是 `fs`，而且**没有多一个 `remote`**。
/// 它不能并进 `FS_AND_REMOTE_BODY`——那把 `fs`+`remote` 的期望值与这条不是同一个形状。
#[tokio::test]
async fn fsinfo_sends_fs_and_no_remote_at_all() {
    let s = MockServer::start().await;
    mount_json(&s, "/operations/fsinfo", serde_json::json!({
        "Features": {"Move": true}, "Name": "alias", "Root": "//?/C:/x"})).await;
    client(&s).operations_fsinfo("decka:").await.unwrap();
    assert_eq!(body_sent_to(&s, "/operations/fsinfo").await, "fs=decka%3A");
}

#[test]
fn the_three_forbidden_methods_never_appear_as_a_call_target() {
    // I4 的源码证人。只找**被双引号整条包住的方法名**——那才是调用位置：方法名永远是以字符串实参
    // 交给 call / send / submit_async 的，`banned` 数组里那三条写的就是这种形状（带引号，不是裸词）。
    // 而解释"为什么不调用它们"的注释必须能提它们，扫裸词会把正当的注释判成缺陷。
    // 这条注释自己也不写成那种形状（裁定 I4 的写法纪律，记在 src-tauri/src/rc/mod.rs 里
    // `config_listremotes` 的注释上）：扫描根今天是 src-tauri/src，本文件不在根内，写出来不咬；
    // 但根一旦扩到整个 crate，散文里写出的那种形状就会被当成一次调用判成缺陷——这条证人会咬到自己。
    // `banned` 那一行不算这种形状：它写的是转义体（名字两侧各多一个反斜杠），
    // 按子串匹配不中；实跑把 root 指到 tests 也是 0 条命中，所以扩根先咬的是散文，不是定义。
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let banned = ["\"core/command\"", "\"config/dump\"", "\"config/get\""];
    let mut hits = Vec::new();
    collect_quoted(&root, &banned, &mut hits);
    assert!(hits.is_empty(), "被禁的调用出现了：{hits:?}");
}

fn collect_quoted(dir: &std::path::Path, banned: &[&str], hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("读不到 src/") {
        let path = entry.expect("目录项可读").path();
        if path.is_dir() { collect_quoted(&path, banned, hits); continue; }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") { continue; }
        let text = std::fs::read_to_string(&path).expect("源文件可读");
        for (i, line) in text.lines().enumerate() {
            for b in banned {
                if line.contains(b) {
                    hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                }
            }
        }
    }
}
