use rclone_deck_lib::runner::tasklog::{Level, TaskLog};

/// R40（执行期发现）：要钉的性质是"**戳里带时区偏移**"，不是"偏移是正的"。
/// 原来这条写的是 `stamp[19..].contains('+') || stamp.ends_with('Z')`，在**任何 UTC 以西的机器上
/// 都是红的**——chrono 在那里渲染 `-05:00`，而 `+` 匹配不上。这台机器 +08:00，所以它一直绿，
/// 绿的其实是运气（brief 的替代写法只用一个 `assert!` 表达，同样在本机测不到西半球那一半，
/// 所以判据抽成函数、再拿 UTC 以西的样本钉住它——见 `the_offset_judgement_accepts_both_signs`）。
fn stamp_carries_a_timezone_offset(stamp: &str) -> bool {
    // 前 19 个字符是 `YYYY-MM-DDTHH:MM:SS`，剩下的偏移段必须以 `Z` 收尾，或带着符号；
    // 符号朝哪边都不该让测试变红。短于 19 的戳 = 连时间都没有，更谈不上偏移。
    match stamp.get(19..) {
        Some(rest) => rest.ends_with('Z') || rest.contains('+') || rest.contains('-'),
        None => false,
    }
}

#[test]
fn create_names_the_file_after_the_job_and_the_second() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "r2-up", &[]).unwrap();
    assert!(log.path().exists());

    let name = log.path().file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("r2-up-"), "{name}");
    assert!(name.ends_with(".log"), "{name}");
    let stamp = &name["r2-up-".len()..name.len() - ".log".len()];
    assert_eq!(stamp.len(), 15, "YYYYMMDD-HHMMSS 应是 15 位，实际 {stamp:?}");
    assert_eq!(stamp.as_bytes()[8], b'-', "{stamp}");
}

#[test]
fn a_second_log_in_the_same_second_gets_a_suffix_instead_of_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let a = TaskLog::create(dir.path(), "r2-up", &[]).unwrap();
    let b = TaskLog::create(dir.path(), "r2-up", &[]).unwrap();
    assert_ne!(a.path(), b.path());

    a.line(Level::Info, "first").unwrap();
    b.line(Level::Info, "second").unwrap();
    assert!(std::fs::read_to_string(a.path()).unwrap().contains("first"));
    assert!(std::fs::read_to_string(b.path()).unwrap().contains("second"));
}

#[test]
fn every_line_carries_an_rfc3339_stamp_and_a_padded_level() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &[]).unwrap();
    log.line(Level::Info, "开始").unwrap();
    log.line(Level::Error, "失败").unwrap();

    let text = std::fs::read_to_string(log.path()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].contains(" INFO  开始"), "{lines:?}");
    assert!(lines[1].contains(" ERROR 失败"), "{lines:?}");

    let stamp = lines[0].split_whitespace().next().unwrap();
    assert_eq!(stamp.as_bytes()[10], b'T', "{stamp}");
    assert!(stamp_carries_a_timezone_offset(stamp), "看不出时区偏移：{stamp}");
}

/// R40 的证人：**这台机器在东半球（+08:00），所以"只认 `+`"那种写法在这里永远测不出问题**——
/// 它在 UTC 以西的机器上是红的。把判据抽成函数后，西半球的样本能在这台机器上直接钉住：
/// 谁把性质写回"偏移必须是正的"，这条就红。
#[test]
fn the_offset_judgement_accepts_both_signs_and_z_not_only_the_eastern_half() {
    // 东（本机形状）、西（原写法会误判成"没有偏移"）、Z 收尾，三种都算带了时区信息。
    for stamp in [
        "2026-09-29T12:34:56.789012345+08:00",
        "2026-09-29T12:34:56.789012345-05:00",
        "2026-09-29T12:34:56Z",
    ] {
        assert!(stamp_carries_a_timezone_offset(stamp), "这是合法的带偏移戳却被判成没有：{stamp}");
    }

    // 反向：没有偏移段的戳必须判否——有人把 `write()` 的格式串换成手动的
    // `%Y-%m-%dT%H:%M:%S`（不带 zone）时，日志就再也没法跨时区对账，那正是这条要拦的。
    assert!(!stamp_carries_a_timezone_offset("2026-09-29T12:34:56"),
        "没有偏移段的戳被放行了，这条断言就成了空转");
    assert!(!stamp_carries_a_timezone_offset("2026-09-29T12"), "连时间都不全的戳被放行了");
}

