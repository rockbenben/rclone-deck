use std::ffi::OsStr;
use std::io;
use std::mem::{size_of, MaybeUninit};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

pub(crate) struct Process {
    pid: libc::pid_t,
    created: u64,
}

fn identity(pid: libc::pid_t) -> io::Result<Option<u64>> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = size_of::<libc::proc_bsdinfo>() as i32;
    let n = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    if n != size {
        let error = io::Error::last_os_error();
        return if n <= 0 && error.raw_os_error() == Some(libc::ESRCH) { Ok(None) } else { Err(error) };
    }
    let info = unsafe { info.assume_init() };
    // XNU's SZOMB (5) is a process that has exited and is waiting to be reaped.
    if info.pbi_status == 5 { return Ok(None); }
    Ok(Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec))
}

impl Process {
    pub fn open(pid: u32, _may_terminate: bool) -> io::Result<Option<Self>> {
        let pid = i32::try_from(pid).ok().filter(|pid| *pid > 0)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid PID"))?;
        Ok(identity(pid)?.map(|created| Self { pid, created }))
    }

    pub fn exited(&self) -> io::Result<bool> { Ok(identity(self.pid)? != Some(self.created)) }

    pub fn creation_time(&self) -> io::Result<u64> { Ok(self.created) }

    pub fn executable(&self) -> io::Result<PathBuf> {
        let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let n = unsafe { libc::proc_pidpath(self.pid, path.as_mut_ptr().cast(), path.len() as u32) };
        if n <= 0 { return Err(io::Error::last_os_error()); }
        if self.exited()? { return Err(io::Error::new(io::ErrorKind::NotFound, "process exited")); }
        let bytes = &path[..n as usize];
        Ok(PathBuf::from(OsStr::from_bytes(bytes.strip_suffix(&[0]).unwrap_or(bytes))))
    }
}

pub(super) fn unix_seconds(micros: u64) -> u64 { micros / 1_000_000 }
