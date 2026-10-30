use super::Mod;
use super::native_plugin_lifecycle::{
    InitializationFailure, LifecycleCallbacks, TeardownOutcome, initialize_or_rollback, teardown,
};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    io::Read,
    mem::size_of,
    sync::Arc,
    thread::ThreadId,
};

pub(crate) const NATIVE_PLUGIN_ABI_V1: u32 = 1;
pub(crate) const NATIVE_HOST_ABI_V1: u32 = 1;
pub(crate) const NATIVE_STATUS_OK: i32 = 0;
pub(crate) const NATIVE_MAX_SETTINGS_JSON_BYTES: usize = 65_536;
pub(crate) const NATIVE_MAX_ASSET_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const NATIVE_MAX_LOG_BYTES: usize = 4_096;
pub(crate) const NATIVE_DRAIN_TIMEOUT_MS: u32 = 250;
pub(crate) const NATIVE_SERVICE_MAP_V1: &str = "shroudforge.map";
pub(crate) const NATIVE_SERVICE_OVERLAY_V1: &str = "shroudforge.overlay";

pub(crate) const NATIVE_STATUS_INVALID_ARGUMENT: i32 = 1;
pub(crate) const NATIVE_STATUS_ABI_MISMATCH: i32 = 2;
pub(crate) const NATIVE_STATUS_UNAVAILABLE: i32 = 3;
pub(crate) const NATIVE_STATUS_UNKNOWN_SERVICE: i32 = 4;
pub(crate) const NATIVE_STATUS_BUFFER_TOO_SMALL: i32 = 5;
pub(crate) const NATIVE_STATUS_BUSY: i32 = 6;
pub(crate) const NATIVE_STATUS_FAILED: i32 = 7;
pub(crate) const NATIVE_STATUS_WRONG_THREAD: i32 = 8;
pub(crate) const NATIVE_STATUS_NOT_FOUND: i32 = 9;
pub(crate) const NATIVE_STATUS_TOO_LARGE: i32 = 10;

const NATIVE_TARGET_CLIENT: u32 = 1;
const NATIVE_TARGET_SERVER: u32 = 2;
const MAX_PACKAGE_ID_BYTES: usize = 256;
const MAX_SERVICE_NAME_BYTES: usize = 64;
pub(crate) const NATIVE_MAX_ASSET_PATH_BYTES: usize = 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativePluginMode {
    LegacyLoadOnly,
    ShroudForgeV1,
}

pub(crate) type NativePluginEntry = unsafe extern "C" fn(u32, u32, *mut SFNativePluginV1) -> i32;
type NativeInitializeFn = unsafe extern "C" fn(*mut c_void, *const SFNativeHostV1) -> i32;
type NativeUpdateFn = unsafe extern "C" fn(*mut c_void, *const SFNativeHostV1, u64) -> i32;
type NativeStopFn = unsafe extern "C" fn(*mut c_void, *const SFNativeHostV1) -> i32;
type NativeShutdownFn = unsafe extern "C" fn(*mut c_void, *const SFNativeHostV1) -> i32;
type NativeLogFn = unsafe extern "C" fn(*mut c_void, u32, *const u8, u32);
type NativeReadSettingsFn = unsafe extern "C" fn(*mut c_void, *mut u8, u32, *mut u32) -> i32;
type NativeReadAssetFn =
    unsafe extern "C" fn(*mut c_void, *const u8, u32, *mut u8, u32, *mut u32) -> i32;
type NativeQueryServiceFn =
    unsafe extern "C" fn(*mut c_void, *const u8, u32, u32, u32, *mut c_void) -> i32;
type NativeRevokeServicesFn = unsafe extern "C" fn(*mut c_void, u64) -> i32;
type NativeDrainServicesFn = unsafe extern "C" fn(*mut c_void, u64, u32) -> i32;

#[repr(C)]
pub(crate) struct SFNativePluginV1 {
    struct_size: u32,
    abi_version: u32,
    plugin_context: *mut c_void,
    initialize: Option<NativeInitializeFn>,
    update: Option<NativeUpdateFn>,
    stop: Option<NativeStopFn>,
    shutdown: Option<NativeShutdownFn>,
}

impl Default for SFNativePluginV1 {
    fn default() -> Self {
        Self {
            struct_size: 0,
            abi_version: 0,
            plugin_context: std::ptr::null_mut(),
            initialize: None,
            update: None,
            stop: None,
            shutdown: None,
        }
    }
}

#[repr(C)]
struct SFNativeHostV1 {
    struct_size: u32,
    abi_version: u32,
    package_id_utf8: *const u8,
    package_id_length: u32,
    package_generation: u64,
    process_target: u32,
    reserved: u32,
    package_context: *mut c_void,
    log: Option<NativeLogFn>,
    read_settings_json: Option<NativeReadSettingsFn>,
    read_asset: Option<NativeReadAssetFn>,
    query_service: Option<NativeQueryServiceFn>,
    revoke_services: Option<NativeRevokeServicesFn>,
    drain_services: Option<NativeDrainServicesFn>,
}

pub(crate) struct NativePluginHostMetadata {
    pub(crate) package_id: String,
    pub(crate) package: Mod,
    pub(crate) package_generation: u64,
    pub(crate) is_server: bool,
    pub(crate) settings_json: Vec<u8>,
    pub(crate) services: Arc<dyn NativePluginServiceLifetime>,
}

