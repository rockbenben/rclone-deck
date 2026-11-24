use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;

/// A pidfd stays tied to the original process when its PID is reused.
pub(crate) struct Process {
    pid: u32,
    fd: OwnedFd,
}

impl Process {
    pub fn open(pid: u32, _may_terminate: bool) -> io::Result<Option<Self>> {
        let pid_arg = i32::try_from(pid).ok().filter(|pid| *pid > 0)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid PID"))?;
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid_arg, 0u32) };
        if fd < 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(libc::ESRCH) { Ok(None) } else { Err(error) };
        }
        Ok(Some(Self { pid, fd: unsafe { OwnedFd::from_raw_fd(fd as i32) } }))
    }

    pub fn exited(&self) -> io::Result<bool> {
        let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        loop {
            match unsafe { libc::poll(&mut pfd, 1, 0) } {
                0 => return Ok(false),
                n if n > 0 => return Ok(pfd.revents & (libc::POLLIN | libc::POLLHUP) != 0),
                _ => {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted { return Err(error); }
                }
            }
        }
    }

    pub fn creation_time(&self) -> io::Result<u64> {
        let stat = std::fs::read(format!("/proc/{}/stat", self.pid))?;
        let stat = String::from_utf8_lossy(&stat);
        // comm is parenthesized and may itself contain spaces and parentheses.
        let ticks: u64 = stat.rsplit_once(") ").and_then(|(_, tail)| tail.split_whitespace().nth(19))
            .and_then(|field| field.parse().ok()).ok_or_else(|| io::Error::other("missing process start time"))?;
        let boot: u64 = std::fs::read_to_string("/proc/stat")?.lines()
            .find_map(|line| line.strip_prefix("btime ").and_then(|field| field.parse().ok()))
            .ok_or_else(|| io::Error::other("missing boot time"))?;
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hz <= 0 { return Err(io::Error::last_os_error()); }
        self.ensure_live()?;
        Ok(boot * 1_000_000 + ticks * 1_000_000 / hz as u64)
    }

    pub fn executable(&self) -> io::Result<PathBuf> {
        let path = std::fs::read_link(format!("/proc/{}/exe", self.pid))?;
        self.ensure_live()?;
        Ok(path)
    }

    fn ensure_live(&self) -> io::Result<()> {
        if self.exited()? { return Err(io::Error::new(io::ErrorKind::NotFound, "process exited")); }
        Ok(())
    }

    pub fn terminate(&self) -> io::Result<()> {
        let status = unsafe {
            libc::syscall(libc::SYS_pidfd_send_signal, self.fd.as_raw_fd(), libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(), 0u32)
        };
        if status < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) { return Err(error); }
        }
        Ok(())
    }
}

pub(super) fn unix_seconds(micros: u64) -> u64 { micros / 1_000_000 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pidfd_tracks_identity_and_signals_the_owned_child() {
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let process = Process::open(child.id(), true).unwrap().unwrap();
        let observed = (|| -> io::Result<_> {
            let before = (process.creation_time()?, process.executable()?, process.exited()?);
            process.terminate()?;
            Ok(before)
        })();
        let _ = child.kill();
        child.wait().unwrap();
        let (start, exe, exited) = observed.unwrap();
        assert!(unix_seconds(start) > 0);
        assert_eq!(std::fs::canonicalize(exe).unwrap(), std::fs::canonicalize("/bin/sleep").unwrap());
        assert!(!exited);
        assert!(process.exited().unwrap());
    }
}
