use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{ERROR_INVALID_PARAMETER, FILETIME, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

/// The same OS handle is used to verify identity, terminate and confirm exit.
pub(crate) struct Process(OwnedHandle);

impl Process {
    pub fn open(pid: u32, may_terminate: bool) -> io::Result<Option<Self>> {
        let mut access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
        if may_terminate {
            access |= PROCESS_TERMINATE;
        }
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() {
            let err = io::Error::last_os_error();
            return if pid != 0 && err.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
                Ok(None)
            } else {
                Err(err)
            };
        }
        // OpenProcess returned a handle owned by this call.
        Ok(Some(Self(unsafe { OwnedHandle::from_raw_handle(handle) })))
    }

    pub fn exited(&self) -> io::Result<bool> {
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub fn creation_time(&self) -> io::Result<u64> {
        let blank = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (blank, blank, blank, blank);
        let ok = unsafe {
            GetProcessTimes(self.0.as_raw_handle(), &mut created, &mut exited, &mut kernel, &mut user)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }

    pub fn executable(&self) -> io::Result<PathBuf> {
        let mut name = vec![0u16; 32_768];
        let mut len = name.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(self.0.as_raw_handle(), 0, name.as_mut_ptr(), &mut len) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(OsString::from_wide(&name[..len as usize]).into())
    }

    pub fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateProcess(self.0.as_raw_handle(), 1) } == 0 {
            let error = io::Error::last_os_error();
            if !self.exited()? {
                return Err(error);
            }
        }
        Ok(())
    }
}

pub(super) fn unix_seconds(filetime: u64) -> u64 {
    (filetime / 10_000_000).saturating_sub(11_644_473_600)
}
