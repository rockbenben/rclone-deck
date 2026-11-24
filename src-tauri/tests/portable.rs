use std::path::Path;

use rclone_deck_lib::config::{edit::config_folder_of, jobs::JobsFile, AppConfig};
use rclone_deck_lib::state::{write_default_jobs_toml, DEFAULT_JOBS_TOML};

#[test]
fn fresh_configuration_and_open_directory_use_native_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config").join("jobs.toml");
    write_default_jobs_toml(&path).unwrap();
    assert_eq!(config_folder_of(&path), path.parent());
    assert!(config_folder_of(Path::new("relative/jobs.toml")).is_none());
    let jobs = JobsFile::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(Path::new(&jobs.defaults.rclone_bin).is_absolute());
    assert!(Path::new(&jobs.jobs[0].src).is_absolute());
    let config = AppConfig::from_defaults(&JobsFile::parse(DEFAULT_JOBS_TOML).unwrap().defaults).unwrap();
    assert!(AppConfig::jobs_toml_path().unwrap().is_absolute());
    assert_eq!(config.log_dir.parent(), config.pid_path.parent());
}

#[cfg(unix)]
#[test]
fn posix_roots_cover_descendants_and_colons_do_not_become_remotes() {
    use rclone_deck_lib::{endpoint::endpoint_of, scope::Scope};

    let root = Scope::local(Path::new("/"));
    let child = endpoint_of("/sample/photos:archive/day");
    assert!(!child.is_remote());
    let child = Scope::of_endpoint(&child);
    assert_eq!(child.describe(), "/sample/photos:archive/day");
    assert!(root.conflicts(&child));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn linux_reclaims_a_daemon_and_shuts_its_replacement_down() {
    use rclone_deck_lib::rcd::{pidfile, RcdSpec, RcdSupervisor};

    let rclone = std::env::var_os("RCLONE_BIN").expect("set RCLONE_BIN to an absolute rclone path");
    let temp = tempfile::tempdir().unwrap();
    let conf = temp.path().join("rclone.conf");
    std::fs::write(&conf, "").unwrap();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let spec = RcdSpec {
        rclone_bin: rclone.into(),
        port: listener.local_addr().unwrap().port(),
        flags: Vec::new(),
        exclude: Vec::new(),
        rclone_config: Some(conf),
        log_file: Some(temp.path().join("rcd.log")),
        pid_path: temp.path().join("rcd.pid"),
        log_level_debug: false,
    };
    drop(listener);
    let first = RcdSupervisor::start(spec.clone()).await.unwrap();
    let previous_pid = first.pid;
    assert_eq!(pidfile::read(&spec.pid_path).unwrap().pid, previous_pid);
    drop(first);
    let second = RcdSupervisor::start(spec.clone()).await.unwrap();
    assert_ne!(second.pid, previous_pid);
    assert!(second.client.call("core/version", &[]).await.is_ok());
    second.shutdown().await.unwrap();
    assert!(!spec.pid_path.exists());
    assert!(!rclone_deck_lib::rcd::port_accepts(spec.port, std::time::Duration::from_millis(200)).await);
}
