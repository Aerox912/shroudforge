//! Owner-thread native access to declared Hotkey Manager settings.
//! Parsing happens on settings updates, never on the render/input polling path.
use super::*;
use std::collections::HashMap;

pub(super) const SERVICE: &str = "shroudforge.keybinds";
pub(super) const INPUT_STATUS_UNKNOWN: u32 = 0;
pub(super) const INPUT_STATUS_INACTIVE: u32 = 1;
pub(super) const INPUT_STATUS_ACTIVE: u32 = 2;
pub(super) const INPUT_ADMISSION_UNKNOWN: u32 = 0;
pub(super) const INPUT_ADMISSION_BLOCKED: u32 = 1;
pub(super) const INPUT_ADMISSION_ACCEPTED: u32 = 2;

#[derive(Default)]
pub(super) struct Keybinds(HashMap<String, Binding>);

struct Binding {
    hotkey: Option<shroudforge_hotkeys::Hotkey>,
    previous_revision: Option<u64>,
}

impl Binding {
    fn sample(&mut self, revision: u64, down: bool, allowed: bool) -> u32 {
        let pressed = self
            .previous_revision
            .is_some_and(|previous| revision > previous);
        self.previous_revision = Some(revision);
        if allowed {
            u32::from(down) | (u32::from(pressed) << 1)
        } else {
            0
        }
    }
}

impl Keybinds {
    pub(super) fn update(
        &mut self,
        declared: &[mod_loader::SettingDefinition],
        json: &[u8],
    ) -> Result<(), String> {
        // Use the same declared values and enabledWhen policy as Lua keybinds.
        let current: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
        let mut values = serde_json::Map::new();
        for setting in declared {
            values.insert(
                setting.key.clone(),
                current
                    .get(&setting.key)
                    .unwrap_or(&setting.default)
                    .clone(),
            );
        }
        let values = serde_json::Value::Object(values);
        let mut next = HashMap::new();
        for setting in declared
            .iter()
            .filter(|s| matches!(s.control, Some(mod_loader::SettingControl::Keybind)))
        {
            let hotkey = if mod_loader::setting_is_enabled(&setting.key, declared, &values) {
                shroudforge_hotkeys::parse_value(&values[&setting.key])?
            } else {
                None
            };
            let previous_revision = self
                .0
                .get(&setting.key)
                .filter(|old| old.hotkey == hotkey)
                .and_then(|old| old.previous_revision);
            next.insert(
                setting.key.clone(),
                Binding {
                    hotkey,
                    previous_revision,
                },
            );
        }
        self.0 = next;
        Ok(())
    }

    fn read(&mut self, key: &str, allowed: bool) -> Result<u32, i32> {
        let binding = self.0.get_mut(key).ok_or(NATIVE_STATUS_NOT_FOUND)?;
        let Some(hotkey) = &binding.hotkey else {
            return Ok(0);
        };
        let revision = shroudforge_hotkeys::hotkey_press_revision(hotkey);
        let down = shroudforge_hotkeys::is_hotkey_down(hotkey);
        Ok(binding.sample(revision, down, allowed))
    }

    fn read_with_status(
        &mut self,
        key: &str,
        focus: Option<bool>,
        capture: Option<bool>,
    ) -> Result<(u32, u32, u32, u32), i32> {
        let binding = self.0.get_mut(key).ok_or(NATIVE_STATUS_NOT_FOUND)?;
        let focus_status = match focus {
            None => INPUT_STATUS_UNKNOWN,
            Some(false) => INPUT_STATUS_INACTIVE,
            Some(true) => INPUT_STATUS_ACTIVE,
        };
        let capture_status = match capture {
            None => INPUT_STATUS_UNKNOWN,
            Some(false) => INPUT_STATUS_INACTIVE,
            Some(true) => INPUT_STATUS_ACTIVE,
        };
        let admission = match (focus, capture) {
            (Some(false), _) | (_, Some(true)) => INPUT_ADMISSION_BLOCKED,
            (Some(true), Some(false)) => INPUT_ADMISSION_ACCEPTED,
            _ => INPUT_ADMISSION_UNKNOWN,
        };
        let flags = if let Some(hotkey) = &binding.hotkey {
            let revision = shroudforge_hotkeys::hotkey_press_revision(hotkey);
            let down = shroudforge_hotkeys::is_hotkey_down(hotkey);
            binding.sample(revision, down, admission == INPUT_ADMISSION_ACCEPTED)
        } else {
            0
        };
        Ok((flags, focus_status, capture_status, admission))
    }
}

#[repr(C)]
pub(super) struct KeybindState {
    pub(super) size: u32,
    pub(super) flags: u32,
}

