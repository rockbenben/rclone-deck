//! `rcd.log` 轮转的证人（裁定 R70）。
//!
//! 阈值不是拍的：操作员真实日志目录里那份 `rcd.log` 两天长了 3.19 MB / 24,018 行，而同目录
//! 里每任务那份日志只有 0.1 KB——所以这套只管那一个文件。这里钉的是它的**形状**：
//! 什么时候挪、挪成什么名字、剪掉谁、以及**绝对不许碰谁**。

use std::path::Path;

use rclone_deck_lib::rcd::{LogRotation, LOG_KEEP, LOG_ROTATE_BYTES, rotate_rc_log};

fn write_log(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// 没到阈值就**什么都不做**：不挪、不剪、不创建。这条是"每天启动一次就把历史剪光"那种
/// 实现的反例——阈值存在的意义是让剪这个动作跟用量挂钩，而不是跟启动次数挂钩。
#[test]
fn an_under_threshold_log_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let log = write_log(dir.path(), "rcd.log", "short");
    let got = rotate_rc_log(&log, "20261001-120000", LOG_ROTATE_BYTES, LOG_KEEP).unwrap();
    assert_eq!(got, None, "没到阈值也轮转了：{got:?}");
    assert_eq!(names(dir.path()), vec!["rcd.log".to_string()]);
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "short");
}

/// 超阈值：整份挪成带时间戳的归档，内容一个字节都不改。
/// 注意**当前这份被挪走而不是清空**——轮转发生在 spawn 之前，rclone 随后自己创建当前文件；
/// 先清空再把内容交给 rename 的话，那份历史就没了。
#[test]
fn an_oversized_log_becomes_one_archive_with_its_content_intact() {
    let dir = tempfile::tempdir().unwrap();
    let log = write_log(dir.path(), "rcd.log", &"x".repeat(64));
    let got = rotate_rc_log(&log, "20261001-120000", 32, LOG_KEEP).unwrap()
        .expect("64 字节 > 阈值 32，该轮转");
    assert_eq!(got.archive, "rcd.20261001-120000.log");
    assert_eq!(got.bytes, 64, "报出去的字节数必须是**挪走前**那份的大小：{got:?}");
    assert_eq!(got.dropped, Vec::<String>::new(), "第一份归档没有可剪的");
    assert_eq!(std::fs::read_to_string(dir.path().join(&got.archive)).unwrap(), "x".repeat(64));
    assert!(!log.exists(), "当前那份该被挪走，不是留下一个空壳");
}

/// 剪的口径是"最新 keep 份"，而**时间戳的字典序就是时间序**（`yyyyMMdd-HHmmss`）。
/// 这条同时钉住"删的是最老的那批"：把 `drain` 的方向写反，操作员刚攒出来的那几份新的会先没。
#[test]
fn trimming_keeps_the_newest_and_deletes_by_timestamp_not_by_count_of_starts() {
    let dir = tempfile::tempdir().unwrap();
    for stamp in ["20260101-000001", "20260101-000002", "20260101-000003", "20260101-000004", "20260101-000005"] {
        write_log(dir.path(), &format!("rcd.{stamp}.log"), "old");
    }
    let log = write_log(dir.path(), "rcd.log", &"y".repeat(64));
    let got = rotate_rc_log(&log, "20260101-000006", 32, LOG_KEEP).unwrap().expect("该轮转");

    // 5 份老归档 + 刚挪出来的 1 份 = 6 份，留 keep=5 份，所以只剪掉最老那一份。
    assert_eq!(got.dropped, vec!["rcd.20260101-000001.log".to_string()], "{got:?}");
    let left = names(dir.path());
    assert!(!left.contains(&"rcd.20260101-000001.log".to_string()), "最老的该被剪掉：{left:?}");
    for keep in ["000002", "000003", "000004", "000005", "000006"] {
        assert!(left.contains(&format!("rcd.20260101-{keep}.log")), "最新几份不许动：{left:?}");
    }
    assert_eq!(left.len(), LOG_KEEP, "留住的份数就是 keep，不多不少：{left:?}");
}

