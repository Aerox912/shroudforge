mod keybinds;
mod network;
mod owner_adoption;
pub(super) mod process_lifetime;
mod settings;
use super::Mod;
use super::native_plugin_lifecycle::{
    InitializationFailure, LifecycleCallbacks, TeardownOutcome, initialize_or_rollback, teardown,
};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    io::Read,
    mem::{ManuallyDrop, size_of},
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
pub(crate) const NATIVE_SERVICE_NOTICES_V1: &str = "shroudforge.notices";

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
pub(crate) const NATIVE_STATUS_CONFLICT: i32 = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeNetworkPolicy {
    Enabled,
    Disabled,
    Unavailable,
}

impl NativeNetworkPolicy {
    pub(crate) fn from_loader_config(config: Option<&serde_json::Value>) -> Self {
        match config
            .and_then(|value| value.pointer("/modules/network/enabled"))
            .and_then(serde_json::Value::as_bool)
        {
            Some(false) => Self::Disabled,
            Some(true) => Self::Enabled,
            None if config.is_some() => Self::Enabled,
            None => Self::Unavailable,
        }
    }

    pub(crate) fn read_loader(root: &std::path::Path) -> Self {
        match shroudforge_loader_config::config::read_loader(root) {
            Ok(config) => Self::from_loader_config(Some(&config)),
            Err(_) => Self::Unavailable,
        }
    }
}

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
    pub(crate) package_binding: mod_loader::package_lookup::ModPackageBinding,
    pub(crate) package_generation: u64,
    pub(crate) is_server: bool,
    pub(crate) installation_root: std::path::PathBuf,
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
    installation_root: std::path::PathBuf,
    package_id_utf8: Box<[u8]>,
    package: Mod,
    package_binding: mod_loader::package_lookup::ModPackageBinding,
    package_generation: u64,
    process_target: u32,
    owner_thread: ThreadId,
    services_revoked: Cell<bool>,
    initializing: Cell<bool>,
    process_admission: Option<Arc<process_lifetime::Admission>>,
    settings_json: RefCell<Vec<u8>>,
    keybinds: RefCell<keybinds::Keybinds>,
    network: network::OwnerState,
    network_policy: Cell<NativeNetworkPolicy>,
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
        let admitted_package_root = metadata.package.fs().root().as_std_path().to_path_buf();
        if !metadata
            .package_binding
            .matches_admitted_package(&metadata.package.info().id, &admitted_package_root)
        {
            return Err("native plugin package binding does not match its admitted package".into());
        }
        mod_loader::package_lookup::validate_mod_package_binding(
            &metadata.installation_root,
            &metadata.package_binding,
        )?;
        let network_policy = NativeNetworkPolicy::read_loader(&metadata.installation_root);
        if metadata.settings_json.len() > NATIVE_MAX_SETTINGS_JSON_BYTES {
            return Err("native plugin settings JSON exceeds 65536 bytes".into());
        }

        let mut keybinds = keybinds::Keybinds::default();
        keybinds.update(&metadata.package.info().settings, &metadata.settings_json)?;
        let process_admission = process_lifetime::owner(
            std::str::from_utf8(&package_id_utf8).unwrap_or(""),
            metadata.package_generation,
        );
        if let Some(admission) = &process_admission {
            admission.validate(&metadata.package.info().id, metadata.package_generation)?;
        }
        Ok(Self {
            installation_root: metadata.installation_root,
            keybinds: RefCell::new(keybinds),
            package_id_utf8,
            package: metadata.package,
            package_binding: metadata.package_binding,
            package_generation: metadata.package_generation,
            process_target: if metadata.is_server {
                NATIVE_TARGET_SERVER
            } else {
                NATIVE_TARGET_CLIENT
            },
            owner_thread: std::thread::current().id(),
            services_revoked: Cell::new(false),
            initializing: Cell::new(false),
            process_admission,
            settings_json: RefCell::new(metadata.settings_json),
            network: network::OwnerState::default(),
            network_policy: Cell::new(network_policy),
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

    fn settings_service_available(&self) -> bool {
        self.process_target == NATIVE_TARGET_CLIENT
            && self.package.info().target.supports_process(false)
            && self
                .package
                .info()
                .capabilities
                .contains(&mod_loader::Capability::Runtime)
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
        if *current != settings_json {
            self.keybinds
                .try_borrow_mut()
                .map_err(|_| "native keybinds are in use")?
                .update(&self.package.info().settings, &settings_json)?;
            *current = settings_json;
        }
        Ok(())
    }

    fn update_network_policy(&self, policy: NativeNetworkPolicy) -> Result<bool, String> {
        if !self.is_owner_thread() {
            return Err("native network policy must be updated on the runtime owner thread".into());
        }
        let previous = self.network_policy.get();
        if previous == policy {
            return Ok(false);
        }
        // Deny network work while invalidating any old reply route. A failed
        // transition cannot leave a stale enabled policy visible, and advances
        // the active-call epoch even when policy is quickly reenabled.
        self.network_policy.set(NativeNetworkPolicy::Unavailable);
        self.network
            .transition_policy()
            .map_err(native_status_description)?;
        self.network_policy.set(policy);
        Ok(true)
    }
}

pub(crate) struct NativePluginInstance {
    plugin: SFNativePluginV1,
    host_context: ManuallyDrop<Box<NativePluginHostContext>>,
    host: ManuallyDrop<Box<SFNativeHostV1>>,
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
        Self::load_inner(entry, metadata, None)
    }
    pub(crate) fn load_preloaded(
        entry: NativePluginEntry,
        metadata: NativePluginHostMetadata,
        owner: Arc<process_lifetime::Admission>,
        module: usize,
    ) -> Result<Self, String> {
        let current = process_lifetime::owner(&metadata.package_id, metadata.package_generation)
            .ok_or("preload root is no longer current")?;
        if !Arc::ptr_eq(&current, &owner) {
            return Err("preload adoption root mismatch".into());
        }
        owner.begin_preload_adoption(
            &metadata.package_id,
            metadata.package_generation,
            if metadata.is_server {
                NATIVE_TARGET_SERVER
            } else {
                NATIVE_TARGET_CLIENT
            },
            module,
            entry as *const () as usize,
        )?;
        let result = Self::load_inner(entry, metadata, Some(&owner));
        if result.is_err() {
            owner.fail_preload();
        }
        result
    }
    fn load_inner(
        entry: NativePluginEntry,
        metadata: NativePluginHostMetadata,
        preload: Option<&Arc<process_lifetime::Admission>>,
    ) -> Result<Self, String> {
        if preload.is_none()
            && process_lifetime::owner(&metadata.package_id, metadata.package_generation)
                .is_some_and(|o| o.is_preload())
        {
            return Err("preload owner requires explicit one-time adoption".into());
        }
        let mut host_context = Box::new(NativePluginHostContext::new(metadata)?);
        if let Some(admission) = &host_context.process_admission {
            admission.bind_host((&*host_context as *const NativePluginHostContext) as usize)?;
        }
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
            host_context: ManuallyDrop::new(host_context),
            host: ManuallyDrop::new(host),
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

    pub(crate) fn update_network_policy(
        &self,
        config: Option<&serde_json::Value>,
    ) -> Result<bool, String> {
        if self.state != NativePluginInstanceState::Active {
            return Err("native plugin is not active".into());
        }
        self.host_context
            .update_network_policy(NativeNetworkPolicy::from_loader_config(config))
    }
}

impl LifecycleCallbacks for NativePluginInstance {
    fn initialize(&mut self) -> i32 {
        if let Some(admission) = &self.host_context.process_admission {
            let package = std::str::from_utf8(&self.host_context.package_id_utf8).unwrap_or("");
            if admission
                .validate(package, self.host.package_generation)
                .is_err()
            {
                return NATIVE_STATUS_CONFLICT;
            }
        }
        self.host_context.initializing.set(true);
        if let Some(owner) = &self.host_context.process_admission {
            owner.mark_host_exposed();
        }
        let status = unsafe {
            self.plugin
                .initialize
                .expect("validated plugin initialize callback")(
                self.plugin.plugin_context,
                self.host.as_ref(),
            )
        };
        self.host_context.initializing.set(false);
        status
    }

    fn requires_process_retention(&self) -> bool {
        self.host_context
            .process_admission
            .as_ref()
            .is_some_and(|owner| owner.is_retained())
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

impl Drop for NativePluginInstance {
    fn drop(&mut self) {
        if let Some(owner) = &self.host_context.process_admission {
            if owner.retain_host_boxes_on_drop() {
                if !self.host_context.services_revoked.get() {
                    let context = (self.host_context.as_ref() as *const NativePluginHostContext)
                        .cast_mut()
                        .cast();
                    let _ = unsafe { host_revoke_services(context, self.host.package_generation) };
                }
                return;
            }
            if owner.is_preload() {
                owner.fail_preload();
            }
        }
        // Only unexposed preload boxes or an ordinary unretained instance may
        // be disposed. Exposed retained host/context allocations stay alive.
        unsafe {
            ManuallyDrop::drop(&mut self.host);
            ManuallyDrop::drop(&mut self.host_context);
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
        NATIVE_STATUS_TOO_LARGE => "service data exceeds the read limit",
        NATIVE_STATUS_CONFLICT => "revision conflict",
        process_lifetime::RESTART_REQUIRED => "process retained; restart required",
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
    if !matches!(
        name,
        NATIVE_SERVICE_MAP_V1
            | NATIVE_SERVICE_OVERLAY_V1
            | NATIVE_SERVICE_NOTICES_V1
            | keybinds::SERVICE
            | network::SERVICE
            | settings::SERVICE
            | process_lifetime::SERVICE
            | owner_adoption::SERVICE
    ) {
        return NATIVE_STATUS_UNKNOWN_SERVICE;
    }
    let supported_version = match name {
        keybinds::SERVICE | network::SERVICE => matches!(requested_version, 1 | 2),
        settings::SERVICE
        | process_lifetime::SERVICE
        | owner_adoption::SERVICE
        | NATIVE_SERVICE_MAP_V1
        | NATIVE_SERVICE_OVERLAY_V1
        | NATIVE_SERVICE_NOTICES_V1 => requested_version == 1,
        _ => false,
    };
    if !supported_version {
        return NATIVE_STATUS_ABI_MISMATCH;
    }
    if name == process_lifetime::SERVICE {
        if !context
            .package
            .info()
            .capabilities
            .contains(&mod_loader::Capability::RuntimeRegisterDll)
            || !context
                .package
                .info()
                .target
                .supports_process(context.process_target == NATIVE_TARGET_SERVER)
        {
            return NATIVE_STATUS_UNAVAILABLE;
        }
        return unsafe { process_lifetime::query(package_context, out_capacity, out_table) };
    }
    if name == owner_adoption::SERVICE {
        return unsafe { owner_adoption::query(package_context, out_capacity, out_table) };
    }
    if name == network::SERVICE {
        if requested_version == 2 {
            return unsafe { network::v2::query(package_context, out_capacity, out_table) };
        }
        return unsafe { network::query(package_context, out_capacity, out_table) };
    }
    // The lifecycle owner has already revalidated enabled package identity
    // before initialization. Check this generation's immutable permissions here,
    // rather than a startup allowlist that would reject later enabled packages.
    if !context.settings_service_available() {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    if name == keybinds::SERVICE {
        return if requested_version == 1 {
            unsafe { keybinds::query_v1(package_context, out_capacity, out_table) }
        } else {
            unsafe { keybinds::query_v2(package_context, out_capacity, out_table) }
        };
    }
    if name == settings::SERVICE {
        return if requested_version == 1 {
            unsafe { settings::query(package_context, out_capacity, out_table) }
        } else {
            NATIVE_STATUS_ABI_MISMATCH
        };
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
    context.network.revoke();
    let adoption_status = owner_adoption::host_revoke(context);
    let package_id = std::str::from_utf8(&context.package_id_utf8).unwrap_or("");
    let status = context.services.revoke(package_id, package_generation);
    if status != NATIVE_STATUS_OK {
        status
    } else {
        adoption_status
    }
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
    if context.network.has_active_call() {
        return NATIVE_STATUS_BUSY;
    }
    if context
        .process_admission
        .as_ref()
        .is_some_and(|o| o.owner_adoption.is_active())
    {
        return NATIVE_STATUS_BUSY;
    }
    let package_id = std::str::from_utf8(&context.package_id_utf8).unwrap_or("");
    context
        .services
        .drain(package_id, package_generation, timeout_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_adoption_known_service_refuses_nonpreload_without_output() {
        let package = TestPackage::new();
        let mut instance = NativePluginInstance::load(
            bounded_entry,
            NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_binding: package.package_binding(),
                package_generation: 910001,
                is_server: false,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            },
        )
        .unwrap();
        let host = instance.host.as_mut();
        let name = b"shroudforge.owner-adoption";
        let mut output = [0x5555555555555555u64; 8];
        assert_eq!(
            unsafe {
                host.query_service.unwrap()(
                    host.package_context,
                    name.as_ptr(),
                    name.len() as u32,
                    1,
                    64,
                    output.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(output, [0x5555555555555555u64; 8]);
    }
    #[cfg(windows)]
    mod preload_tests {
        use super::*;
        thread_local! {static REENTER:RefCell<Option<Box<dyn FnMut()->bool>>> = const{RefCell::new(None)};}
        unsafe extern "C" fn reenter() -> i32 {
            REENTER.with(|slot| {
                slot.try_borrow_mut()
                    .ok()
                    .and_then(|mut value| value.as_mut().map(|f| i32::from(f())))
                    .unwrap_or(0)
            })
        }
        use crate::session::native_plugins::{
            NativeLibrary, load_admitted_native_library, preload_artifact::PreloadArtifact,
            spawn_admitted_native_load,
        };
        use std::{io::Write, sync::Barrier, time::Duration};
        unsafe extern "C" fn check_root(cookie: usize) -> i32 {
            if cookie == 0 {
                return 0;
            }
            let root = unsafe { &*(cookie as *const process_lifetime::Admission) };
            let (_, host, mapping, exposed) = root.preload_test_snapshot();
            i32::from(
                root.root_cookie().1 == cookie
                    && root.preload_host_pinned()
                    && host == 0
                    && mapping == 0
                    && !exposed,
            )
        }
        struct Environment;
        impl Drop for Environment {
            fn drop(&mut self) {
                unsafe {
                    for key in [
                        "SF_PRELOAD_TEST_CHECK",
                        "SF_PRELOAD_TEST_ENTRY_FAIL",
                        "SF_PRELOAD_TEST_INITIALIZE_FAIL",
                        "SF_PRELOAD_TEST_REENTER",
                    ] {
                        std::env::remove_var(key);
                    }
                }
                REENTER.with(|slot| *slot.borrow_mut() = None);
            }
        }
        struct Fixture {
            package: TestPackage,
            root: Arc<process_lifetime::Admission>,
            path: std::path::PathBuf,
            generation: u64,
            _environment: Environment,
        }
        impl Fixture {
            fn new(label: &str) -> Self {
                let generation = process_lifetime::next_generation().unwrap();
                let id = format!("preload-{label}-{generation}");
                let package = TestPackage::named(false, "client", &id);
                let path = package
                    .package
                    .fs()
                    .root()
                    .as_std_path()
                    .join(format!("fixture-{generation}.dll"));
                fs::copy(
                    std::env::var_os("SF_PRELOAD_FIXTURE_DLL")
                        .expect("build owned preload_fixture.c and set SF_PRELOAD_FIXTURE_DLL"),
                    &path,
                )
                .unwrap();
                // A PE overlay distinguishes cases without changing loaded code.
                // Cross-package alias tests deliberately copy the exact same bytes.
                writeln!(
                    fs::OpenOptions::new().append(true).open(&path).unwrap(),
                    "SF-GM-PROVIDER-09:{generation}:{label}"
                )
                .unwrap();
                let artifact = PreloadArtifact::open(&path).unwrap();
                let relative = path.file_name().unwrap().to_str().unwrap().to_owned();
                let root = process_lifetime::reserve_preload(
                    &id,
                    generation,
                    process_lifetime::PreloadIntent {
                        artifact,
                        package_binding: package.package_binding(),
                        relative_path: relative,
                        process_target: NATIVE_TARGET_CLIENT,
                    },
                )
                .unwrap();
                unsafe {
                    std::env::set_var(
                        "SF_PRELOAD_TEST_CHECK",
                        format!(
                            "{:x}:{:x}",
                            check_root as *const () as usize,
                            root.root_cookie().1
                        ),
                    );
                }
                Self {
                    package,
                    root,
                    path,
                    generation,
                    _environment: Environment,
                }
            }
            fn metadata(&self) -> NativePluginHostMetadata {
                NativePluginHostMetadata {
                    package_id: self.package.package.info().id.clone(),
                    package: self.package.package.clone(),
                    package_binding: self.package.package_binding(),
                    package_generation: self.generation,
                    is_server: false,
                    installation_root: self.package.root.clone(),
                    settings_json: b"{}".to_vec(),
                    services: Arc::new(NoNativePluginServices),
                }
            }
            fn load(&self) -> NativeLibrary {
                let receiver = spawn_admitted_native_load(
                    self.package.package.info().id.clone(),
                    self.path.clone(),
                    None,
                    Some(self.root.clone()),
                    || {},
                )
                .unwrap();
                receiver
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .unwrap()
            }
            fn instance(&self, library: &NativeLibrary) -> NativePluginInstance {
                NativePluginInstance::load_preloaded(
                    library.entrypoint().unwrap().unwrap(),
                    self.metadata(),
                    self.root.clone(),
                    library.handle as usize,
                )
                .unwrap()
            }
        }
        fn count(library: &NativeLibrary, name: &[u8]) -> i32 {
            let address = unsafe {
                windows_sys::Win32::System::LibraryLoader::GetProcAddress(
                    library.handle as _,
                    name.as_ptr(),
                )
            }
            .unwrap();
            let read: unsafe extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            unsafe { read() }
        }
        #[test]
        fn preload_actual_worker_pins_root_before_dllmain_and_adopts_once() {
            let f = Fixture::new("success");
            let weak = Arc::downgrade(&f.root);
            let cookie = f.root.root_cookie();
            assert!(f.root.preload_host_pinned());
            assert_eq!(f.root.preload_test_snapshot().1, 0);
            let library = f.load();
            let entry = library.entrypoint().unwrap().unwrap();
            let mut wrong = f.metadata();
            wrong.package_generation += 1;
            assert!(
                NativePluginInstance::load_preloaded(
                    entry,
                    wrong,
                    f.root.clone(),
                    library.handle as usize
                )
                .is_err()
            );
            let module = library.handle as usize;
            let root = f.root.clone();
            let package = f.package.package.info().id.clone();
            let generation = f.generation;
            assert!(
                std::thread::spawn(move || root.begin_preload_adoption(
                    &package,
                    generation,
                    NATIVE_TARGET_CLIENT,
                    module,
                    entry as *const () as usize
                ))
                .join()
                .unwrap()
                .is_err()
            );
            for (target, module) in [
                (NATIVE_TARGET_SERVER, library.handle as usize),
                (NATIVE_TARGET_CLIENT, library.handle as usize + 1),
            ] {
                assert!(
                    f.root
                        .begin_preload_adoption(
                            &f.package.package.info().id,
                            f.generation,
                            target,
                            module,
                            entry as *const () as usize
                        )
                        .is_err()
                );
            }
            let mut instance = f.instance(&library);
            assert_eq!(count(&library, b"PreloadFixtureEntryCount\0"), 1);
            let address =
                (instance.host_context.as_ref() as *const NativePluginHostContext) as usize;
            assert_ne!(address, cookie.1);
            assert!(f.root.is_bound_host(address));
            assert!(
                NativePluginInstance::load_preloaded(
                    entry,
                    f.metadata(),
                    f.root.clone(),
                    library.handle as usize
                )
                .is_err()
            );
            instance.start().unwrap();
            assert!(f.root.preload_test_snapshot().3);
            assert!(matches!(instance.stop(), TeardownOutcome::Retain { .. }));
            assert_eq!(count(&library, b"PreloadFixtureShutdownCount\0"), 0);
            drop(instance);
            drop(library);
            assert_eq!(weak.upgrade().unwrap().root_cookie(), cookie);
            assert!(
                process_lifetime::reserve(&f.package.package.info().id, f.generation + 1).is_err()
            );
        }
        #[test]
        fn preload_retained_host_context_survives_unexpected_instance_drop() {
            let f = Fixture::new("unexpected-drop");
            let library = f.load();
            let mut instance = f.instance(&library);
            instance.start().unwrap();
            let host = instance.host.as_ref() as *const SFNativeHostV1;
            let context = instance.host.package_context;
            drop(instance);
            drop(library);
            assert!(f.root.is_bound_host(context as usize));
            let host = unsafe { &*host };
            let mut output = [0u8; 2];
            let mut required = 0;
            assert_eq!(
                unsafe {
                    host.read_settings_json.unwrap()(context, output.as_mut_ptr(), 2, &mut required)
                },
                NATIVE_STATUS_OK
            );
            assert_eq!(output, *b"{}");
            assert_eq!(required, 2);
            let name = process_lifetime::SERVICE.as_bytes();
            let mut table = [0u64; 4];
            assert_eq!(
                unsafe {
                    host.query_service.unwrap()(
                        context,
                        name.as_ptr(),
                        name.len() as u32,
                        1,
                        32,
                        table.as_mut_ptr().cast(),
                    )
                },
                NATIVE_STATUS_UNAVAILABLE
            );
        }
        #[test]
        fn preload_entry_failure_disposes_only_unexposed_context_and_stays_sticky() {
            let f = Fixture::new("entry-failure");
            let library = f.load();
            unsafe {
                std::env::set_var("SF_PRELOAD_TEST_ENTRY_FAIL", "1");
            }
            assert!(
                NativePluginInstance::load_preloaded(
                    library.entrypoint().unwrap().unwrap(),
                    f.metadata(),
                    f.root.clone(),
                    library.handle as usize
                )
                .is_err()
            );
            let (_, host, mapping, exposed) = f.root.preload_test_snapshot();
            assert_eq!(host, 0);
            assert_ne!(mapping, 0);
            assert!(!exposed);
            drop(library);
            assert!(process_lifetime::preload_is_sticky(
                &f.package.package.info().id
            ));
            assert!(
                process_lifetime::reserve(&f.package.package.info().id, f.generation + 1).is_err()
            );
        }
        #[test]
        fn preload_initialize_failure_retains_exposed_context() {
            let f = Fixture::new("initialize-failure");
            let library = f.load();
            let mut instance = f.instance(&library);
            unsafe {
                std::env::set_var("SF_PRELOAD_TEST_INITIALIZE_FAIL", "1");
            }
            let host = instance.host.as_ref() as *const SFNativeHostV1;
            let context = instance.host.package_context;
            assert!(matches!(
                instance.start().unwrap_err().cleanup,
                TeardownOutcome::Retain { .. }
            ));
            drop(instance);
            drop(library);
            assert!(f.root.is_bound_host(context as usize));
            assert!(f.root.preload_test_snapshot().3);
            let host = unsafe { &*host };
            let mut required = 0;
            assert_eq!(
                unsafe {
                    host.read_settings_json.unwrap()(
                        context,
                        std::ptr::null_mut(),
                        0,
                        &mut required,
                    )
                },
                NATIVE_STATUS_BUFFER_TOO_SMALL
            );
            assert_eq!(required, 2);
        }
        #[test]
        fn preload_cancel_before_worker_prevents_target_mapping_and_retry() {
            let f = Fixture::new("cancel-before");
            drop(crate::session::native_plugins::PendingPreload::new(
                f.root.clone(),
            ));
            assert!(
                load_admitted_native_library(
                    &f.package.package.info().id,
                    &f.path,
                    None,
                    Some(f.root.clone())
                )
                .is_err()
            );
            assert_eq!(f.root.preload_test_snapshot().2, 0);
            assert!(f.root.preload_host_pinned());
            assert!(
                process_lifetime::reserve(&f.package.package.info().id, f.generation + 1).is_err()
            );
            let weak = Arc::downgrade(&f.root);
            let cookie = f.root.root_cookie();
            drop(f);
            assert_eq!(weak.upgrade().unwrap().root_cookie(), cookie);
        }
        #[test]
        fn preload_receiver_drop_after_real_mapping_keeps_module_and_suppresses_entry() {
            let f = Fixture::new("cancel-after");
            let mapped = Arc::new(Barrier::new(2));
            let resume = Arc::new(Barrier::new(2));
            let a = mapped.clone();
            let b = resume.clone();
            let receiver = spawn_admitted_native_load(
                f.package.package.info().id.clone(),
                f.path.clone(),
                None,
                Some(f.root.clone()),
                move || {
                    a.wait();
                    b.wait();
                },
            )
            .unwrap();
            mapped.wait();
            let module = f.root.preload_test_snapshot().2;
            assert_ne!(module, 0);
            drop(crate::session::native_plugins::PendingPreload::new(
                f.root.clone(),
            ));
            drop(receiver.receiver);
            resume.wait();
            receiver.worker.join().unwrap();
            // The real send has failed and NativeLibrary has dropped. The
            // mapping must remain alive through the committed root.
            let entry = unsafe {
                windows_sys::Win32::System::LibraryLoader::GetProcAddress(
                    module as _,
                    b"ShroudForgePluginEntry\0".as_ptr(),
                )
            };
            assert!(entry.is_some());
            assert!(
                f.root
                    .begin_preload_adoption(
                        &f.package.package.info().id,
                        f.generation,
                        NATIVE_TARGET_CLIENT,
                        module,
                        entry.unwrap() as *const () as usize
                    )
                    .is_err()
            );
            assert_eq!(f.root.preload_test_snapshot().1, 0);
            assert!(f.root.preload_host_pinned());
        }
        #[test]
        fn preload_wrong_selected_mapping_is_quarantined_without_entry() {
            let f = Fixture::new("wrong-mapping");
            let other = f.path.with_file_name("other.dll");
            fs::copy(&f.path, &other).unwrap();
            let receiver = spawn_admitted_native_load(
                f.package.package.info().id.clone(),
                other,
                None,
                Some(f.root.clone()),
                || {},
            )
            .unwrap();
            assert!(
                receiver
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .is_err()
            );
            let (_, host, mapping, exposed) = f.root.preload_test_snapshot();
            assert_ne!(mapping, 0);
            assert_eq!(host, 0);
            assert!(!exposed);
        }
        #[test]
        fn preload_same_artifact_second_package_and_generation_are_refused() {
            let f = Fixture::new("artifact-alias");
            let id = format!("second-owner-{}", f.generation);
            let other = TestPackage::named(false, "client", &id);
            let copied = other.package.fs().root().as_std_path().join("copy.dll");
            fs::copy(&f.path, &copied).unwrap();
            let intent = process_lifetime::PreloadIntent {
                artifact: PreloadArtifact::open(&copied).unwrap(),
                package_binding: other.package_binding(),
                relative_path: "copy.dll".into(),
                process_target: NATIVE_TARGET_CLIENT,
            };
            assert!(
                process_lifetime::reserve_preload(
                    &id,
                    process_lifetime::next_generation().unwrap(),
                    intent
                )
                .is_err()
            );
            assert!(
                process_lifetime::reserve(&f.package.package.info().id, f.generation + 1).is_err()
            );
        }
        #[test]
        fn preload_reentrant_adoption_cannot_enter_target_twice() {
            let f = Fixture::new("reentrant");
            let library = f.load();
            let entry = library.entrypoint().unwrap().unwrap();
            let module = library.handle as usize;
            let root = f.root.clone();
            let package = f.package.package.clone();
            let binding = f.package.package_binding();
            let directory = f.package.root.clone();
            let generation = f.generation;
            REENTER.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    NativePluginInstance::load_preloaded(
                        entry,
                        NativePluginHostMetadata {
                            package_id: package.info().id.clone(),
                            package: package.clone(),
                            package_binding: binding.clone(),
                            package_generation: generation,
                            is_server: false,
                            installation_root: directory.clone(),
                            settings_json: b"{}".to_vec(),
                            services: Arc::new(NoNativePluginServices),
                        },
                        root.clone(),
                        module,
                    )
                    .is_err()
                }))
            });
            unsafe {
                std::env::set_var(
                    "SF_PRELOAD_TEST_REENTER",
                    format!("{:x}", reenter as *const () as usize),
                );
            }
            let instance = f.instance(&library);
            assert_eq!(count(&library, b"PreloadFixtureEntryCount\0"), 1);
            drop(instance);
            assert_eq!(f.root.preload_test_snapshot().1, 0);
        }
        #[test]
        fn preload_loadlibrary_failure_after_pin_is_sticky_without_mapping() {
            let f = Fixture::new("load-failure");
            let task = spawn_admitted_native_load(
                f.package.package.info().id.clone(),
                f.path.with_file_name("missing.dll"),
                None,
                Some(f.root.clone()),
                || {},
            )
            .unwrap();
            assert!(task.recv_timeout(Duration::from_secs(10)).unwrap().is_err());
            task.worker.join().unwrap();
            let (_, host, mapping, exposed) = f.root.preload_test_snapshot();
            assert_eq!((host, mapping, exposed), (0, 0, false));
            assert!(f.root.preload_host_pinned());
            assert!(
                process_lifetime::reserve(&f.package.package.info().id, f.generation + 1).is_err()
            );
        }
    }
    use std::{
        fs,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    pub(super) struct TestPackage {
        pub(super) package: Mod,
        pub(super) root: PathBuf,
    }

    impl TestPackage {
        pub(super) fn package_binding(&self) -> mod_loader::package_lookup::ModPackageBinding {
            self.package.package_binding().clone()
        }

        fn new() -> Self {
            Self::with_runtime(true)
        }
        fn with_runtime(runtime: bool) -> Self {
            Self::with_target(runtime, "client")
        }
        fn with_target(runtime: bool, target: &str) -> Self {
            Self::named(runtime, target, "example")
        }
        pub(super) fn named(runtime: bool, target: &str, id: &str) -> Self {
            let target_dir = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let root = target_dir.join(format!("native-plugin-test-{}", uuid::Uuid::new_v4()));
            let package_root = root.join("mods").join("example");
            fs::create_dir_all(package_root.join("assets")).unwrap();
            fs::write(
                package_root.join("mod.json"),
                serde_json::json!({"id":id,"name":"Example","version":"1.0.0",
                    "capabilities":if runtime {vec!["runtime-register-dll","runtime"]}else{vec!["runtime-register-dll"]}}).to_string(),
            )
            .unwrap();
            fs::write(
                package_root.join("extended.mod.json"),
                serde_json::json!({
                    "schemaVersion": 1,
                    "enabled": true,
                    "targets": [target],
                    "settings": {
                        "toggle": {"value":"SDL:67","control":"keybind","applyAt":"live"},
                        "mode": {"value":"vein"},
                        "activation_key": {"value":"SDL:70"},
                        "toggle_migration_revision": {"value":1},
                        "radius_key": {"value":"SDL:82"},
                        "radius_multiplier": {"value":1.0}
                    }
                })
                .to_string(),
            )
            .unwrap();
            fs::write(package_root.join("assets").join("atlas.bin"), b"atlas").unwrap();
            let utf8_root = camino::Utf8Path::from_path(&root).unwrap();
            let environment = mod_loader::ModEnvironment::load(utf8_root).unwrap();
            let package = environment.mod_registry().get(id).unwrap().clone();
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
    fn native_keybind_service_is_bounded_owner_scoped_and_revoked() {
        let package = TestPackage::new();
        let mut context = Box::new(
            NativePluginHostContext::new(NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_binding: package.package_binding(),
                package_generation: 8,
                is_server: false,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            })
            .unwrap(),
        );
        let host = context.host_table();
        let name = keybinds::SERVICE.as_bytes();
        let mut table = std::mem::MaybeUninit::<keybinds::KeybindService>::uninit();
        let query = host.query_service.unwrap();
        let capacity = size_of::<keybinds::KeybindService>() as u32;
        assert_eq!(
            unsafe {
                query(
                    host.package_context,
                    name.as_ptr(),
                    name.len() as u32,
                    3,
                    capacity,
                    table.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_ABI_MISMATCH
        );
        assert_eq!(
            unsafe {
                query(
                    host.package_context,
                    name.as_ptr(),
                    name.len() as u32,
                    1,
                    capacity - 1,
                    table.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_BUFFER_TOO_SMALL
        );
        assert_eq!(
            unsafe {
                query(
                    host.package_context,
                    name.as_ptr(),
                    name.len() as u32,
                    1,
                    capacity,
                    table.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_OK
        );
        let table = unsafe { table.assume_init() };
        let read = table.read.unwrap();
        let mut state = keybinds::KeybindState {
            size: 999,
            flags: 999,
        };
        let size = size_of::<keybinds::KeybindState>() as u32;
        assert_eq!(
            unsafe {
                read(
                    host.package_context,
                    b"toggle".as_ptr(),
                    6,
                    size - 1,
                    &mut state,
                )
            },
            NATIVE_STATUS_BUFFER_TOO_SMALL
        );
        assert_eq!(state.flags, 999);
        assert_eq!(
            unsafe {
                read(
                    host.package_context,
                    b"foreign".as_ptr(),
                    7,
                    size,
                    &mut state,
                )
            },
            NATIVE_STATUS_NOT_FOUND
        );
        assert_eq!(
            unsafe {
                read(
                    host.package_context,
                    b"toggle".as_ptr(),
                    6,
                    size,
                    &mut state,
                )
            },
            NATIVE_STATUS_OK
        );
        assert_eq!(state.size, size);
        context
            .update_settings_json(br#"{"toggle":""}"#.to_vec())
            .unwrap();
        assert_eq!(
            unsafe {
                read(
                    host.package_context,
                    b"toggle".as_ptr(),
                    6,
                    size,
                    &mut state,
                )
            },
            NATIVE_STATUS_OK
        );
        assert_eq!(state.flags, 0);
        let original_thread = context.owner_thread;
        context.owner_thread = std::thread::spawn(|| std::thread::current().id())
            .join()
            .unwrap();
        assert_eq!(
            unsafe {
                read(
                    host.package_context,
                    b"toggle".as_ptr(),
                    6,
                    size,
                    &mut state,
                )
            },
            NATIVE_STATUS_WRONG_THREAD
        );
        context.owner_thread = original_thread;
        context.services_revoked.set(true);
        assert_eq!(
            unsafe {
                read(
                    host.package_context,
                    b"toggle".as_ptr(),
                    6,
                    size,
                    &mut state,
                )
            },
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(size_of::<keybinds::KeybindService>(), 24);
        assert_eq!(size, 8);

        context.services_revoked.set(false);
        let mut v2_table = std::mem::MaybeUninit::<keybinds::KeybindServiceV2>::uninit();
        let v2_capacity = size_of::<keybinds::KeybindServiceV2>() as u32;
        assert_eq!(
            unsafe {
                query(
                    host.package_context,
                    name.as_ptr(),
                    name.len() as u32,
                    2,
                    v2_capacity,
                    v2_table.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_OK
        );
        let v2_table = unsafe { v2_table.assume_init() };
        let read_v2 = v2_table.read.unwrap();
        let v2_size = size_of::<keybinds::KeybindStateV2>() as u32;
        let mut v2_state = keybinds::KeybindStateV2 {
            size: 999,
            flags: 999,
            focus_status: 999,
            capture_status: 999,
            admission_status: 999,
        };
        assert_eq!(
            unsafe {
                read_v2(
                    host.package_context,
                    b"toggle".as_ptr(),
                    6,
                    v2_size - 1,
                    &mut v2_state,
                )
            },
            NATIVE_STATUS_BUFFER_TOO_SMALL
        );
        assert_eq!(v2_state.size, 999);
        assert_eq!(
            unsafe {
                read_v2(
                    host.package_context,
                    b"toggle".as_ptr(),
                    6,
                    v2_size,
                    &mut v2_state,
                )
            },
            NATIVE_STATUS_OK
        );
        assert_eq!(v2_state.size, v2_size);
        assert!(v2_state.focus_status <= keybinds::INPUT_STATUS_ACTIVE);
        assert!(v2_state.capture_status <= keybinds::INPUT_STATUS_ACTIVE);
        assert!(v2_state.admission_status <= keybinds::INPUT_ADMISSION_ACCEPTED);
        let original_thread = context.owner_thread;
        context.owner_thread = std::thread::spawn(|| std::thread::current().id())
            .join()
            .unwrap();
        assert_eq!(
            unsafe {
                read_v2(
                    host.package_context,
                    b"toggle".as_ptr(),
                    6,
                    v2_size,
                    &mut v2_state,
                )
            },
            NATIVE_STATUS_WRONG_THREAD
        );
        context.owner_thread = original_thread;
        context.services_revoked.set(true);
        assert_eq!(
            unsafe {
                read_v2(
                    host.package_context,
                    b"toggle".as_ptr(),
                    6,
                    v2_size,
                    &mut v2_state,
                )
            },
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(size_of::<keybinds::KeybindServiceV2>(), 24);
        assert_eq!(v2_size, 20);
    }

    #[test]
    fn native_network_policy_refresh_uses_owner_thread_and_fail_closed_transitions() {
        let package = TestPackage::new();
        let mut instance = NativePluginInstance::load(
            bounded_entry,
            NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_binding: package.package_binding(),
                package_generation: 91,
                is_server: false,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            },
        )
        .unwrap();
        instance.start().unwrap();
        assert_eq!(
            instance.host_context.network_policy.get(),
            NativeNetworkPolicy::Enabled,
            "read_loader defaults enable Network when no settings file exists"
        );

        let disabled = serde_json::json!({"modules":{"network":{"enabled":false}}});
        assert!(instance.update_network_policy(Some(&disabled)).unwrap());
        assert_eq!(
            instance.host_context.network_policy.get(),
            NativeNetworkPolicy::Disabled
        );
        assert!(instance.update_network_policy(None).unwrap());
        assert_eq!(
            instance.host_context.network_policy.get(),
            NativeNetworkPolicy::Unavailable
        );
        let enabled = serde_json::json!({"modules":{"network":{"enabled":true}}});
        assert!(instance.update_network_policy(Some(&enabled)).unwrap());
        assert_eq!(
            instance.host_context.network_policy.get(),
            NativeNetworkPolicy::Enabled
        );

        let owner_thread = instance.host_context.owner_thread;
        instance.host_context.owner_thread = std::thread::spawn(|| std::thread::current().id())
            .join()
            .unwrap();
        assert!(instance.update_network_policy(Some(&disabled)).is_err());
        assert_eq!(
            instance.host_context.network_policy.get(),
            NativeNetworkPolicy::Enabled
        );
        instance.host_context.owner_thread = owner_thread;
    }

    #[test]
    fn native_settings_service_reads_coherent_snapshots_and_applies_declared_partial_patches() {
        let package = TestPackage::new();
        let mut context = Box::new(
            NativePluginHostContext::new(NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_binding: package.package_binding(),
                package_generation: 18,
                is_server: false,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            })
            .unwrap(),
        );
        let host = context.host_table();
        let query = host.query_service.unwrap();
        let service_name = settings::SERVICE.as_bytes();
        let mut table = std::mem::MaybeUninit::<settings::SettingsService>::uninit();
        let table_size = size_of::<settings::SettingsService>() as u32;
        assert_eq!(
            unsafe {
                query(
                    host.package_context,
                    service_name.as_ptr(),
                    service_name.len() as u32,
                    2,
                    table_size,
                    table.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_ABI_MISMATCH
        );
        assert_eq!(
            unsafe {
                query(
                    host.package_context,
                    service_name.as_ptr(),
                    service_name.len() as u32,
                    1,
                    table_size - 1,
                    table.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_BUFFER_TOO_SMALL
        );
        assert_eq!(
            unsafe {
                query(
                    host.package_context,
                    service_name.as_ptr(),
                    service_name.len() as u32,
                    1,
                    table_size,
                    table.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_OK
        );
        let table = unsafe { table.assume_init() };
        assert_eq!(table.version, 1);
        let read_snapshot = table.read_snapshot.unwrap();
        let patch = table.patch.unwrap();
        let mut short = [0xA5; 2];
        let mut required = 0;
        assert_eq!(
            unsafe {
                read_snapshot(
                    host.package_context,
                    short.as_mut_ptr(),
                    short.len() as u32,
                    &mut required,
                )
            },
            NATIVE_STATUS_BUFFER_TOO_SMALL
        );
        assert!(required > short.len() as u32);
        assert_eq!(short, [0xA5; 2]);
        let mut snapshot_bytes = vec![0; required as usize];
        assert_eq!(
            unsafe {
                read_snapshot(
                    host.package_context,
                    snapshot_bytes.as_mut_ptr(),
                    snapshot_bytes.len() as u32,
                    &mut required,
                )
            },
            NATIVE_STATUS_OK
        );
        let snapshot: serde_json::Value = serde_json::from_slice(&snapshot_bytes).unwrap();
        let revision = snapshot["revision"].as_str().unwrap().to_owned();
        assert_eq!(revision.len(), 64);
        assert_eq!(snapshot["values"]["mode"], "vein");
        assert_eq!(snapshot["values"]["activation_key"], "SDL:70");
        assert_eq!(snapshot["values"]["toggle_migration_revision"], 1);
        assert_eq!(snapshot["values"]["radius_key"], "SDL:82");
        assert_eq!(snapshot["values"]["radius_multiplier"], 1.0);

        let update = br#"{"radius_multiplier":2.5}"#;
        assert_eq!(
            unsafe {
                patch(
                    host.package_context,
                    revision.as_ptr(),
                    revision.len() as u32,
                    update.as_ptr(),
                    update.len() as u32,
                )
            },
            NATIVE_STATUS_OK
        );
        assert_eq!(
            unsafe {
                patch(
                    host.package_context,
                    revision.as_ptr(),
                    revision.len() as u32,
                    br#"{"radius_multiplier":9.0}"#.as_ptr(),
                    br#"{"radius_multiplier":9.0}"#.len() as u32,
                )
            },
            NATIVE_STATUS_CONFLICT
        );
        assert_eq!(
            unsafe {
                read_snapshot(
                    host.package_context,
                    snapshot_bytes.as_mut_ptr(),
                    snapshot_bytes.len() as u32,
                    &mut required,
                )
            },
            NATIVE_STATUS_OK
        );
        let after: serde_json::Value =
            serde_json::from_slice(&snapshot_bytes[..required as usize]).unwrap();
        assert_eq!(after["values"]["radius_multiplier"], 2.5);
        assert_eq!(after["values"]["mode"], "vein");
        assert_eq!(after["values"]["activation_key"], "SDL:70");
        assert_eq!(after["values"]["toggle_migration_revision"], 1);
        assert_eq!(after["values"]["radius_key"], "SDL:82");
        assert_ne!(after["revision"], revision);

        let current_revision = after["revision"].as_str().unwrap();
        let oversized = vec![b' '; NATIVE_MAX_SETTINGS_JSON_BYTES + 1];
        assert_eq!(
            unsafe {
                patch(
                    host.package_context,
                    current_revision.as_ptr(),
                    current_revision.len() as u32,
                    oversized.as_ptr(),
                    oversized.len() as u32,
                )
            },
            NATIVE_STATUS_TOO_LARGE
        );
        assert_eq!(
            unsafe {
                patch(
                    host.package_context,
                    current_revision.as_ptr(),
                    current_revision.len() as u32,
                    b"[]".as_ptr(),
                    2,
                )
            },
            NATIVE_STATUS_INVALID_ARGUMENT
        );
        assert_eq!(
            unsafe {
                patch(
                    host.package_context,
                    current_revision.as_ptr(),
                    current_revision.len() as u32,
                    b"{\xFF}".as_ptr(),
                    3,
                )
            },
            NATIVE_STATUS_INVALID_ARGUMENT
        );
        assert_eq!(
            unsafe {
                patch(
                    host.package_context,
                    current_revision.as_ptr(),
                    current_revision.len() as u32,
                    br#"{"foreign":true}"#.as_ptr(),
                    br#"{"foreign":true}"#.len() as u32,
                )
            },
            NATIVE_STATUS_FAILED
        );
        assert_eq!(
            unsafe {
                read_snapshot(
                    host.package_context,
                    snapshot_bytes.as_mut_ptr(),
                    snapshot_bytes.len() as u32,
                    &mut required,
                )
            },
            NATIVE_STATUS_OK
        );
        let after_invalid: serde_json::Value =
            serde_json::from_slice(&snapshot_bytes[..required as usize]).unwrap();
        assert_eq!(after_invalid["revision"], current_revision);

        let original_thread = context.owner_thread;
        context.owner_thread = std::thread::spawn(|| std::thread::current().id())
            .join()
            .unwrap();
        assert_eq!(
            unsafe {
                read_snapshot(
                    host.package_context,
                    snapshot_bytes.as_mut_ptr(),
                    snapshot_bytes.len() as u32,
                    &mut required,
                )
            },
            NATIVE_STATUS_WRONG_THREAD
        );
        assert_eq!(
            unsafe {
                patch(
                    host.package_context,
                    current_revision.as_ptr(),
                    current_revision.len() as u32,
                    update.as_ptr(),
                    update.len() as u32,
                )
            },
            NATIVE_STATUS_WRONG_THREAD
        );
        context.owner_thread = original_thread;
        context.services_revoked.set(true);
        assert_eq!(
            unsafe {
                read_snapshot(
                    host.package_context,
                    snapshot_bytes.as_mut_ptr(),
                    snapshot_bytes.len() as u32,
                    &mut required,
                )
            },
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(
            unsafe {
                patch(
                    host.package_context,
                    current_revision.as_ptr(),
                    current_revision.len() as u32,
                    update.as_ptr(),
                    update.len() as u32,
                )
            },
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(size_of::<settings::SettingsService>(), 32);
    }

    #[test]
    fn native_settings_service_does_not_rebind_after_package_replacement() {
        let package = TestPackage::new();
        let mut context = Box::new(
            NativePluginHostContext::new(NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_binding: package.package_binding(),
                package_generation: 22,
                is_server: false,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            })
            .unwrap(),
        );
        let host = context.host_table();
        let name = settings::SERVICE.as_bytes();
        let mut table = std::mem::MaybeUninit::<settings::SettingsService>::uninit();
        assert_eq!(
            unsafe {
                host.query_service.unwrap()(
                    host.package_context,
                    name.as_ptr(),
                    name.len() as u32,
                    1,
                    size_of::<settings::SettingsService>() as u32,
                    table.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_OK
        );
        let table = unsafe { table.assume_init() };
        let read_snapshot = table.read_snapshot.unwrap();
        let patch = table.patch.unwrap();
        let mut required = 0;
        assert_eq!(
            unsafe { read_snapshot(host.package_context, std::ptr::null_mut(), 0, &mut required) },
            NATIVE_STATUS_BUFFER_TOO_SMALL
        );
        let mut snapshot_bytes = vec![0; required as usize];
        assert_eq!(
            unsafe {
                read_snapshot(
                    host.package_context,
                    snapshot_bytes.as_mut_ptr(),
                    snapshot_bytes.len() as u32,
                    &mut required,
                )
            },
            NATIVE_STATUS_OK
        );
        let snapshot: serde_json::Value = serde_json::from_slice(&snapshot_bytes).unwrap();
        let revision = snapshot["revision"].as_str().unwrap().to_owned();

        let package_root = package.root.join("mods").join("example");
        let removed_package = package.root.join("removed-original-package");
        let manifest_bytes = fs::read(package_root.join("mod.json")).unwrap();
        let mut replacement_extension: serde_json::Value =
            serde_json::from_slice(&fs::read(package_root.join("extended.mod.json")).unwrap())
                .unwrap();
        replacement_extension["settings"]["radius_multiplier"]["value"] = 7.0.into();
        fs::rename(&package_root, &removed_package).unwrap();
        fs::create_dir_all(&package_root).unwrap();
        fs::write(package_root.join("mod.json"), manifest_bytes).unwrap();
        fs::write(
            package_root.join("extended.mod.json"),
            serde_json::to_vec(&replacement_extension).unwrap(),
        )
        .unwrap();
        let replacement_before = fs::read(package_root.join("extended.mod.json")).unwrap();
        let original_before = fs::read(removed_package.join("extended.mod.json")).unwrap();

        assert_eq!(
            unsafe {
                read_snapshot(
                    host.package_context,
                    snapshot_bytes.as_mut_ptr(),
                    snapshot_bytes.len() as u32,
                    &mut required,
                )
            },
            NATIVE_STATUS_FAILED
        );
        let update = br#"{"radius_multiplier":9.0}"#;
        assert_eq!(
            unsafe {
                patch(
                    host.package_context,
                    revision.as_ptr(),
                    revision.len() as u32,
                    update.as_ptr(),
                    update.len() as u32,
                )
            },
            NATIVE_STATUS_FAILED
        );
        assert_eq!(
            fs::read(package_root.join("extended.mod.json")).unwrap(),
            replacement_before
        );
        assert_eq!(
            fs::read(removed_package.join("extended.mod.json")).unwrap(),
            original_before
        );
    }

    #[test]
    fn native_settings_service_requires_owner_runtime_capability_and_client_target() {
        for (package, is_server) in [
            (TestPackage::with_runtime(false), false),
            (TestPackage::with_target(true, "server"), false),
            (TestPackage::new(), true),
        ] {
            let mut context = Box::new(
                NativePluginHostContext::new(NativePluginHostMetadata {
                    package_id: "example".into(),
                    package: package.package.clone(),
                    package_binding: package.package_binding(),
                    package_generation: 19,
                    is_server,
                    installation_root: package.root.clone(),
                    settings_json: b"{}".to_vec(),
                    services: Arc::new(NoNativePluginServices),
                })
                .unwrap(),
            );
            let host = context.host_table();
            let name = settings::SERVICE.as_bytes();
            let mut table = 0u32;
            assert_eq!(
                unsafe {
                    host.query_service.unwrap()(
                        host.package_context,
                        name.as_ptr(),
                        name.len() as u32,
                        1,
                        size_of::<settings::SettingsService>() as u32,
                        (&mut table as *mut u32).cast(),
                    )
                },
                NATIVE_STATUS_UNAVAILABLE
            );
            assert_eq!(table, 0);
        }

        let package = TestPackage::new();
        let mismatched_owner = NativePluginHostContext::new(NativePluginHostMetadata {
            package_id: "another-mod".into(),
            package: package.package.clone(),
            package_binding: package.package_binding(),
            package_generation: 20,
            is_server: false,
            installation_root: package.root.clone(),
            settings_json: b"{}".to_vec(),
            services: Arc::new(NoNativePluginServices),
        });
        assert!(mismatched_owner.is_err());
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
                package_binding: package.package_binding(),
                package_generation: 7,
                is_server: false,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            })
            .unwrap(),
        );
        let host = context.host_table();
        let query = host.query_service.unwrap();
        for service_name in [
            NATIVE_SERVICE_MAP_V1,
            NATIVE_SERVICE_OVERLAY_V1,
            NATIVE_SERVICE_NOTICES_V1,
        ] {
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
                package_binding: package.package_binding(),
                package_generation: 9,
                is_server: false,
                installation_root: package.root.clone(),
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
                package_binding: package.package_binding(),
                package_generation: 8,
                is_server: false,
                installation_root: package.root.clone(),
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
                package_binding: package.package_binding(),
                package_generation: 41,
                is_server: false,
                installation_root: package.root.clone(),
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
            host.query_service.unwrap()(
                host.package_context,
                map_service.as_ptr(),
                map_service.len() as u32,
                1,
                size_of::<u32>() as u32,
                (&mut service_table as *mut u32).cast(),
            )
        };
        assert_eq!(terminal_query, NATIVE_STATUS_UNAVAILABLE);
        assert_eq!(service_table, 0xA5A5_A5A5);
    }

    #[test]
    fn host_drain_refuses_to_claim_quiescence_while_network_lease_is_active() {
        let package = TestPackage::new();
        let services = Arc::new(TestServiceLifetime {
            revoke_status: NATIVE_STATUS_OK,
            drain_status: NATIVE_STATUS_OK,
            queried_owner: Mutex::new(None),
            revoked_owner: Mutex::new(None),
            drained_owner: Mutex::new(None),
        });
        let mut context = Box::new(
            NativePluginHostContext::new(NativePluginHostMetadata {
                package_id: "example".into(),
                package: package.package.clone(),
                package_binding: package.package_binding(),
                package_generation: 58,
                is_server: false,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: services.clone(),
            })
            .unwrap(),
        );
        let host = context.host_table();
        let lease = network::OwnerServiceLease::acquire(&context).unwrap();

        assert_eq!(
            unsafe {
                host.drain_services.unwrap()(
                    host.package_context,
                    host.package_generation,
                    NATIVE_DRAIN_TIMEOUT_MS,
                )
            },
            NATIVE_STATUS_BUSY
        );
        assert!(services.drained_owner.lock().unwrap().is_none());
        assert!(context.network.has_active_call());

        drop(lease);
        assert_eq!(
            unsafe {
                host.drain_services.unwrap()(
                    host.package_context,
                    host.package_generation,
                    NATIVE_DRAIN_TIMEOUT_MS,
                )
            },
            NATIVE_STATUS_OK
        );
        assert_eq!(
            *services.drained_owner.lock().unwrap(),
            Some(("example".to_owned(), 58, NATIVE_DRAIN_TIMEOUT_MS))
        );
    }

    #[test]
    fn map_service_authority_is_checked_before_forwarding_for_each_generation() {
        for service in [NATIVE_SERVICE_MAP_V1, NATIVE_SERVICE_NOTICES_V1] {
            for (runtime, is_server) in [(false, false), (true, true), (true, false)] {
                let package = TestPackage::with_runtime(runtime);
                let services = Arc::new(TestServiceLifetime {
                    revoke_status: 0,
                    drain_status: 0,
                    queried_owner: Mutex::new(None),
                    revoked_owner: Mutex::new(None),
                    drained_owner: Mutex::new(None),
                });
                let mut context = Box::new(
                    NativePluginHostContext::new(NativePluginHostMetadata {
                        package_id: "example".into(),
                        package: package.package.clone(),
                        package_binding: package.package_binding(),
                        package_generation: 77,
                        is_server,
                        installation_root: package.root.clone(),
                        settings_json: b"{}".to_vec(),
                        services: services.clone(),
                    })
                    .unwrap(),
                );
                let host = context.host_table();
                let name = service.as_bytes();
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
                assert_eq!(
                    status,
                    if runtime && !is_server {
                        0
                    } else {
                        NATIVE_STATUS_UNAVAILABLE
                    }
                );
                assert_eq!(
                    services.queried_owner.lock().unwrap().is_some(),
                    runtime && !is_server
                );
                if !runtime || is_server {
                    assert_eq!(output, 0);
                }
            }
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
            package_binding: package.package_binding(),
            package_generation: generation,
            is_server: false,
            installation_root: package.root.clone(),
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
                package_binding: package.package_binding(),
                package_generation: 50,
                is_server: false,
                installation_root: package.root.clone(),
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

    fn process_lifetime_fixture(
        is_server: bool,
    ) -> (
        TestPackage,
        Arc<process_lifetime::Admission>,
        NativePluginInstance,
    ) {
        let generation = process_lifetime::next_generation().unwrap();
        let id = format!("lifetime-fixture-{generation}");
        let package = TestPackage::named(false, if is_server { "server" } else { "client" }, &id);
        let admission = process_lifetime::reserve(&id, generation).unwrap();
        admission
            .bind_module(bounded_entry as *const () as usize)
            .unwrap();
        let instance = NativePluginInstance::load(
            bounded_entry,
            NativePluginHostMetadata {
                package_id: id,
                package: package.package.clone(),
                package_binding: package.package_binding(),
                package_generation: generation,
                is_server,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            },
        )
        .unwrap();
        (package, admission, instance)
    }

    unsafe fn lifetime_query(
        host: &SFNativeHostV1,
        version: u32,
        capacity: u32,
        output: *mut c_void,
    ) -> i32 {
        let name = process_lifetime::SERVICE.as_bytes();
        unsafe {
            host.query_service.unwrap()(
                host.package_context,
                name.as_ptr(),
                name.len() as u32,
                version,
                capacity,
                output,
            )
        }
    }

    #[test]
    #[cfg(windows)]
    fn process_lifetime_query_does_not_retain_and_errors_leave_output_untouched() {
        use process_lifetime::{RetentionResult, Service};
        for is_server in [false, true] {
            let (_package, admission, instance) = process_lifetime_fixture(is_server);
            instance.host_context.initializing.set(true);
            let host = instance.host.as_ref();
            let mut table = Service {
                size: 99,
                version: 99,
                context: std::ptr::null_mut(),
                retain_until_process_exit: None,
            };
            let capacity = size_of::<Service>() as u32;
            assert_eq!(
                unsafe { lifetime_query(host, 2, capacity, (&mut table as *mut Service).cast()) },
                NATIVE_STATUS_ABI_MISMATCH
            );
            assert_eq!(table.size, 99);
            assert_eq!(
                unsafe {
                    lifetime_query(host, 1, capacity - 1, (&mut table as *mut Service).cast())
                },
                NATIVE_STATUS_BUFFER_TOO_SMALL
            );
            assert_eq!(table.size, 99);
            assert_eq!(
                unsafe { lifetime_query(host, 1, capacity, std::ptr::null_mut()) },
                NATIVE_STATUS_INVALID_ARGUMENT
            );
            assert_eq!(
                unsafe { lifetime_query(host, 1, capacity, (&mut table as *mut Service).cast()) },
                0
            );
            assert!(!admission.is_retained());
            let sentinel = RetentionResult {
                size: 9,
                version: 9,
                package_generation: 9,
                retained: 9,
                restart_required: 9,
            };
            let mut result = sentinel;
            let retain = table.retain_until_process_exit.unwrap();
            let out_size = size_of::<RetentionResult>() as u32;
            assert_eq!(
                unsafe { retain(std::ptr::null_mut(), out_size, &mut result) },
                NATIVE_STATUS_INVALID_ARGUMENT
            );
            assert_eq!(
                unsafe { retain(table.context, out_size - 1, &mut result) },
                NATIVE_STATUS_BUFFER_TOO_SMALL
            );
            assert_eq!(result, sentinel);
            assert_eq!(
                unsafe { retain(table.context, out_size, std::ptr::null_mut()) },
                NATIVE_STATUS_INVALID_ARGUMENT
            );
            assert!(!admission.is_retained());
            let address = table.context as usize;
            let status = std::thread::spawn(move || {
                let mut result = sentinel;
                let status = unsafe { retain(address as *mut c_void, out_size, &mut result) };
                assert_eq!(result, sentinel);
                status
            })
            .join()
            .unwrap();
            assert_eq!(status, NATIVE_STATUS_WRONG_THREAD);
            assert!(!admission.is_retained());
            assert_eq!(unsafe { retain(table.context, out_size, &mut result) }, 0);
            assert!(admission.is_retained());
            assert_eq!(
                result,
                RetentionResult {
                    size: out_size,
                    version: 1,
                    package_generation: host.package_generation,
                    retained: 1,
                    restart_required: 1
                }
            );
            assert_eq!(unsafe { retain(table.context, out_size, &mut result) }, 0); // exact-owner repeat
            instance.host_context.initializing.set(false);
            result = sentinel;
            assert_eq!(
                unsafe { retain(table.context, out_size, &mut result) },
                NATIVE_STATUS_UNAVAILABLE
            );
            assert_eq!(result, sentinel);
            instance.host_context.initializing.set(true);
            assert_eq!(
                unsafe {
                    host.revoke_services.unwrap()(host.package_context, host.package_generation)
                },
                0
            );
            assert_eq!(
                unsafe { retain(table.context, out_size, &mut result) },
                NATIVE_STATUS_UNAVAILABLE
            );
            assert_eq!(result, sentinel);
        }
    }

    struct LifetimeCallbackState {
        events: Vec<&'static str>,
        initialize_status: i32,
        shutdown_drops: Arc<std::sync::atomic::AtomicUsize>,
    }
    unsafe extern "C" fn lifetime_initialize(
        context: *mut c_void,
        host: *const SFNativeHostV1,
    ) -> i32 {
        let state = unsafe { &mut *context.cast::<LifetimeCallbackState>() };
        let host = unsafe { &*host };
        let mut table = std::mem::MaybeUninit::<process_lifetime::Service>::uninit();
        assert_eq!(
            unsafe {
                lifetime_query(
                    host,
                    1,
                    size_of::<process_lifetime::Service>() as u32,
                    table.as_mut_ptr().cast(),
                )
            },
            0
        );
        let table = unsafe { table.assume_init() };
        let mut result = std::mem::MaybeUninit::<process_lifetime::RetentionResult>::uninit();
        assert_eq!(
            unsafe {
                table.retain_until_process_exit.unwrap()(
                    table.context,
                    size_of::<process_lifetime::RetentionResult>() as u32,
                    result.as_mut_ptr(),
                )
            },
            0
        );
        state.events.push("initialize retained");
        state.initialize_status
    }
    unsafe extern "C" fn lifetime_stop(context: *mut c_void, _host: *const SFNativeHostV1) -> i32 {
        unsafe { &mut *context.cast::<LifetimeCallbackState>() }
            .events
            .push("stop");
        0
    }
    unsafe extern "C" fn lifetime_shutdown(
        context: *mut c_void,
        _host: *const SFNativeHostV1,
    ) -> i32 {
        let state = unsafe { Box::from_raw(context.cast::<LifetimeCallbackState>()) };
        state
            .shutdown_drops
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        0
    }
    #[test]
    #[cfg(windows)]
    fn process_lifetime_actual_instance_retains_state_on_stop_and_partial_initialize_failure() {
        for initialize_status in [0, NATIVE_STATUS_FAILED] {
            let (_package, admission, mut instance) = process_lifetime_fixture(false);
            let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let state = Box::into_raw(Box::new(LifetimeCallbackState {
                events: Vec::new(),
                initialize_status,
                shutdown_drops: drops.clone(),
            }));
            instance.plugin.plugin_context = state.cast();
            instance.plugin.initialize = Some(lifetime_initialize);
            instance.plugin.stop = Some(lifetime_stop);
            instance.plugin.shutdown = Some(lifetime_shutdown);
            let expected = TeardownOutcome::Retain {
                stage: "process lifetime; restart required",
                status: process_lifetime::RESTART_REQUIRED,
            };
            if initialize_status == 0 {
                instance.start().unwrap();
                assert_eq!(instance.stop(), expected);
            } else {
                assert_eq!(instance.start().unwrap_err().cleanup, expected);
            }
            assert_eq!(unsafe { &*state }.events, ["initialize retained", "stop"]);
            assert_eq!(drops.load(std::sync::atomic::Ordering::Relaxed), 0);
            assert!(admission.is_retained());
            let generation = instance.host.package_generation;
            let id = String::from_utf8(instance.host_context.package_id_utf8.to_vec()).unwrap();
            // Exactly the existing shutdown_native_dlls retained storage policy.
            std::mem::forget(instance);
            assert!(process_lifetime::reserve(&id, generation + 1).is_err());
            assert_eq!(unsafe { &*state }.events.last(), Some(&"stop"));
            // The synthetic producer owns this STATE. Its retained callback is
            // still readable; deliberately leave it alive through process exit.
        }
    }

    #[test]
    fn process_lifetime_c_layout_and_unrequested_teardown_stay_compatible() {
        assert_eq!(size_of::<process_lifetime::RetentionResult>(), 24);
        assert_eq!(size_of::<process_lifetime::Service>(), 24);
        assert_eq!(size_of::<SFNativeHostV1>(), 96);
        assert_eq!(size_of::<SFNativePluginV1>(), 48);
        let (_package, admission, mut instance) = process_lifetime_fixture(false);
        instance.start().unwrap();
        assert!(!admission.is_retained());
        assert_eq!(instance.stop(), TeardownOutcome::Quiescent);
    }

    #[test]
    fn process_lifetime_query_only_does_not_change_normal_instance_teardown() {
        unsafe extern "C" fn query_only(_: *mut c_void, host: *const SFNativeHostV1) -> i32 {
            let mut table = std::mem::MaybeUninit::<process_lifetime::Service>::uninit();
            unsafe {
                lifetime_query(
                    &*host,
                    1,
                    size_of::<process_lifetime::Service>() as u32,
                    table.as_mut_ptr().cast(),
                )
            }
        }
        let (_package, admission, mut instance) = process_lifetime_fixture(false);
        instance.plugin.initialize = Some(query_only);
        instance.start().unwrap();
        assert!(!admission.is_retained());
        assert_eq!(instance.stop(), TeardownOutcome::Quiescent);
    }

    #[test]
    #[cfg(windows)]
    fn process_lifetime_update_failure_and_duplicate_host_cannot_release_state() {
        unsafe extern "C" fn failed_update(
            _: *mut c_void,
            _: *const SFNativeHostV1,
            _: u64,
        ) -> i32 {
            NATIVE_STATUS_FAILED
        }
        let (package, admission, mut instance) = process_lifetime_fixture(false);
        let duplicate = NativePluginInstance::load(
            bounded_entry,
            NativePluginHostMetadata {
                package_id: package.package.info().id.clone(),
                package: package.package.clone(),
                package_binding: package.package_binding(),
                package_generation: instance.host.package_generation,
                is_server: false,
                installation_root: package.root.clone(),
                settings_json: b"{}".to_vec(),
                services: Arc::new(NoNativePluginServices),
            },
        );
        assert!(duplicate.is_err()); // before entry; this generation has one exact host
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let state = Box::into_raw(Box::new(LifetimeCallbackState {
            events: Vec::new(),
            initialize_status: 0,
            shutdown_drops: drops.clone(),
        }));
        instance.plugin.plugin_context = state.cast();
        instance.plugin.initialize = Some(lifetime_initialize);
        instance.plugin.update = Some(failed_update);
        instance.plugin.stop = Some(lifetime_stop);
        instance.plugin.shutdown = Some(lifetime_shutdown);
        instance.start().unwrap();
        assert_eq!(instance.update(1), NATIVE_STATUS_FAILED);
        // update_native_plugins routes this failure to stop_native_plugin.
        assert_eq!(
            instance.stop(),
            TeardownOutcome::Retain {
                stage: "process lifetime; restart required",
                status: process_lifetime::RESTART_REQUIRED,
            }
        );
        assert!(admission.is_retained());
        assert_eq!(drops.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert!(instance.host_context.services_revoked.get());
        std::mem::forget(instance);
        assert_eq!(unsafe { &*state }.events, ["initialize retained", "stop"]);
    }
}
