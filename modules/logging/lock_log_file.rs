use crate::*;

#[cfg(windows)]
pub(crate) fn with_log_lock<T>(
    _root: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0},
        System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
    };
    let name = "Local\\ShroudForgeLog";
    let wide: Vec<u16> = OsStr::new(&name).encode_wide().chain(Some(0)).collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide.as_ptr()) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let wait = unsafe { WaitForSingleObject(handle, u32::MAX) };
    if wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED {
        unsafe {
            CloseHandle(handle);
        }
        return Err(io::Error::last_os_error());
    }
    let result = operation();
    unsafe {
        ReleaseMutex(handle);
        CloseHandle(handle);
    }
    result
}

#[cfg(unix)]
pub(crate) fn with_log_lock<T>(
    root: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    use std::{fs::OpenOptions, os::fd::AsRawFd};

    let log = shroudforge_loader_config::paths::current_log(root);
    let file_name = log
        .file_name()
        .ok_or_else(|| io::Error::other("log path has no file name"))?;
    let lock = log.with_file_name(format!("{}.lock", file_name.to_string_lossy()));
    if let Some(parent) = lock.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(lock)?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let result = operation();
    let unlock = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if unlock != 0 {
        return Err(io::Error::last_os_error());
    }
    result
}

#[cfg(not(any(windows, unix)))]
pub(crate) fn with_log_lock<T>(
    _root: &Path,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| io::Error::other("logging lock poisoned"))?;
    operation()
}