#[test]
fn kv_appends_fields_in_the_order_given() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &[]).unwrap();
    log.kv(Level::Info, "任务提交", &[("jobid", "12".into()), ("dry_run", "true".into())]).unwrap();

    let text = std::fs::read_to_string(log.path()).unwrap();
    assert!(text.contains("任务提交 jobid=12 dry_run=true"), "{text}");
}

#[test]
fn a_registered_secret_value_is_scrubbed_wherever_it_appears() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &["hunter2secret".into()]).unwrap();
    log.line(Level::Info, "argv=--rc-pass hunter2secret").unwrap();
    log.kv(Level::Info, "启动", &[("pass", "hunter2secret".into())]).unwrap();

    let text = std::fs::read_to_string(log.path()).unwrap();
    assert!(!text.contains("hunter2secret"), "密钥原文进了日志：{text}");
    assert_eq!(text.matches("[REDACTED]").count(), 2, "{text}");
}

#[test]
fn an_unregistered_value_is_not_scrubbed() {
    // 脱敏是按值精确匹配，不是"看到 pass 这个词就抹掉"——否则日志会变成一堆 [REDACTED] 没法用。
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &["hunter2secret".into()]).unwrap();
    log.kv(Level::Info, "启动", &[("pass", "not-the-secret".into())]).unwrap();

    let text = std::fs::read_to_string(log.path()).unwrap();
    assert!(text.contains("pass=not-the-secret"), "{text}");
}

#[test]
fn tail_returns_whole_lines_from_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &[]).unwrap();
    for i in 0..20 {
        log.line(Level::Info, &format!("line-{i}")).unwrap();
    }

    let t = TaskLog::tail(log.path(), 120).unwrap();
    assert!(t.contains("line-19"), "{t}");
    assert!(!t.contains("line-5"), "{t}");
    assert!(t.lines().count() <= 3, "{t}");
    for l in t.lines() {
        assert!(l.starts_with("20"), "tail 从半个时间戳起头了：{l:?}");
    }
}

#[test]
fn tail_returns_everything_when_the_budget_is_bigger_than_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &[]).unwrap();
    log.line(Level::Info, "only").unwrap();

    let t = TaskLog::tail(log.path(), 1 << 20).unwrap();
    assert_eq!(t.lines().count(), 1, "{t}");
    assert!(t.contains("only"));
}

#[test]
fn tail_of_a_missing_file_is_empty_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(TaskLog::tail(&dir.path().join("nope.log"), 1024).unwrap(), "");
}

/// R40 的形状一：窗口起点**正好落在行边界**上。原写法不管三七二十一丢掉第一段到第一个换行为止
/// 的字节，于是起点那条**完整**行被当成半行丢了——界面少看一行，而这一行往往正是最新的那几条之一。
/// 分清"半行"还是"整行"只能回头看窗口前那一个字节，所以这条按字节偏移把预算精确摆到行首。
#[test]
fn a_window_beginning_exactly_on_a_line_boundary_does_not_drop_that_line() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &[]).unwrap();
    for i in 0..10 {
        log.line(Level::Info, &format!("row-{i}")).unwrap();
    }

    let bytes = std::fs::read(log.path()).unwrap();
    let needle = b"row-7";
    let at = bytes.windows(needle.len()).position(|w| w == needle).expect("夹具里应当有 row-7");
    let line_start = bytes[..at].iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
    let budget = (bytes.len() - line_start) as u64;

    let t = TaskLog::tail(log.path(), budget).unwrap();
    assert!(t.contains("row-7"),
        "预算正好从行首开始，第一条完整行却被当成半行丢了：{t}");
    assert!(t.contains("row-8") && t.contains("row-9"), "{t}");
    assert!(!t.contains("row-6"), "预算之外的旧行不该出现：{t}");
    assert_eq!(t.lines().count(), 3, "行边界起点应当给出三条整行：{t}");
    for l in t.lines() {
        assert!(l.starts_with("20"), "tail 从半个时间戳起头了：{l:?}");
    }
}

