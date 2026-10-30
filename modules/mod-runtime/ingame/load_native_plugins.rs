use crate::{env::AppState, runner::LuaModRunner};
use mod_loader::ModEnvironment;

pub(super) fn load_native_plugins(
    env: &ModEnvironment,
    runner: &LuaModRunner,
    errors: &mut serde_json::Map<String, serde_json::Value>,
) {
    for target_mod in env.enabled_mods() {
        if let Some(reason) = errors.get(&target_mod.info().id) {
            tracing::warn!(target: "shroudforge::runtime", mod_id = %target_mod.info().id,
            reason = %reason, "Skipping native DLL scan for blocked mod");
            continue;
        }
        let app_state = runner.lua.app_data_ref::<AppState>().unwrap();
        tracing::debug!(target: "shroudforge::runtime", mod_id = %target_mod.info().id,
        "Checking native DLL declaration");
        match app_state.load_native_dll_declaration(target_mod) {
            Ok(true) => {
                tracing::debug!(target: "shroudforge::runtime", mod_id = %target_mod.info().id,
                "Native DLL queued for isolated loading");
                app_state.report_runtime_effect(
                    &target_mod.info().id,
                    "loading",
                    "native-plugin.ini DLL queued. Lua runtime continues independently",
                );
            }
            Ok(false) => {
                tracing::debug!(target: "shroudforge::runtime", mod_id = %target_mod.info().id,
            "No native DLL declaration")
            }
            Err(error) => {
                tracing::error!(target: "shroudforge::runtime", mod_id = %target_mod.info().id,
                "native DLL failed to load: {error}");
                app_state.report_runtime_effect(&target_mod.info().id, "error", &error);
                errors.insert(
                    target_mod.info().id.clone(),
                    format!("native DLL load failed: {error}").into(),
                );
            }
        }
    }
}
