use super::native_plugin_abi::{NATIVE_STATUS_UNAVAILABLE, NativePluginServiceLifetime};
use mod_loader::ModEnvironment;
use std::ffi::c_void;
/// Immutable control-plane authority. No AppState/RefCell is retained by this
/// object and native callbacks dispatch directly inside the provider.
pub(crate) struct MapOverlayServices {
    client: bool,
}
impl MapOverlayServices {
    pub(crate) fn new(env: &ModEnvironment, is_server: bool) -> Self {
        let _ = env;
        Self { client: !is_server }
    }
}
impl NativePluginServiceLifetime for MapOverlayServices {
    unsafe fn query_service(
        &self,
        package: &str,
        generation: u64,
        name: &str,
        version: u32,
        capacity: u32,
        output: *mut c_void,
    ) -> i32 {
        if !self.client {
            return NATIVE_STATUS_UNAVAILABLE;
        }
        unsafe {
            shroudforge_runtime_bridge::query_map_overlay_service(
                package, generation, name, version, capacity, output,
            )
        }
    }
    fn revoke(&self, package: &str, generation: u64) -> i32 {
        shroudforge_runtime_bridge::revoke_map_overlay_services(package, generation)
    }
    fn drain(&self, package: &str, generation: u64, timeout: u32) -> i32 {
        shroudforge_runtime_bridge::drain_map_overlay_services(package, generation, timeout)
    }
}
impl super::AppState {
    pub(crate) fn install_map_overlay_services(&self) -> Result<(), String> {
        self.set_native_plugin_service_lifetime(std::sync::Arc::new(MapOverlayServices::new(
            self.env(),
            self.is_server(),
        )))
    }
}

static LUA_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
impl super::AppState {
    pub(crate) fn lua_map_owner(&self, id: &str) -> u64 {
        *self
            .map_lua_owners
            .borrow_mut()
            .entry(id.to_owned())
            .or_insert_with(|| {
                (1u64 << 63) | LUA_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            })
    }
    pub(crate) fn release_lua_map_owner(&self, id: &str) {
        if let Some(generation) = self.map_lua_owners.borrow_mut().remove(id) {
            let revoke = shroudforge_runtime_bridge::revoke_map_overlay_services(id, generation);
            let drain = shroudforge_runtime_bridge::drain_map_overlay_services(id, generation, 250);
            if revoke != 0 || drain != 0 {
                tracing::warn!(
                    mod_id = id,
                    revoke,
                    drain,
                    "Lua map owner retained pending native drain"
                );
            }
        }
    }
    pub(crate) fn release_all_lua_map_owners(&self) {
        let ids = self
            .map_lua_owners
            .borrow()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for id in ids {
            self.release_lua_map_owner(&id);
        }
    }
}