/// Runtime-owned gate for callbacks registered through host services.
/// Implementations revoke by package identity and generation, then report BUSY
/// from drain until every callback acquired through that owner has exited.
pub(crate) trait NativePluginServiceLifetime: Send + Sync {
    /// Write the requested service table only when `out_capacity` is large
    /// enough. Implementations must not touch AppState or Lua from callbacks.
    unsafe fn query_service(
        &self,
        package_id: &str,
        package_generation: u64,
        service_name: &str,
        requested_version: u32,
        out_capacity: u32,
        out_table: *mut c_void,
    ) -> i32;
    fn revoke(&self, package_id: &str, package_generation: u64) -> i32;
    fn drain(&self, package_id: &str, package_generation: u64, timeout_ms: u32) -> i32;
}

pub(crate) struct NoNativePluginServices;

impl NativePluginServiceLifetime for NoNativePluginServices {
    unsafe fn query_service(
        &self,
        _package_id: &str,
        _package_generation: u64,
        _service_name: &str,
        _requested_version: u32,
        _out_capacity: u32,
        _out_table: *mut c_void,
    ) -> i32 {
        NATIVE_STATUS_UNAVAILABLE
    }

    fn revoke(&self, _package_id: &str, _package_generation: u64) -> i32 {
        NATIVE_STATUS_OK
    }

    fn drain(&self, _package_id: &str, _package_generation: u64, _timeout_ms: u32) -> i32 {
        NATIVE_STATUS_OK
    }
}

struct NativePluginHostContext {
    package_id_utf8: Box<[u8]>,
    package: Mod,
    package_generation: u64,
    process_target: u32,
    owner_thread: ThreadId,
    services_revoked: Cell<bool>,
    settings_json: RefCell<Vec<u8>>,
    services: Arc<dyn NativePluginServiceLifetime>,
}

impl NativePluginHostContext {
    fn new(metadata: NativePluginHostMetadata) -> Result<Self, String> {
        let package_id_utf8 = metadata.package_id.into_bytes().into_boxed_slice();
        if package_id_utf8.is_empty() || package_id_utf8.len() > MAX_PACKAGE_ID_BYTES {
            return Err("native plugin package ID exceeds the ABI bound".into());
        }
        if metadata.package.info().id.as_bytes() != package_id_utf8.as_ref() {
            return Err("native plugin package identity does not match its package".into());
        }
        if metadata.settings_json.len() > NATIVE_MAX_SETTINGS_JSON_BYTES {
            return Err("native plugin settings JSON exceeds 65536 bytes".into());
        }

        Ok(Self {
            package_id_utf8,
            package: metadata.package,
            package_generation: metadata.package_generation,
            process_target: if metadata.is_server {
                NATIVE_TARGET_SERVER
            } else {
                NATIVE_TARGET_CLIENT
            },
            owner_thread: std::thread::current().id(),
            services_revoked: Cell::new(false),
            settings_json: RefCell::new(metadata.settings_json),
            services: metadata.services,
        })
    }

    fn host_table(&mut self) -> SFNativeHostV1 {
        SFNativeHostV1 {
            struct_size: size_of::<SFNativeHostV1>() as u32,
            abi_version: NATIVE_HOST_ABI_V1,
            package_id_utf8: self.package_id_utf8.as_ptr(),
            package_id_length: self.package_id_utf8.len() as u32,
            package_generation: self.package_generation,
            process_target: self.process_target,
            reserved: 0,
            package_context: (self as *mut Self).cast(),
            log: Some(host_log),
            read_settings_json: Some(host_read_settings_json),
            read_asset: Some(host_read_asset),
            query_service: Some(host_query_service),
            revoke_services: Some(host_revoke_services),
            drain_services: Some(host_drain_services),
        }
    }

    fn is_owner_thread(&self) -> bool {
        std::thread::current().id() == self.owner_thread
    }

    fn update_settings_json(&self, settings_json: Vec<u8>) -> Result<(), String> {
        if !self.is_owner_thread() {
            return Err(
                "native plugin settings must be updated on the runtime owner thread".into(),
            );
        }
        if settings_json.len() > NATIVE_MAX_SETTINGS_JSON_BYTES {
            return Err("native plugin settings JSON exceeds 65536 bytes".into());
        }
        let mut current = self
            .settings_json
            .try_borrow_mut()
            .map_err(|_| "native plugin settings are currently being read".to_owned())?;
        *current = settings_json;
        Ok(())
    }
}

pub(crate) struct NativePluginInstance {
    plugin: SFNativePluginV1,
    host_context: Box<NativePluginHostContext>,
    host: Box<SFNativeHostV1>,
    state: NativePluginInstanceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativePluginInstanceState {
    New,
    Active,
    Stopped,
    Retained,
}

impl NativePluginInstance {
    pub(crate) fn load(
        entry: NativePluginEntry,
        metadata: NativePluginHostMetadata,
    ) -> Result<Self, String> {
        let mut host_context = Box::new(NativePluginHostContext::new(metadata)?);
        let host = Box::new(host_context.host_table());
        let mut plugin = SFNativePluginV1::default();
        let out_size = size_of::<SFNativePluginV1>() as u32;
        let status = unsafe { entry(NATIVE_PLUGIN_ABI_V1, out_size, &mut plugin) };
        if status != NATIVE_STATUS_OK {
            return Err(format!(
                "ShroudForgePluginEntry rejected ABI v1: {}",
                native_status_description(status)
            ));
        }
        validate_plugin_table(out_size, &plugin)?;
        Ok(Self {
            plugin,
            host_context,
            host,
            state: NativePluginInstanceState::New,
        })
    }

