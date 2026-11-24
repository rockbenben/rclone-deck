//! macOS has no public API to signal an arbitrary process through a stable handle.
//! A guardian owns the actual Child and shuts it down when the panel's pipe closes.
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[cfg(target_os = "macos")]
pub(super) const FLAG: &str = "--rclone-deck-supervise";

#[doc(hidden)]
#[cfg(target_os = "macos")]
pub fn run_if_requested() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(FLAG.as_ref()) { return; }
    let result = args.next().ok_or_else(|| io::Error::other("missing rclone executable"))
        .and_then(|exe| {
            let mut command = Command::new(exe);
            command.args(args);
            supervise(command, io::stdin(), io::stdout())
        });
    if let Err(error) = &result { eprintln!("rclone guardian: {error}"); }
    std::process::exit(i32::from(result.is_err()));
}

fn supervise(mut command: Command, mut parent: impl Read + Send + 'static, mut ready: impl Write) -> io::Result<()> {
    let (closed, state) = mpsc::channel();
    std::thread::Builder::new().spawn(move || {
        let _ = parent.read(&mut [0]);
        let _ = closed.send(());
    })?;
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).spawn()?;
    let result = (|| -> io::Result<()> {
        writeln!(ready, "{}", child.id())?;
        ready.flush()?;
        loop {
            if child.try_wait()?.is_some() { return Ok(()); }
            match state.recv_timeout(Duration::from_millis(100)) {
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                _ => return Ok(()),
            }
        }
    })();
    // Only this thread waits or signals, and the PID can't be reused until it reaps the child.
    if child.try_wait()?.is_none() {
        child.kill()?;
        child.wait()?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    #[test]
    fn closes_the_daemon_when_the_panels_pipe_is_closed() {
        let (panel, parent) = UnixStream::pair().unwrap();
        let (ready, report) = UnixStream::pair().unwrap();
        report.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let guardian = std::thread::spawn(move || {
            let mut command = Command::new("/bin/sleep");
            command.arg("30");
            supervise(command, parent, ready).unwrap();
        });
        let mut pid = String::new();
        BufReader::new(report).read_line(&mut pid).unwrap();
        let daemon = super::super::process::Process::open(pid.trim().parse().unwrap(), false).unwrap().unwrap();
        assert!(!daemon.exited().unwrap());
        drop(panel);
        guardian.join().unwrap();
        assert!(daemon.exited().unwrap());
    }
}
