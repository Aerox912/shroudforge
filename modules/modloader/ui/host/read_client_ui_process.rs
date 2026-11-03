use super::read_host_arguments::SYNCHRONIZE_ACCESS;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, WaitForSingleObject,
};

pub(crate) fn active_client_ui_pid(root: &Path) -> Option<u32> {
    let pid = shroudforge_loader_config::config::window_state(root)["modloaderUi"]["clientPid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid != 0)?;
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE_ACCESS,
            0,
            pid,
        )
    };
    if process.is_null() {
        return None;
    }
    let active = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT
        && process_uses_current_executable(process);
    unsafe { CloseHandle(process) };
    active.then_some(pid)
}

fn process_uses_current_executable(process: windows_sys::Win32::Foundation::HANDLE) -> bool {
    let mut buffer = vec![0u16; 32768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return false;
    }
    let process_path = PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
    let Ok(process_path) = process_path.canonicalize() else {
        return false;
    };
    let Ok(current_path) = std::env::current_exe().and_then(|path| path.canonicalize()) else {
        return false;
    };
    process_path
        .to_string_lossy()
        .eq_ignore_ascii_case(&current_path.to_string_lossy())
}