    pub(crate) fn start(&mut self) -> Result<(), InitializationFailure> {
        if self.state != NativePluginInstanceState::New {
            return Err(InitializationFailure {
                status: NATIVE_STATUS_FAILED,
                cleanup: TeardownOutcome::Retain {
                    stage: "duplicate initialization",
                    status: NATIVE_STATUS_FAILED,
                },
            });
        }
        match initialize_or_rollback(self) {
            Ok(()) => {
                self.state = NativePluginInstanceState::Active;
                Ok(())
            }
            Err(failure) => {
                self.state = match failure.cleanup {
                    TeardownOutcome::Quiescent => NativePluginInstanceState::Stopped,
                    TeardownOutcome::Retain { .. } => NativePluginInstanceState::Retained,
                };
                Err(failure)
            }
        }
    }

    pub(crate) fn update(&mut self, elapsed_microseconds: u64) -> i32 {
        if self.state != NativePluginInstanceState::Active {
            return NATIVE_STATUS_FAILED;
        }
        unsafe {
            self.plugin
                .update
                .expect("validated plugin update callback")(
                self.plugin.plugin_context,
                self.host.as_ref(),
                elapsed_microseconds,
            )
        }
    }

    pub(crate) fn stop(&mut self) -> TeardownOutcome {
        if self.state != NativePluginInstanceState::Active {
            return TeardownOutcome::Retain {
                stage: "plugin is not active",
                status: NATIVE_STATUS_FAILED,
            };
        }
        let result = teardown(self);
        self.state = match result {
            TeardownOutcome::Quiescent => NativePluginInstanceState::Stopped,
            TeardownOutcome::Retain { .. } => NativePluginInstanceState::Retained,
        };
        result
    }

    pub(crate) fn update_settings_json(&self, settings_json: Vec<u8>) -> Result<(), String> {
        if self.state != NativePluginInstanceState::Active {
            return Err("native plugin is not active".into());
        }
        self.host_context.update_settings_json(settings_json)
    }
}

impl LifecycleCallbacks for NativePluginInstance {
    fn initialize(&mut self) -> i32 {
        unsafe {
            self.plugin
                .initialize
                .expect("validated plugin initialize callback")(
                self.plugin.plugin_context,
                self.host.as_ref(),
            )
        }
    }

    fn revoke_services(&mut self) -> i32 {
        let revoke = self.host.revoke_services.expect("host revoke callback");
        unsafe { revoke(self.host.package_context, self.host.package_generation) }
    }

    fn drain_services(&mut self, timeout_ms: u32) -> i32 {
        let drain = self.host.drain_services.expect("host drain callback");
        unsafe {
            drain(
                self.host.package_context,
                self.host.package_generation,
                timeout_ms,
            )
        }
    }

    fn stop(&mut self) -> i32 {
        unsafe {
            self.plugin.stop.expect("validated plugin stop callback")(
                self.plugin.plugin_context,
                self.host.as_ref(),
            )
        }
    }

    fn shutdown(&mut self) -> i32 {
        unsafe {
            self.plugin
                .shutdown
                .expect("validated plugin shutdown callback")(
                self.plugin.plugin_context,
                self.host.as_ref(),
            )
        }
    }
}

pub(crate) fn entrypoint_for_mode(
    mode: NativePluginMode,
    lookup: impl FnOnce() -> Option<NativePluginEntry>,
) -> Result<Option<NativePluginEntry>, String> {
    match mode {
        NativePluginMode::LegacyLoadOnly => Ok(None),
        NativePluginMode::ShroudForgeV1 => lookup()
            .map(Some)
            .ok_or_else(|| "opt-in native plugin is missing ShroudForgePluginEntry".into()),
    }
}

fn validate_plugin_table(out_size: u32, plugin: &SFNativePluginV1) -> Result<(), String> {
    let required_size = size_of::<SFNativePluginV1>() as u32;
    if out_size < required_size
        || plugin.struct_size < required_size
        || plugin.struct_size > out_size
    {
        return Err(format!(
            "native plugin ABI v1 table size {} is incompatible with host size {out_size}",
            plugin.struct_size
        ));
    }
    if plugin.abi_version != NATIVE_PLUGIN_ABI_V1 {
        return Err(format!(
            "native plugin ABI version {} is unsupported; expected {}",
            plugin.abi_version, NATIVE_PLUGIN_ABI_V1
        ));
    }
    if plugin.initialize.is_none()
        || plugin.update.is_none()
        || plugin.stop.is_none()
        || plugin.shutdown.is_none()
    {
        return Err("native plugin ABI v1 table is missing a required callback".into());
    }
    Ok(())
}

fn native_status_description(status: i32) -> &'static str {
    match status {
        NATIVE_STATUS_OK => "success",
        NATIVE_STATUS_INVALID_ARGUMENT => "invalid argument",
        NATIVE_STATUS_ABI_MISMATCH => "ABI mismatch",
        NATIVE_STATUS_UNAVAILABLE => "capability unavailable",
        NATIVE_STATUS_UNKNOWN_SERVICE => "unknown service",
        NATIVE_STATUS_BUFFER_TOO_SMALL => "buffer too small",
        NATIVE_STATUS_BUSY => "busy",
        NATIVE_STATUS_FAILED => "failed",
        NATIVE_STATUS_WRONG_THREAD => "wrong thread",
        NATIVE_STATUS_NOT_FOUND => "not found",
        NATIVE_STATUS_TOO_LARGE => "asset exceeds the read limit",
        _ => "unknown status",
    }
}