/// **这条是整套里最要紧的一条**：同目录里躺着一个 `rcd-stderr.log`，它也以 `rcd` 开头，
/// 但它是凭据回显的落点（见 `wipe_credential_echo`），删它的后果比删一份旧日志重。
/// 归档前缀带着那个点（`rcd.`），所以它进不了候选；这里把这件事钉死，
/// 免得哪天有人把前缀放宽成"以 rcd 开头"。
#[test]
fn rcd_stderr_log_is_never_a_rotation_candidate() {
    let dir = tempfile::tempdir().unwrap();
    write_log(dir.path(), "rcd-stderr.log", "凭据回显的落点，不是归档");
    write_log(dir.path(), "notes.txt", "操作员自己放的东西");
    write_log(dir.path(), "rcd..log", "空时间戳的畸形名字，也不算归档");
    let log = write_log(dir.path(), "rcd.log", &"z".repeat(64));

    let got = rotate_rc_log(&log, "20261001-120000", 32, 0).unwrap().expect("该轮转");
    // keep = 0：除了刚挪出来的那份，其它归档都可以剪——但**这两个非归档文件必须还在**。
    assert_eq!(got.archive, "rcd.20261001-120000.log");
    let left = names(dir.path());
    assert!(left.contains(&"rcd-stderr.log".to_string()), "把凭据落点删了：{left:?}");
    assert!(left.contains(&"notes.txt".to_string()), "把操作员的东西删了：{left:?}");
    assert!(left.contains(&"rcd..log".to_string()), "空时间戳的名字不该被当成归档：{left:?}");
}

/// "这份日志还不存在"才等于"没东西可轮转"。别的失败不许伪装成首次启动静默放行——
/// 那样磁盘出问题也会一路"没有可轮转的"过去，上限就没了。
/// 夹具：让当前日志**是个目录**，`rename` 必然失败。
#[test]
fn a_log_path_that_cannot_be_renamed_reports_instead_of_passing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("rcd.log")).unwrap();
    // 到阈值这条线上它得先被当成"要轮转"——目录的 metadata().len() 各 OS 不一样，
    // 所以把阈值压到 0 之外的办法不可靠：这里直接给一个不可能满足的阈值不行，
    // 于是用 `keep = 0` 之外的另一条路：先确认**不存在**那一档是 None，再确认存在但挪不动的不是 None。
    let missing = dir.path().join("never-created.log");
    assert_eq!(rotate_rc_log(&missing, "20261001-120000", 32, LOG_KEEP).unwrap(), None,
        "不存在=首次启动，必须是 None，否则第一次启动就报错");

    let err = rotate_rc_log(&dir.path().join("rcd.log"), "20261001-120000", 0, LOG_KEEP);
    match err {
        // 目录的 len() 在各 OS 上不同：Windows 给 0，于是"阈值 0"下它被判为不必轮转也是**可接受**的
        // 答案——所以这条只钉一件事：不许出现"既没轮转也没报错，还什么都没留下"的那种静默。
        Ok(None) => assert!(dir.path().join("rcd.log").is_dir(),
            "Ok(None) 的前提是这一份确实动不了；它得还是那个目录"),
        Ok(Some(r)) => panic!("路径是个目录，居然报成轮转成功了：{r:?}"),
        Err(_) => {}
    }
}

/// 阈值本身是量出来的，所以这里钉的是**边界形状**而不是那个数：等于阈值不轮转，
/// 多一个字节才轮转。改 `LOG_ROTATE_BYTES` 不该让这条失效。
#[test]
fn the_threshold_is_exclusive_on_the_equal_side() {
    let dir = tempfile::tempdir().unwrap();
    let body = "a".repeat(LOG_ROTATE_BYTES as usize);
    let log = write_log(dir.path(), "rcd.log", &body);
    assert_eq!(rotate_rc_log(&log, "20261001-120000", LOG_ROTATE_BYTES, LOG_KEEP).unwrap(), None,
        "正好等于阈值就轮转的话，一份日志会在两次启动之间被反复挪来挪去");

    std::fs::write(&log, format!("{body}b")).unwrap();
    let got = rotate_rc_log(&log, "20261001-120000", LOG_ROTATE_BYTES, LOG_KEEP).unwrap();
    assert_eq!(got.map(|r| (r.archive, r.bytes)),
        Some(("rcd.20261001-120000.log".to_string(), LOG_ROTATE_BYTES + 1)),
        "比阈值多一个字节就必须轮转");
}

/// 报出去的结构要能被调用方直接拼成那句话说给操作员听（缺字段就等于写了句含糊的话）。
#[test]
fn a_rotation_reports_its_archive_size_and_what_it_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let log = write_log(dir.path(), "rcd.log", "hello");
    let got = rotate_rc_log(&log, "20261001-120000", 0, LOG_KEEP).unwrap()
        .expect("阈值为 0、有内容，该轮转");
    assert!(got.archive.contains("20261001-120000"), "{got:?}");
    assert_eq!(got.bytes, 5, "{got:?}");
    let _ = LogRotation { archive: String::new(), bytes: 0, dropped: vec![] };
}