/// R40 的形状二：最新那一行**比预算还长**。原写法在窗口里找不到换行就返回 `""`——
/// "这次运行只写了一条超长行"于是渲染成"没有日志"，而这正是本模块存在的理由
/// （删除清单是一行一条 key，最坏情况就是长行）。软预算的含义是：只丢前导那半行，
/// 窗口里没有换行时把**它装着的那段**交回去，而不是空串。
#[test]
fn a_newest_line_longer_than_the_budget_is_not_reported_as_no_log() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &[]).unwrap();
    log.line(Level::Info, "run started").unwrap();
    let long = format!("deleted: bucket/{}", "deep/dir/".repeat(400));
    log.line(Level::Info, &long).unwrap();

    let t = TaskLog::tail(log.path(), 64).unwrap();
    assert!(!t.is_empty(), "最新那一行比预算长，界面于是显示'没有日志'——模块的立身之本被推翻");
    // 给的是那一行的**后半**（预算就那么多），但它必须是最新那一行的内容。
    assert!(t.contains("deep/dir/"), "窗口里这段不是最新那一行：{t}");
    assert!(!t.contains("run started"), "预算 64 字节却把旧行也捞进来了：{t}");

    // 同一个分支更硬的一种：崩溃时最后那一行只写了一半——既没有换行收尾，长度也超过预算，
    // 于是窗口里一个 `\n` 都找不到（`find('\n')` 返回 None）。
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(log.path()).unwrap();
        f.write_all(format!("deleted: bucket/{}partial-line-from-a-crash", "x".repeat(200)).as_bytes())
            .unwrap();
    }
    let t = TaskLog::tail(log.path(), 48).unwrap();
    assert!(t.contains("partial-line-from-a-crash"),
        "窗口里没有换行时返回了空串：{t}");
}

// ---- 下面四条是本任务额外钉死的安全/可用性性质（brief 的 9 条没覆盖）----

/// R37（实测）：rclone 自己会在 stderr 里回显口令，形如
/// `DEBUG : Setting rc_pass="<pw>" from environment variable RCLONE_RC_PASS`。
/// 这条对脱敏器是**真实输入**：同一行里同一个密码出现两次，一次裸值、一次在 `KEY="值"` 里。
/// 两个都必须消失，而这行的其余部分（解释这条日志为什么存在）必须留着——过度脱敏会毁掉
/// 事后回溯能力（R35：`error` 是载荷）。
#[test]
fn the_same_secret_in_two_shapes_in_one_line_is_scrubbed_without_eating_the_line() {
    let pw = "hunter2secret";
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &[pw.into()]).unwrap();
    log.line(
        Level::Warn,
        &format!(r#"DEBUG : Setting rc_pass="{pw}" from environment variable RCLONE_RC_PASS={pw}"#),
    )
    .unwrap();

    let text = std::fs::read_to_string(log.path()).unwrap();
    assert!(!text.contains(pw), "密钥原文进了日志：{text}");
    assert_eq!(text.matches("[REDACTED]").count(), 2, "两种形状应各擦一处：{text}");
    // 只有值被抹掉，字段名与原因留在原位。
    assert!(
        text.contains(r#"Setting rc_pass="[REDACTED]" from environment variable RCLONE_RC_PASS=[REDACTED]"#),
        "{text}"
    );
}

/// 空串和极短串必须**整条跳过**：`"".replace` 会在每个位置插入掩码，把日志刷成一片
/// [REDACTED]；`"ab"` 这种长度会把所有正常词都抹掉。按值脱敏的代价就是要设一个下限，
/// 但这个下限只能让日志变啰嗦，不能让它不可用。
#[test]
fn empty_and_short_secret_values_are_skipped_instead_of_masking_everything() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = vec!["".to_string(), "ab".to_string(), "abc".to_string()];
    let log = TaskLog::create(dir.path(), "j", &secrets).unwrap();
    log.line(Level::Info, "删除 ab 与 abc 与空串").unwrap();
    log.kv(Level::Info, "state", &[("error", "aborted after 3 retries".into())]).unwrap();

    let text = std::fs::read_to_string(log.path()).unwrap();
    assert!(!text.contains("[REDACTED]"), "短值/空值被当成了密钥：{text}");
    // R35：失败原因是载荷，不能顺手抹掉。
    assert!(text.contains("error=aborted after 3 retries"), "{text}");
}

