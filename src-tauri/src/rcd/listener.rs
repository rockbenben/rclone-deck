#[cfg(target_os = "linux")]
pub(super) fn port_owner_pid(port: u16) -> Option<u32> {
    use std::os::unix::ffi::OsStrExt;
    let table = std::fs::read_to_string("/proc/net/tcp").ok()?;
    let inodes = listener_inodes(&table, port);
    if inodes.len() != 1 { return None; }
    let socket = format!("socket:[{}]", inodes[0]);
    let mut owner = None;
    for process in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = process.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(process.path().join("fd")) else { continue; };
        if fds.flatten().any(|fd| std::fs::read_link(fd.path())
            .is_ok_and(|link| link.as_os_str().as_bytes() == socket.as_bytes()))
        {
            if owner.is_some() { return None; }
            owner = Some(pid);
        }
    }
    owner
}

#[cfg(target_os = "linux")]
fn listener_inodes(table: &str, port: u16) -> Vec<&str> {
    let loopback = format!("0100007F:{port:04X}");
    let wildcard = format!("00000000:{port:04X}");
    table.lines().filter_map(|line| {
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        (fields.len() >= 10 && fields[3] == "0A" && [loopback.as_str(), wildcard.as_str()].contains(&fields[1]))
            .then(|| fields[9])
    }).collect()
}

#[cfg(target_os = "macos")]
pub(super) fn port_owner_pid(port: u16) -> Option<u32> {
    let filter = format!("-i4TCP:{port}");
    let out = std::process::Command::new("/usr/sbin/lsof")
        .args(["-nP", "-a", &filter, "-sTCP:LISTEN", "-Fpn"])
        .output().ok()?;
    if !out.status.success() { return None; }
    listener_from_lsof(&String::from_utf8_lossy(&out.stdout), port)
}

#[cfg(any(test, target_os = "macos"))]
fn listener_from_lsof(output: &str, port: u16) -> Option<u32> {
    let loopback = format!("n127.0.0.1:{port}");
    let wildcard = format!("n*:{port}");
    let mut process = None;
    let mut owner = None;
    for line in output.lines() {
        if let Some(pid) = line.strip_prefix('p') {
            process = pid.parse().ok();
        } else if line == loopback || line == wildcard {
            let pid = process?;
            if owner.is_some_and(|owner| owner != pid) { return None; }
            owner = Some(pid);
        }
    }
    owner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_actual_loopback_listener() {
        let socket = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        assert_eq!(port_owner_pid(socket.local_addr().unwrap().port()), Some(std::process::id()));
    }

    #[test]
    fn lsof_ignores_other_interfaces_and_rejects_ambiguous_owners() {
        let output = "p10\nn10.0.0.4:6400\np11\nn127.0.0.1:6400\np12\nn*:6401\n";
        assert_eq!(listener_from_lsof(output, 6400), Some(11));
        assert_eq!(listener_from_lsof(output, 6401), Some(12));
        assert_eq!(listener_from_lsof(&format!("{output}p13\nn*:6400\n"), 6400), None);
    }
}