unsafe fn host_context<'a>(package_context: *mut c_void) -> Option<&'a NativePluginHostContext> {
    if package_context.is_null() {
        None
    } else {
        Some(unsafe { &*package_context.cast::<NativePluginHostContext>() })
    }
}

unsafe extern "C" fn host_log(
    package_context: *mut c_void,
    level: u32,
    message_utf8: *const u8,
    message_length: u32,
) {
    let Some(context) = (unsafe { host_context(package_context) }) else {
        return;
    };
    if message_length > 0 && message_utf8.is_null() {
        return;
    }
    let length = (message_length as usize).min(NATIVE_MAX_LOG_BYTES);
    let bytes = if length == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(message_utf8, length) }
    };
    let message = String::from_utf8_lossy(bytes);
    let mod_id = std::str::from_utf8(&context.package_id_utf8).unwrap_or("<invalid>");
    match level {
        1 => tracing::error!(target: "shroudforge::native_plugin", mod_id, message = %message),
        2 => tracing::warn!(target: "shroudforge::native_plugin", mod_id, message = %message),
        3 => tracing::info!(target: "shroudforge::native_plugin", mod_id, message = %message),
        4 => tracing::debug!(target: "shroudforge::native_plugin", mod_id, message = %message),
        5 => tracing::trace!(target: "shroudforge::native_plugin", mod_id, message = %message),
        _ => {
            tracing::warn!(target: "shroudforge::native_plugin", mod_id, "Plugin used an unknown log level")
        }
    }
}