#[repr(C)]
pub(super) struct KeybindStateV2 {
    pub(super) size: u32,
    pub(super) flags: u32,
    pub(super) focus_status: u32,
    pub(super) capture_status: u32,
    pub(super) admission_status: u32,
}

#[repr(C)]
pub(super) struct KeybindService {
    size: u32,
    version: u32,
    context: *mut c_void,
    pub(super) read:
        Option<unsafe extern "C" fn(*mut c_void, *const u8, u32, u32, *mut KeybindState) -> i32>,
}

#[repr(C)]
pub(super) struct KeybindServiceV2 {
    size: u32,
    version: u32,
    context: *mut c_void,
    pub(super) read:
        Option<unsafe extern "C" fn(*mut c_void, *const u8, u32, u32, *mut KeybindStateV2) -> i32>,
}

pub(super) unsafe fn query_v1(context: *mut c_void, capacity: u32, output: *mut c_void) -> i32 {
    if capacity < size_of::<KeybindService>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    unsafe {
        output.cast::<KeybindService>().write(KeybindService {
            size: size_of::<KeybindService>() as u32,
            version: 1,
            context,
            read: Some(read),
        });
    }
    NATIVE_STATUS_OK
}

unsafe extern "C" fn read(
    context: *mut c_void,
    key: *const u8,
    length: u32,
    capacity: u32,
    output: *mut KeybindState,
) -> i32 {
    let Some(context) = (unsafe { host_context(context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !context.is_owner_thread() {
        return NATIVE_STATUS_WRONG_THREAD;
    }
    if context.services_revoked.get() {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    if !context.settings_service_available() {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    if capacity < size_of::<KeybindState>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() || key.is_null() || length == 0 || length > 256 {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let Ok(key) = std::str::from_utf8(unsafe { std::slice::from_raw_parts(key, length as usize) })
    else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    let Ok(mut bindings) = context.keybinds.try_borrow_mut() else {
        return NATIVE_STATUS_BUSY;
    };
    let allowed =
        crate::lua::shroudforge::v1::is_game_focused() && !shroudforge_hotkeys::is_capture_active();
    match bindings.read(key, allowed) {
        Ok(flags) => {
            unsafe {
                output.write(KeybindState {
                    size: size_of::<KeybindState>() as u32,
                    flags,
                });
            }
            NATIVE_STATUS_OK
        }
        Err(status) => status,
    }
}

pub(super) unsafe fn query_v2(context: *mut c_void, capacity: u32, output: *mut c_void) -> i32 {
    if capacity < size_of::<KeybindServiceV2>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    unsafe {
        output.cast::<KeybindServiceV2>().write(KeybindServiceV2 {
            size: size_of::<KeybindServiceV2>() as u32,
            version: 2,
            context,
            read: Some(read_v2),
        });
    }
    NATIVE_STATUS_OK
}

unsafe extern "C" fn read_v2(
    context: *mut c_void,
    key: *const u8,
    length: u32,
    capacity: u32,
    output: *mut KeybindStateV2,
) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        read_v2_inner(context, key, length, capacity, output)
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

unsafe fn read_v2_inner(
    context: *mut c_void,
    key: *const u8,
    length: u32,
    capacity: u32,
    output: *mut KeybindStateV2,
) -> i32 {
    let Some(context) = (unsafe { host_context(context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !context.is_owner_thread() {
        return NATIVE_STATUS_WRONG_THREAD;
    }
    if context.services_revoked.get() {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    if !context.settings_service_available() {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    if capacity < size_of::<KeybindStateV2>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() || key.is_null() || length == 0 || length > 256 {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let key_bytes = unsafe { std::slice::from_raw_parts(key, length as usize) };
    let Ok(key) = std::str::from_utf8(key_bytes) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    let Ok(mut bindings) = context.keybinds.try_borrow_mut() else {
        return NATIVE_STATUS_BUSY;
    };
    let focus = crate::lua::shroudforge::v1::try_is_game_focused();
    let capture = shroudforge_hotkeys::capture_active_status();
    match bindings.read_with_status(key, focus, capture) {
        Ok((flags, focus_status, capture_status, admission_status)) => {
            unsafe {
                output.write(KeybindStateV2 {
                    size: size_of::<KeybindStateV2>() as u32,
                    flags,
                    focus_status,
                    capture_status,
                    admission_status,
                });
            }
            NATIVE_STATUS_OK
        }
        Err(status) => status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_rebind_disable_and_chords_share_sdl_parser() {
        let definitions: Vec<mod_loader::SettingDefinition> = serde_json::from_value(serde_json::json!([
            {"key":"enabled","type":"boolean","control":"toggle","label":"Enabled","default":true},
            {"key":"toggle","type":"string","control":"keybind","label":"Toggle","default":"SDL:67",
             "enabledWhen":{"setting":"enabled","equals":true}}
        ])).unwrap();
        let mut keys = Keybinds::default();
        keys.update(&definitions, b"{}").unwrap();
        assert_eq!(
            keys.0["toggle"].hotkey.as_ref().unwrap().normalized(),
            "SDL:67"
        );
        keys.0.get_mut("toggle").unwrap().sample(20, false, true);
        keys.update(&definitions, br#"{"enabled":true}"#).unwrap();
        assert_eq!(keys.0["toggle"].previous_revision, Some(20));
        keys.update(&definitions, br#"{"toggle":"Ctrl+SDL:68|Mouse:4"}"#)
            .unwrap();
        assert_eq!(keys.0["toggle"].previous_revision, None);
        assert_eq!(
            keys.0["toggle"].hotkey.as_ref().unwrap().normalized(),
            "Ctrl+SDL:68|Mouse:4"
        );
        assert_eq!(keys.0.get_mut("toggle").unwrap().sample(21, false, true), 0);
        keys.update(&definitions, br#"{"enabled":false}"#).unwrap();
        assert!(keys.0["toggle"].hotkey.is_none());
        assert_eq!(keys.read("toggle", true), Ok(0));
        keys.update(&definitions, br#"{"toggle":""}"#).unwrap();
        assert!(keys.0["toggle"].hotkey.is_none());
        assert!(keys.update(&definitions, br#"{"toggle":"F10"}"#).is_err());
        assert!(keys.0["toggle"].hotkey.is_none()); // failed update is atomic
        assert_eq!(keys.read("foreign", true), Err(NATIVE_STATUS_NOT_FOUND));
    }

    #[test]
    fn presses_are_consumed_once_and_suppressed_during_capture_or_focus_loss() {
        let mut binding = Binding {
            hotkey: None,
            previous_revision: None,
        };
        assert_eq!(binding.sample(4, false, true), 0); // initial baseline
        assert_eq!(binding.sample(5, false, true), 2); // tap survived between polls
        assert_eq!(binding.sample(5, true, true), 1); // held, no second edge
        assert_eq!(binding.sample(6, true, false), 0); // capture or background
        assert_eq!(binding.sample(6, false, true), 0); // never replay it
        assert_eq!(binding.sample(7, true, true), 3);
    }

    #[test]
    fn v2_reports_unknown_blocked_and_accepted_input_states_separately() {
        let definitions: Vec<mod_loader::SettingDefinition> =
            serde_json::from_value(serde_json::json!([{
                "key":"toggle","type":"string","control":"keybind",
                "label":"Toggle","default":"SDL:67"
            }]))
            .unwrap();
        let mut keys = Keybinds::default();
        keys.update(&definitions, b"{}").unwrap();

        let (_, focus, capture, admission) =
            keys.read_with_status("toggle", None, Some(false)).unwrap();
        assert_eq!(
            (focus, capture, admission),
            (
                INPUT_STATUS_UNKNOWN,
                INPUT_STATUS_INACTIVE,
                INPUT_ADMISSION_UNKNOWN,
            )
        );

        let (_, focus, capture, admission) =
            keys.read_with_status("toggle", Some(false), None).unwrap();
        assert_eq!(
            (focus, capture, admission),
            (
                INPUT_STATUS_INACTIVE,
                INPUT_STATUS_UNKNOWN,
                INPUT_ADMISSION_BLOCKED,
            )
        );

        let (_, focus, capture, admission) = keys
            .read_with_status("toggle", Some(true), Some(true))
            .unwrap();
        assert_eq!(
            (focus, capture, admission),
            (
                INPUT_STATUS_ACTIVE,
                INPUT_STATUS_ACTIVE,
                INPUT_ADMISSION_BLOCKED,
            )
        );

        let (_, focus, capture, admission) = keys
            .read_with_status("toggle", Some(true), Some(false))
            .unwrap();
        assert_eq!(
            (focus, capture, admission),
            (
                INPUT_STATUS_ACTIVE,
                INPUT_STATUS_INACTIVE,
                INPUT_ADMISSION_ACCEPTED,
            )
        );
    }

    #[test]
    fn unknown_and_blocked_presses_are_consumed_before_input_becomes_accepted() {
        let mut binding = Binding {
            hotkey: None,
            previous_revision: None,
        };
        assert_eq!(binding.sample(20, false, true), 0); // initial baseline
        assert_eq!(binding.sample(21, false, false), 0); // unknown or blocked edge
        assert_eq!(binding.sample(21, true, true), 1); // edge was consumed; only held is reported
        assert_eq!(binding.sample(22, false, true), 2); // new accepted edge
    }
}
