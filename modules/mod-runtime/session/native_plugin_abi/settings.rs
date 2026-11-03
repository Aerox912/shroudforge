use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub(super) const SERVICE: &str = "shroudforge.settings";
const MAX_REVISION_BYTES: usize = 64;

#[repr(C)]
pub(super) struct SettingsService {
    size: u32,
    pub(super) version: u32,
    context: *mut c_void,
    pub(super) read_snapshot:
        Option<unsafe extern "C" fn(*mut c_void, *mut u8, u32, *mut u32) -> i32>,
    pub(super) patch:
        Option<unsafe extern "C" fn(*mut c_void, *const u8, u32, *const u8, u32) -> i32>,
}

pub(super) unsafe fn query(context: *mut c_void, capacity: u32, output: *mut c_void) -> i32 {
    if capacity < size_of::<SettingsService>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    unsafe {
        output.cast::<SettingsService>().write(SettingsService {
            size: size_of::<SettingsService>() as u32,
            version: 1,
            context,
            read_snapshot: Some(read_snapshot),
            patch: Some(patch),
        });
    }
    NATIVE_STATUS_OK
}

unsafe extern "C" fn read_snapshot(
    context: *mut c_void,
    destination: *mut u8,
    capacity: u32,
    out_required_bytes: *mut u32,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| unsafe {
        read_snapshot_inner(context, destination, capacity, out_required_bytes)
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

unsafe fn read_snapshot_inner(
    context: *mut c_void,
    destination: *mut u8,
    capacity: u32,
    out_required_bytes: *mut u32,
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
    if out_required_bytes.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }

    let snapshot = match mod_loader::read_bound_mod_settings_snapshot(
        &context.installation_root,
        &context.package_binding,
    ) {
        Ok(snapshot) => snapshot,
        Err(_) => return NATIVE_STATUS_FAILED,
    };
    if !snapshot.values.is_object() {
        return NATIVE_STATUS_FAILED;
    }
    let bytes = match serde_json::to_vec(&serde_json::json!({
        "revision": snapshot.revision,
        "values": snapshot.values,
    })) {
        Ok(bytes) if bytes.len() <= NATIVE_MAX_SETTINGS_JSON_BYTES => bytes,
        Ok(_) => {
            unsafe { out_required_bytes.write(0) };
            return NATIVE_STATUS_TOO_LARGE;
        }
        Err(_) => return NATIVE_STATUS_FAILED,
    };
    unsafe { out_required_bytes.write(bytes.len() as u32) };
    if capacity < bytes.len() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if destination.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
    NATIVE_STATUS_OK
}

unsafe extern "C" fn patch(
    context: *mut c_void,
    expected_revision_utf8: *const u8,
    expected_revision_length: u32,
    patch_json_utf8: *const u8,
    patch_json_length: u32,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| unsafe {
        patch_inner(
            context,
            expected_revision_utf8,
            expected_revision_length,
            patch_json_utf8,
            patch_json_length,
        )
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

unsafe fn patch_inner(
    context: *mut c_void,
    expected_revision_utf8: *const u8,
    expected_revision_length: u32,
    patch_json_utf8: *const u8,
    patch_json_length: u32,
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
    let revision_length = expected_revision_length as usize;
    let patch_length = patch_json_length as usize;
    if expected_revision_utf8.is_null()
        || patch_json_utf8.is_null()
        || revision_length != MAX_REVISION_BYTES
        || patch_length == 0
    {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    if patch_length > NATIVE_MAX_SETTINGS_JSON_BYTES {
        return NATIVE_STATUS_TOO_LARGE;
    }
    let revision_bytes =
        unsafe { std::slice::from_raw_parts(expected_revision_utf8, revision_length) };
    if !revision_bytes.iter().all(u8::is_ascii_hexdigit) {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let revision = match std::str::from_utf8(revision_bytes) {
        Ok(revision) => revision,
        Err(_) => return NATIVE_STATUS_INVALID_ARGUMENT,
    };
    let patch_bytes = unsafe { std::slice::from_raw_parts(patch_json_utf8, patch_length) };
    if std::str::from_utf8(patch_bytes).is_err() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let values = match serde_json::from_slice::<serde_json::Value>(patch_bytes) {
        Ok(serde_json::Value::Object(values)) => serde_json::Value::Object(values),
        Ok(_) => return NATIVE_STATUS_INVALID_ARGUMENT,
        Err(_) => return NATIVE_STATUS_INVALID_ARGUMENT,
    };
    match mod_loader::save_bound_mod_settings_with_status(
        &context.installation_root,
        &context.package_binding,
        &values,
        revision,
    ) {
        Ok(()) => NATIVE_STATUS_OK,
        Err(mod_loader::ModConfigurationError::RevisionConflict) => NATIVE_STATUS_CONFLICT,
        Err(mod_loader::ModConfigurationError::Failed(_)) => NATIVE_STATUS_FAILED,
    }
}