unsafe extern "C" fn host_read_settings_json(
    package_context: *mut c_void,
    destination: *mut u8,
    capacity: u32,
    out_required_bytes: *mut u32,
) -> i32 {
    let Some(context) = (unsafe { host_context(package_context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !context.is_owner_thread() {
        return NATIVE_STATUS_WRONG_THREAD;
    }
    if out_required_bytes.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let Ok(settings_json) = context.settings_json.try_borrow() else {
        return NATIVE_STATUS_BUSY;
    };
    unsafe { out_required_bytes.write(settings_json.len() as u32) };
    if capacity < settings_json.len() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if !settings_json.is_empty() && destination.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    if !settings_json.is_empty() {
        unsafe {
            std::ptr::copy_nonoverlapping(settings_json.as_ptr(), destination, settings_json.len())
        };
    }
    NATIVE_STATUS_OK
}

unsafe extern "C" fn host_read_asset(
    package_context: *mut c_void,
    relative_path_utf8: *const u8,
    relative_path_length: u32,
    destination: *mut u8,
    capacity: u32,
    out_required_bytes: *mut u32,
) -> i32 {
    let Some(context) = (unsafe { host_context(package_context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !context.is_owner_thread() {
        return NATIVE_STATUS_WRONG_THREAD;
    }
    if out_required_bytes.is_null()
        || relative_path_utf8.is_null()
        || relative_path_length == 0
        || relative_path_length as usize > NATIVE_MAX_ASSET_PATH_BYTES
    {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let path_bytes =
        unsafe { std::slice::from_raw_parts(relative_path_utf8, relative_path_length as usize) };
    let Ok(path_str) = std::str::from_utf8(path_bytes) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    let relative_path = camino::Utf8Path::new(path_str);
    if path_str.contains('\0')
        || path_str.contains(':')
        || relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| matches!(component.as_str(), "." | ".."))
    {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }

    let mut filesystem = context.package.fs();
    let mut reader = match filesystem.read_file(relative_path) {
        Ok(reader) => reader,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return NATIVE_STATUS_NOT_FOUND;
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return NATIVE_STATUS_INVALID_ARGUMENT;
        }
        Err(_) => return NATIVE_STATUS_FAILED,
    };
    let mut bytes = Vec::new();
    if reader
        .by_ref()
        .take((NATIVE_MAX_ASSET_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return NATIVE_STATUS_FAILED;
    }
    if bytes.len() > NATIVE_MAX_ASSET_BYTES {
        unsafe { out_required_bytes.write(0) };
        return NATIVE_STATUS_TOO_LARGE;
    }
    unsafe { out_required_bytes.write(bytes.len() as u32) };
    if capacity < bytes.len() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if !bytes.is_empty() && destination.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    if !bytes.is_empty() {
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
    }
    NATIVE_STATUS_OK
}

unsafe extern "C" fn host_query_service(
    package_context: *mut c_void,
    service_name_utf8: *const u8,
    service_name_length: u32,
    requested_version: u32,
    out_capacity: u32,
    out_table: *mut c_void,
) -> i32 {
    let Some(context) = (unsafe { host_context(package_context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !context.is_owner_thread() {
        return NATIVE_STATUS_WRONG_THREAD;
    }
    if context.services_revoked.get() {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    let length = service_name_length as usize;
    if length == 0 || length > MAX_SERVICE_NAME_BYTES || service_name_utf8.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let name = unsafe { std::slice::from_raw_parts(service_name_utf8, length) };
    let Ok(name) = std::str::from_utf8(name) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !matches!(name, NATIVE_SERVICE_MAP_V1 | NATIVE_SERVICE_OVERLAY_V1) {
        return NATIVE_STATUS_UNKNOWN_SERVICE;
    }
    if requested_version != 1 {
        return NATIVE_STATUS_ABI_MISMATCH;
    }
    // The lifecycle owner has already revalidated enabled package identity
    // before initialization. Check this generation's immutable permissions here,
    // rather than a startup allowlist that would reject later enabled packages.
    if context.process_target != NATIVE_TARGET_CLIENT
        || !context.package.info().target.supports_process(false)
        || !context.package.info().capabilities.contains(&mod_loader::Capability::Runtime)
    {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    let package_id = std::str::from_utf8(&context.package_id_utf8).unwrap_or("");
    unsafe {
        context.services.query_service(
            package_id,
            context.package_generation,
            name,
            requested_version,
            out_capacity,
            out_table,
        )
    }
}

unsafe extern "C" fn host_revoke_services(
    package_context: *mut c_void,
    package_generation: u64,
) -> i32 {
    let Some(context) = (unsafe { host_context(package_context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !context.is_owner_thread() {
        return NATIVE_STATUS_WRONG_THREAD;
    }
    if context.package_generation != package_generation {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    // Close the SDK table before delegating, even if durable native revoke
    // fails and teardown must retain this generation's module/context.
    context.services_revoked.set(true);
    let package_id = std::str::from_utf8(&context.package_id_utf8).unwrap_or("");
    context.services.revoke(package_id, package_generation)
}

unsafe extern "C" fn host_drain_services(
    package_context: *mut c_void,
    package_generation: u64,
    timeout_ms: u32,
) -> i32 {
    let Some(context) = (unsafe { host_context(package_context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    if !context.is_owner_thread() {
        return NATIVE_STATUS_WRONG_THREAD;
    }
    if context.package_generation != package_generation || timeout_ms > NATIVE_DRAIN_TIMEOUT_MS {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let package_id = std::str::from_utf8(&context.package_id_utf8).unwrap_or("");
    context
        .services
        .drain(package_id, package_generation, timeout_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    struct TestPackage {
        package: Mod,
        root: PathBuf,
    }

    impl TestPackage {
        fn new() -> Self {Self::with_runtime(true)}
        fn with_runtime(runtime:bool) -> Self {
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let root = target.join(format!("native-plugin-test-{}", uuid::Uuid::new_v4()));
            let package_root = root.join("mods").join("example");
            fs::create_dir_all(package_root.join("assets")).unwrap();
            fs::write(
                package_root.join("mod.json"),
                serde_json::json!({"id":"example","name":"Example","version":"1.0.0",
                    "capabilities":if runtime {vec!["runtime-register-dll","runtime"]}else{vec!["runtime-register-dll"]}}).to_string(),
            )
            .unwrap();
            fs::write(
                package_root.join("extended.mod.json"),
                r#"{"schemaVersion":1,"enabled":true,"targets":["client"],"settings":{}}"#,
            )
            .unwrap();
            fs::write(package_root.join("assets").join("atlas.bin"), b"atlas").unwrap();
            let utf8_root = camino::Utf8Path::from_path(&root).unwrap();
            let environment = mod_loader::ModEnvironment::load(utf8_root).unwrap();
            let package = environment.mod_registry().get("example").unwrap().clone();
            Self { package, root }
        }
    }

    impl Drop for TestPackage {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct TestServiceLifetime {
        revoke_status: i32,
        drain_status: i32,
        queried_owner: Mutex<Option<(String, u64, String, u32, u32)>>,
        revoked_owner: Mutex<Option<(String, u64)>>,
        drained_owner: Mutex<Option<(String, u64, u32)>>,
    }

    impl NativePluginServiceLifetime for TestServiceLifetime {
        unsafe fn query_service(
            &self,
            package_id: &str,
            package_generation: u64,
            service_name: &str,
            requested_version: u32,
            out_capacity: u32,
            out_table: *mut c_void,
        ) -> i32 {
            *self.queried_owner.lock().unwrap() = Some((
                package_id.to_owned(),
                package_generation,
                service_name.to_owned(),
                requested_version,
                out_capacity,
            ));
            if out_capacity < size_of::<u32>() as u32 || out_table.is_null() {
                return NATIVE_STATUS_BUFFER_TOO_SMALL;
            }
            unsafe { out_table.cast::<u32>().write(0x5346_4D50) };
            NATIVE_STATUS_OK
        }

        fn revoke(&self, package_id: &str, package_generation: u64) -> i32 {
            *self.revoked_owner.lock().unwrap() = Some((package_id.to_owned(), package_generation));
            self.revoke_status
        }

        fn drain(&self, package_id: &str, package_generation: u64, timeout_ms: u32) -> i32 {
            *self.drained_owner.lock().unwrap() =
                Some((package_id.to_owned(), package_generation, timeout_ms));
            self.drain_status
        }
    }

    unsafe extern "C" fn unused_callback(
        _context: *mut c_void,
        _host: *const SFNativeHostV1,
    ) -> i32 {
        NATIVE_STATUS_OK
    }

    unsafe extern "C" fn unused_update(
        _context: *mut c_void,
        _host: *const SFNativeHostV1,
        _elapsed_microseconds: u64,
    ) -> i32 {
        NATIVE_STATUS_OK
    }

    unsafe extern "C" fn bounded_entry(
        requested_abi: u32,
        out_size: u32,
        out_plugin: *mut SFNativePluginV1,
    ) -> i32 {
        if requested_abi != NATIVE_PLUGIN_ABI_V1
            || out_size < size_of::<SFNativePluginV1>() as u32
            || out_plugin.is_null()
        {
            return NATIVE_STATUS_ABI_MISMATCH;
        }
        unsafe {
            out_plugin.write(SFNativePluginV1 {
                struct_size: size_of::<SFNativePluginV1>() as u32,
                abi_version: NATIVE_PLUGIN_ABI_V1,
                plugin_context: std::ptr::null_mut(),
                initialize: Some(unused_callback),
                update: Some(unused_update),
                stop: Some(unused_callback),
                shutdown: Some(unused_callback),
            })
        };
        NATIVE_STATUS_OK
    }

    fn valid_plugin_table() -> SFNativePluginV1 {
        SFNativePluginV1 {
            struct_size: size_of::<SFNativePluginV1>() as u32,
            abi_version: NATIVE_PLUGIN_ABI_V1,
            plugin_context: std::ptr::null_mut(),
            initialize: Some(unused_callback),
            update: Some(unused_update),
            stop: Some(unused_callback),
            shutdown: Some(unused_callback),
        }
    }

    #[test]
    fn legacy_load_only_mode_never_looks_up_an_entrypoint() {
        let mut looked_up = false;
        let entry = entrypoint_for_mode(NativePluginMode::LegacyLoadOnly, || {
            looked_up = true;
            None
        })
        .unwrap();

        assert!(entry.is_none());
        assert!(!looked_up);
    }

    #[test]
    fn explicit_mode_requires_the_entrypoint() {
        let error = entrypoint_for_mode(NativePluginMode::ShroudForgeV1, || None).unwrap_err();
        assert!(error.contains("missing ShroudForgePluginEntry"));
    }

    #[test]
    fn short_output_buffer_is_rejected_before_any_write() {
        #[repr(C)]
        struct GuardedOutput {
            output: SFNativePluginV1,
            sentinel: u64,
        }

        let mut guarded = GuardedOutput {
            output: SFNativePluginV1::default(),
            sentinel: 0xA5A5_5A5A_D3C3_B4B4,
        };
        let output_before = guarded.output.struct_size;
        let sentinel_before = guarded.sentinel;
        let status = unsafe {
            bounded_entry(
                NATIVE_PLUGIN_ABI_V1,
                size_of::<SFNativePluginV1>() as u32 - 1,
                &mut guarded.output,
            )
        };

        assert_eq!(status, NATIVE_STATUS_ABI_MISMATCH);
        assert_eq!(guarded.output.struct_size, output_before);
        assert_eq!(guarded.sentinel, sentinel_before);
    }

    #[test]
    fn plugin_table_requires_matching_version_size_and_callbacks() {
        let out_size = size_of::<SFNativePluginV1>() as u32;
        let valid = valid_plugin_table();
        assert!(validate_plugin_table(out_size, &valid).is_ok());

        let mut wrong_version = valid_plugin_table();
        wrong_version.abi_version = 2;
        assert!(
            validate_plugin_table(out_size, &wrong_version)
                .unwrap_err()
                .contains("version 2")
        );

        let mut short_table = valid_plugin_table();
        short_table.struct_size -= 1;
        assert!(
            validate_plugin_table(out_size, &short_table)
                .unwrap_err()
                .contains("table size")
        );

        let mut missing_callback = valid_plugin_table();
        missing_callback.shutdown = None;
        assert!(
            validate_plugin_table(out_size, &missing_callback)
                .unwrap_err()
                .contains("missing a required callback")
        );
    }

    #[test]
    fn current_map_services_are_explicitly_unavailable() {
        let package = TestPackage::new();
        let mut context = Box::new(
            NativePluginHostContext::new(NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_generation: 7,
                is_server: false,
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            })
            .unwrap(),
        );
        let host = context.host_table();
        let query = host.query_service.unwrap();
        for service_name in [NATIVE_SERVICE_MAP_V1, NATIVE_SERVICE_OVERLAY_V1] {
            let service_name = service_name.as_bytes();
            let result = unsafe {
                query(
                    host.package_context,
                    service_name.as_ptr(),
                    service_name.len() as u32,
                    1,
                    0,
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(result, NATIVE_STATUS_UNAVAILABLE);
        }

        let unknown_name = b"shroudforge.unknown";
        let unknown_result = unsafe {
            query(
                host.package_context,
                unknown_name.as_ptr(),
                unknown_name.len() as u32,
                1,
                0,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(unknown_result, NATIVE_STATUS_UNKNOWN_SERVICE);
    }

    #[test]
    fn settings_json_read_is_bounded_and_does_not_partially_write() {
        let package = TestPackage::new();
        let mut context = Box::new(
            NativePluginHostContext::new(NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_generation: 9,
                is_server: false,
                settings_json: br#"{"enabled":true}"#.to_vec(),
                services: Arc::new(NoNativePluginServices),
            })
            .unwrap(),
        );
        let host = context.host_table();
        let read_settings = host.read_settings_json.unwrap();
        let mut short = [0xA5; 2];
        let mut required = 0;
        let short_status = unsafe {
            read_settings(
                host.package_context,
                short.as_mut_ptr(),
                short.len() as u32,
                &mut required,
            )
        };
        assert_eq!(short_status, NATIVE_STATUS_BUFFER_TOO_SMALL);
        assert_eq!(required, br#"{"enabled":true}"#.len() as u32);
        assert_eq!(short, [0xA5; 2]);

        let mut full = vec![0; required as usize];
        let full_status = unsafe {
            read_settings(
                host.package_context,
                full.as_mut_ptr(),
                full.len() as u32,
                &mut required,
            )
        };
        assert_eq!(full_status, NATIVE_STATUS_OK);
        assert_eq!(full, br#"{"enabled":true}"#);
    }

    #[test]
    fn package_asset_reads_validate_paths_and_report_required_capacity() {
        let package = TestPackage::new();
        let mut context = Box::new(
            NativePluginHostContext::new(NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_generation: 8,
                is_server: false,
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            })
            .unwrap(),
        );
        let host = context.host_table();
        let read_asset = host.read_asset.unwrap();
        let path = b"assets/atlas.bin";
        let mut short_buffer = [0xA5; 2];
        let mut required_bytes = 0;

        let short_status = unsafe {
            read_asset(
                host.package_context,
                path.as_ptr(),
                path.len() as u32,
                short_buffer.as_mut_ptr(),
                short_buffer.len() as u32,
                &mut required_bytes,
            )
        };
        assert_eq!(short_status, NATIVE_STATUS_BUFFER_TOO_SMALL);
        assert_eq!(required_bytes, 5);
        assert_eq!(short_buffer, [0xA5; 2]);

        let mut asset = [0; 5];
        let read_status = unsafe {
            read_asset(
                host.package_context,
                path.as_ptr(),
                path.len() as u32,
                asset.as_mut_ptr(),
                asset.len() as u32,
                &mut required_bytes,
            )
        };
        assert_eq!(read_status, NATIVE_STATUS_OK);
        assert_eq!(&asset, b"atlas");

        let traversal = b"../outside.bin";
        let traversal_status = unsafe {
            read_asset(
                host.package_context,
                traversal.as_ptr(),
                traversal.len() as u32,
                asset.as_mut_ptr(),
                asset.len() as u32,
                &mut required_bytes,
            )
        };
        assert_eq!(traversal_status, NATIVE_STATUS_INVALID_ARGUMENT);

        let missing = b"assets/missing.bin";
        let missing_status = unsafe {
            read_asset(
                host.package_context,
                missing.as_ptr(),
                missing.len() as u32,
                asset.as_mut_ptr(),
                asset.len() as u32,
                &mut required_bytes,
            )
        };
        assert_eq!(missing_status, NATIVE_STATUS_NOT_FOUND);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn version_one_abi_layout_sizes_are_stable_on_64_bit_hosts() {
        assert_eq!(size_of::<SFNativePluginV1>(), 48);
        assert_eq!(size_of::<SFNativeHostV1>(), 96);
    }

    #[test]
    fn service_lifetime_hooks_bind_revoke_and_drain_to_owner_generation() {
        let package = TestPackage::new();
        let services = Arc::new(TestServiceLifetime {
            revoke_status: 0,
            drain_status: NATIVE_STATUS_BUSY,
            queried_owner: Mutex::new(None),
            revoked_owner: Mutex::new(None),
            drained_owner: Mutex::new(None),
        });
        let mut context = Box::new(
            NativePluginHostContext::new(NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_generation: 41,
                is_server: false,
                settings_json: b"{}".to_vec(),
                services: services.clone(),
            })
            .unwrap(),
        );
        let host = context.host_table();

        let map_service = NATIVE_SERVICE_MAP_V1.as_bytes();
        let mut service_table = 0xDEAD_BEEFu32;
        let short_query = unsafe {
            host.query_service.unwrap()(
                host.package_context,
                map_service.as_ptr(),
                map_service.len() as u32,
                1,
                (size_of::<u32>() - 1) as u32,
                (&mut service_table as *mut u32).cast(),
            )
        };
        assert_eq!(short_query, NATIVE_STATUS_BUFFER_TOO_SMALL);
        assert_eq!(service_table, 0xDEAD_BEEF);

        let full_query = unsafe {
            host.query_service.unwrap()(
                host.package_context,
                map_service.as_ptr(),
                map_service.len() as u32,
                1,
                size_of::<u32>() as u32,
                (&mut service_table as *mut u32).cast(),
            )
        };
        assert_eq!(full_query, NATIVE_STATUS_OK);
        assert_eq!(service_table, 0x5346_4D50);
        assert_eq!(
            *services.queried_owner.lock().unwrap(),
            Some((
                "example".to_owned(),
                41,
                NATIVE_SERVICE_MAP_V1.to_owned(),
                1,
                size_of::<u32>() as u32,
            ))
        );
        let revoke_status =
            unsafe { host.revoke_services.unwrap()(host.package_context, host.package_generation) };
        let drain_status = unsafe {
            host.drain_services.unwrap()(
                host.package_context,
                host.package_generation,
                NATIVE_DRAIN_TIMEOUT_MS,
            )
        };

        assert_eq!(revoke_status, NATIVE_STATUS_OK);
        assert_eq!(drain_status, NATIVE_STATUS_BUSY);
        assert_eq!(
            *services.revoked_owner.lock().unwrap(),
            Some(("example".to_owned(), 41))
        );
        assert_eq!(
            *services.drained_owner.lock().unwrap(),
            Some(("example".to_owned(), 41, NATIVE_DRAIN_TIMEOUT_MS))
        );

        service_table = 0xA5A5_A5A5;
        let terminal_query = unsafe {
            host.query_service.unwrap()(host.package_context,map_service.as_ptr(),map_service.len() as u32,1,size_of::<u32>() as u32,(&mut service_table as *mut u32).cast())
        };
        assert_eq!(terminal_query,NATIVE_STATUS_UNAVAILABLE);
        assert_eq!(service_table,0xA5A5_A5A5);
    }
    #[test]
    fn map_service_authority_is_checked_before_forwarding_for_each_generation() {
        for (runtime,is_server) in [(false,false),(true,true),(true,false)] {
            let package=TestPackage::with_runtime(runtime);
            let services=Arc::new(TestServiceLifetime{revoke_status:0,drain_status:0,queried_owner:Mutex::new(None),revoked_owner:Mutex::new(None),drained_owner:Mutex::new(None)});
            let mut context=Box::new(NativePluginHostContext::new(NativePluginHostMetadata {
                package_id:"example".into(),package:package.package.clone(),package_generation:77,is_server,settings_json:b"{}".to_vec(),services:services.clone()
            }).unwrap());
            let host=context.host_table();let name=NATIVE_SERVICE_MAP_V1.as_bytes();let mut output=0u32;
            let status=unsafe{host.query_service.unwrap()(host.package_context,name.as_ptr(),name.len() as u32,1,4,(&mut output as *mut u32).cast())};
            assert_eq!(status,if runtime && !is_server {0}else{NATIVE_STATUS_UNAVAILABLE});
            assert_eq!(services.queried_owner.lock().unwrap().is_some(),runtime && !is_server);
            if !runtime || is_server {assert_eq!(output,0);}
        }
    }

    #[test]
    fn first_service_query_from_stop_or_shutdown_is_terminal_after_revoke() {
        unsafe extern "C" fn late_query(context: *mut c_void, host: *const SFNativeHostV1) -> i32 {
            let host = unsafe { &*host };
            let name = NATIVE_SERVICE_OVERLAY_V1.as_bytes();
            let mut output = 0u32;
            let status = unsafe {
                host.query_service.unwrap()(
                    host.package_context,
                    name.as_ptr(),
                    name.len() as u32,
                    1,
                    4,
                    (&mut output as *mut u32).cast(),
                )
            };
            unsafe { (&mut *context.cast::<Vec<i32>>()).push(status) };
            0
        }
        let package = TestPackage::new();
        let services = Arc::new(TestServiceLifetime {
            revoke_status: 0,
            drain_status: 0,
            queried_owner: Mutex::new(None),
            revoked_owner: Mutex::new(None),
            drained_owner: Mutex::new(None),
        });
        let metadata = |generation| NativePluginHostMetadata {
            package_id: "example".into(),
            package: package.package.clone(),
            package_generation: generation,
            is_server: false,
            settings_json: b"{}".to_vec(),
            services: services.clone(),
        };
        let mut late_statuses = Box::new(Vec::<i32>::new());
        let mut instance = NativePluginInstance::load(bounded_entry, metadata(42)).unwrap();
        instance.plugin.plugin_context = (&mut *late_statuses as *mut Vec<i32>).cast();
        instance.plugin.stop = Some(late_query);
        instance.plugin.shutdown = Some(late_query);
        instance.start().unwrap();
        assert!(services.queried_owner.lock().unwrap().is_none());
        assert_eq!(instance.stop(), TeardownOutcome::Quiescent);
        assert_eq!(
            *late_statuses,
            vec![NATIVE_STATUS_UNAVAILABLE, NATIVE_STATUS_UNAVAILABLE]
        );
        assert!(services.queried_owner.lock().unwrap().is_none());
        let mut next = Box::new(NativePluginHostContext::new(metadata(43)).unwrap());
        let table = next.host_table();
        let name = NATIVE_SERVICE_MAP_V1.as_bytes();
        let mut output = 0u32;
        assert_eq!(
            unsafe {
                table.query_service.unwrap()(
                    table.package_context,
                    name.as_ptr(),
                    name.len() as u32,
                    1,
                    4,
                    (&mut output as *mut u32).cast(),
                )
            },
            0
        );
        assert_eq!(
            services.queried_owner.lock().unwrap().as_ref().unwrap().1,
            43
        );
    }

    #[test]
    fn failed_native_revoke_closes_sdk_gate_and_retains_generation() {
        let package = TestPackage::new();
        let services = Arc::new(TestServiceLifetime {
            revoke_status: NATIVE_STATUS_BUSY,
            drain_status: 0,
            queried_owner: Mutex::new(None),
            revoked_owner: Mutex::new(None),
            drained_owner: Mutex::new(None),
        });
        let mut instance = NativePluginInstance::load(
            bounded_entry,
            NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_generation: 50,
                is_server: false,
                settings_json: b"{}".to_vec(),
                services: services.clone(),
            },
        )
        .unwrap();
        instance.start().unwrap();
        assert_eq!(
            instance.stop(),
            TeardownOutcome::Retain {
                stage: "service revocation",
                status: NATIVE_STATUS_BUSY
            }
        );
        assert_eq!(instance.state, NativePluginInstanceState::Retained);
        let host = instance.host.as_ref();
        let name = NATIVE_SERVICE_MAP_V1.as_bytes();
        let mut output = 0u32;
        assert_eq!(
            unsafe {
                host.query_service.unwrap()(
                    host.package_context,
                    name.as_ptr(),
                    name.len() as u32,
                    1,
                    4,
                    (&mut output as *mut u32).cast(),
                )
            },
            NATIVE_STATUS_UNAVAILABLE
        );
        assert!(services.queried_owner.lock().unwrap().is_none());
    }

}