/// 写进去的行必须在 `line()` 返回时就落到文件里（TaskLog 还活着、没关没 drop）：
/// 崩溃时留下的删除清单是唯一证据，缓冲在半路等于没写。
#[test]
fn a_line_is_readable_from_disk_before_the_log_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &[]).unwrap();
    let path = log.path().to_path_buf();

    log.line(Level::Info, "deleted: img/a.png").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains("deleted: img/a.png"), "{text}");

    log.kv(Level::Info, "deleted", &[("key", "img/b.png".into())]).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 2, "第二行还停在缓冲区里：{text}");
    assert!(text.contains("deleted key=img/b.png"), "{text}");
}

/// R39（控制器裁定，落在 brief 之后）：一个密钥可能是另一个的子串。按**给的顺序**换的话，
/// 短的先命中会把长的打断，把长密钥的尾巴原样留在日志里——按值脱敏因此必须长值优先。
#[test]
fn a_secret_that_contains_another_secret_leaves_no_tail() {
    let dir = tempfile::tempdir().unwrap();
    // 故意把短的值放在前面。
    let secrets = vec!["deck-abc".to_string(), "deck-abcdef".to_string()];
    let log = TaskLog::create(dir.path(), "j", &secrets).unwrap();
    log.line(Level::Info, "rc-user=deck-abc pass=deck-abcdef").unwrap();

    let text = std::fs::read_to_string(log.path()).unwrap();
    assert_eq!(text.matches("[REDACTED]").count(), 2, "{text}");
    assert!(!text.contains("deck-"), "子串密钥没擦干净：{text}");
    // 结构必须正好是两个掩码，尾巴一个字符都不能剩。
    let line = text.lines().next().unwrap();
    assert!(line.ends_with("rc-user=[REDACTED] pass=[REDACTED]"), "留了尾巴：{line:?}");
}

/// Windows 侧的硬要求：日志目录不存在要建（含反斜杠分隔的多级、非 ASCII 名），
/// 非 ASCII 的 job id 要能进文件名并把内容读回来。
#[test]
fn a_missing_nested_log_dir_is_created_and_non_ascii_paths_survive() {
    let dir = tempfile::tempdir().unwrap();
    let log_dir = std::path::PathBuf::from(format!(r#"{}\日志目录\runs"#, dir.path().display()));
    let log = TaskLog::create(&log_dir, "图片上传-测试", &["pw-value".into()]).unwrap();
    log.line(Level::Info, "删除 D:\\图床\\2026\\a.png").unwrap();

    assert!(log.path().exists());
    let name = log.path().file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("图片上传-测试-"), "{name}");
    let text = std::fs::read_to_string(log.path()).unwrap();
    assert!(text.contains(r"删除 D:\图床\2026\a.png"), "{text}");
}

/// 删除清单可能有几百行，每行一个键；一行都不能丢、不能截断（截断的清单正好让事故无法回溯）。
#[test]
fn hundreds_of_lines_all_land_intact_including_a_long_deletion_key() {
    let dir = tempfile::tempdir().unwrap();
    let log = TaskLog::create(dir.path(), "j", &[]).unwrap();
    let long_key = format!("very-long-key-{}", "x".repeat(600));
    log.line(Level::Info, &long_key).unwrap();
    for i in 0..300 {
        log.line(Level::Info, &format!("deleted: bucket/dir{i}/file-{i}.png")).unwrap();
    }

    let text = std::fs::read_to_string(log.path()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 301, "行数不对，有行被丢了");
    assert!(lines[0].ends_with(&long_key), "首行被截断");
    for i in 0..300 {
        assert!(text.contains(&format!("deleted: bucket/dir{i}/file-{i}.png")), "少了第 {i} 行");
    }
    assert_eq!(TaskLog::tail(log.path(), 1 << 20).unwrap(), text, "tail 应能原样返回整份文件");
}
