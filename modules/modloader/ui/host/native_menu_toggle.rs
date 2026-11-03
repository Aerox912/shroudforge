use super::window::UiEvent;
use std::ptr;
use std::thread::{self, JoinHandle};
use tao::event_loop::EventLoopProxy;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, ResetEvent, SetEvent, WaitForMultipleObjects,
};

pub(crate) struct NativeMenuToggle {
    enabled_event: HANDLE,
    open_event: HANDLE,
    stop_event: HANDLE,
    enabled: bool,
    watcher: Option<JoinHandle<()>>,
}

impl NativeMenuToggle {
    pub(crate) fn new(
        game_pid: u32,
        enabled: bool,
        proxy: EventLoopProxy<UiEvent>,
    ) -> Result<Option<Self>, String> {
        if game_pid == 0 {
            return Ok(None);
        }
        let enabled_name = event_name("Enabled", game_pid);
        let open_name = event_name("Open", game_pid);
        let stop_name = event_name("Stop", game_pid);
        let enabled_event = create_event(&enabled_name, true)?;
        let open_event = match create_event(&open_name, false) {
            Ok(event) => event,
            Err(error) => {
                unsafe { CloseHandle(enabled_event) };
                return Err(error);
            }
        };
        let stop_event = match create_event(&stop_name, true) {
            Ok(event) => event,
            Err(error) => {
                unsafe {
                    CloseHandle(open_event);
                    CloseHandle(enabled_event);
                }
                return Err(error);
            }
        };
        let mut result = Self {
            enabled_event,
            open_event,
            stop_event,
            enabled: false,
            watcher: None,
        };
        let visibility_result = unsafe {
            if enabled {
                SetEvent(result.enabled_event)
            } else {
                ResetEvent(result.enabled_event)
            }
        };
        if visibility_result == 0 {
            return Err("Could not set native mod menu visibility".into());
        }
        result.enabled = enabled;
        let stop_handle = stop_event as usize;
        let open_handle = open_event as usize;
        result.watcher = Some(
            thread::Builder::new()
                .name("sf-native-menu-toggle".into())
                .spawn(move || {
                    let handles = [stop_handle as HANDLE, open_handle as HANDLE];
                    loop {
                        let result =
                            unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
                        if result == WAIT_OBJECT_0 {
                            break;
                        }
                        if result == WAIT_OBJECT_0 + 1 {
                            if proxy.send_event(UiEvent::Toggle).is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                })
                .map_err(|error| format!("Could not start native mod menu listener: {error}"))?,
        );
        Ok(Some(result))
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) -> bool {
        if self.enabled == enabled {
            return true;
        }
        let result = unsafe {
            if enabled {
                SetEvent(self.enabled_event)
            } else {
                ResetEvent(self.enabled_event)
            }
        };
        if result != 0 {
            self.enabled = enabled;
            true
        } else {
            false
        }
    }
}

impl Drop for NativeMenuToggle {
    fn drop(&mut self) {
        unsafe { SetEvent(self.stop_event) };
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
        unsafe {
            ResetEvent(self.enabled_event);
            CloseHandle(self.stop_event);
            CloseHandle(self.open_event);
            CloseHandle(self.enabled_event);
        }
    }
}

fn create_event(name: &[u16], manual_reset: bool) -> Result<HANDLE, String> {
    let event = unsafe { CreateEventW(ptr::null(), i32::from(manual_reset), 0, name.as_ptr()) };
    if event.is_null() {
        Err(format!(
            "Could not create native menu event: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(event)
    }
}

fn event_name(kind: &str, game_pid: u32) -> Vec<u16> {
    format!("Local\\ShroudForge.ModMenu.{kind}.{game_pid}")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}
