#[path = "asset_preparation/mod.rs"]
mod pregame;
#[cfg(any(windows, target_os = "linux"))]
#[path = "start_updater_controller.rs"]
mod start_updater_controller;
#[cfg(test)]
mod tests;

use shroudforge_api::{IngameRuntime, ShroudForgeApi};
use std::any::Any;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub use pregame::run as prepare;

#[derive(Debug, Error)]
pub enum LoaderError {
    #[error("loader environment error: {0}")]
    Environment(String),
    #[error("pregame phase failed: {0}")]
    Pregame(String),
    #[error("runtime initialization failed: {0}")]
    Runtime(String),
}

#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub lua: Vec<String>,
}

/// Run update-versioned migrations with the installed updater before the loader
/// reads data that the update may have changed.
#[cfg(windows)]
pub fn run_post_install_migrations(root: &Path) -> Result<(), String> {
    use std::{io::Write, os::windows::process::CommandExt, process::Stdio, time::Duration};

    let executable = shroudforge_loader_config::paths::updater_executable(root);
    if !executable.is_file() {
        return Ok(());
    }
    let request = serde_json::to_vec(&serde_json::json!({
        "command": "run-post-install-maintenance"
    }))
    .map_err(|error| format!("could not encode post-install maintenance request: {error}"))?;

    for attempt in 0..40 {
        let mut child = std::process::Command::new(&executable)
            .args(["--request", "--root"])
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|error| format!("could not start the installed updater: {error}"))?;
        child
            .stdin
            .take()
            .ok_or("installed updater request stdin is unavailable")?
            .write_all(&request)
            .map_err(|error| format!("could not send post-install maintenance request: {error}"))?;
        let output = child
            .wait_with_output()
            .map_err(|error| format!("could not read post-install maintenance result: {error}"))?;
        let response: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("installed updater returned an invalid response: {error}"))?;
        if output.status.success() && response["ok"] == true {
            return Ok(());
        }
        let error = response["error"]
            .as_str()
            .unwrap_or("installed updater could not finish post-install maintenance");
        let worker_is_busy = error.contains("post-install maintenance was deferred")
            || error.contains("previous update queue finishes");
        if !worker_is_busy || attempt == 39 {
            let message = format!("post-install migration failed: {error}");
            let _ = shroudforge_logging::append(root, 'E', "updater", &message);
            return Err(message);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn run_post_install_migrations(_: &Path) -> Result<(), String> {
    Ok(())
}

pub(crate) fn recycle_incompatible_game_asset_backups(
    root: &Path,
    target: &str,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::{io::Write, os::windows::process::CommandExt, process::Stdio};

        let request = serde_json::json!({
            "command": "recycle-game-asset-backups",
            "target": target,
        });
        let executable = shroudforge_loader_config::paths::updater_executable(root);
        if !executable.is_file() {
            return Err(
                "ShroudForge Updater is unavailable, so the old asset backups were kept".into(),
            );
        }
        let mut child = std::process::Command::new(executable)
            .args(["--request", "--root"])
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|error| format!("could not start ShroudForge Updater: {error}"))?;
        child
            .stdin
            .take()
            .ok_or("ShroudForge Updater request stdin is unavailable")?
            .write_all(&serde_json::to_vec(&request).map_err(|error| error.to_string())?)
            .map_err(|error| format!("could not send recycle-bin request: {error}"))?;
        let output = child
            .wait_with_output()
            .map_err(|error| format!("could not read recycle-bin result: {error}"))?;
        let response: serde_json::Value =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                format!("ShroudForge Updater returned an invalid response: {error}")
            })?;
        if output.status.success() && response["ok"] == true {
            Ok(())
        } else {
            Err(response["error"]
                .as_str()
                .unwrap_or("could not move the incompatible asset backups to the recycle bin")
                .to_owned())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (root, target);
        Err("moving incompatible game asset backups to the recycle bin is available on Windows only".into())
    }
}

pub struct ModLoader {
    api: Option<ShroudForgeApi>,
    runtime: Option<IngameRuntime>,
    is_client: bool,
}

impl ModLoader {
    pub fn new(api: ShroudForgeApi, is_client: bool) -> Self {
        Self {
            api: Some(api),
            runtime: None,
            is_client,
        }
    }

    pub fn load_directory(&mut self, game: impl AsRef<Path>) -> Result<LoadReport, LoaderError> {
        let game = game.as_ref();
        let root = game
            .to_str()
            .ok_or_else(|| LoaderError::Environment("game path is not UTF-8".into()))?;
        let environment = shroudforge_package::ModEnvironment::load(root)
            .map_err(|report| LoaderError::Environment(format!("{report:?}")))?;
        let file_name = if self.is_client {
            "enshrouded"
        } else {
            "enshrouded_server"
        };
        let runtime = match &self.api {
            Some(api) => IngameRuntime::start(&environment, api.clone(), file_name),
            None => IngameRuntime::start_live(&environment, file_name),
        }
        .map_err(|error| LoaderError::Runtime(error.to_string()))?;
        let report = LoadReport {
            lua: runtime.active_mod_ids(),
        };
        self.runtime = Some(runtime);
        Ok(report)
    }

    pub fn update(&mut self, delta_seconds: f64) {
        if let Some(runtime) = &mut self.runtime {
            runtime.update(delta_seconds);
        }
    }
}

#[repr(C)]
pub struct RuntimeHandle(ModLoader);

/// Private selected Stage1 diagnostic path, called synchronously before the
/// bootstrap releases PE entry. No ordinary SDK initialization/adoption.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_prepare_early_owner_probe(host: *const std::ffi::c_void) -> u32 {
    std::panic::catch_unwind(|| unsafe { shroudforge_api::prepare_early_owner_probe(host) }).unwrap_or(2)
}
#[unsafe(no_mangle)]
pub extern "C" fn shroudforge_tick_early_owner_probe() -> u32 {
    std::panic::catch_unwind(shroudforge_api::tick_early_owner_probe).unwrap_or(2)
}
#[unsafe(no_mangle)]
pub extern "C" fn shroudforge_revoke_early_owner_probe() {
    let _ = std::panic::catch_unwind(shroudforge_api::revoke_early_owner_probe);
}

fn panic_detail(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "panic payload was not a string".to_owned()
    }
}

/// 0: no demand; 1: explicit enabled client native-overlay demand; 2: unavailable.
/// Kept separate from runtime creation so the bootstrap can arm before main.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_startup_overlay_demand(game: *const u16) -> u32 {
    std::panic::catch_unwind(|| {
        if game.is_null() { return 2; }
        let mut length = 0;
        unsafe { while length < 32768 && *game.add(length) != 0 { length += 1; } }
        if length == 32768 { return 2; }
        let root = PathBuf::from(String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(game, length) }));
        if !root.join("enshrouded.exe").is_file() { return 0; }
        let Some(path) = root.to_str() else { return 2; };
        let result = shroudforge_package::ModEnvironment::load(path)
            .map_err(|error| format!("{error:?}"))
            .and_then(|env| shroudforge_api::native_overlay_startup_demand(&env));
        match result {
            Ok(value) => u32::from(value),
            Err(error) => { tracing::warn!(target: "shroudforge::startup", %error, "Overlay startup demand unavailable; rendering requires restart after configuration repair"); 2 }
        }
    }).unwrap_or(2)
}

/// 0: no demand; 1: the enabled client First Person package is in the plan;
/// 2: the plan could not be read. This runs before the executable entrypoint
/// is released so the native provider can prepare only the requested stubs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_startup_first_person_demand(game: *const u16) -> u32 {
    std::panic::catch_unwind(|| {
        if game.is_null() { return 2; }
        let mut length = 0;
        unsafe { while length < 32768 && *game.add(length) != 0 { length += 1; } }
        if length == 32768 { return 2; }
        let root = PathBuf::from(String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(game, length) }));
        if !root.join("enshrouded.exe").is_file() { return 0; }
        let Some(path) = root.to_str() else { return 2; };
        match shroudforge_package::ModEnvironment::load(path)
            .map_err(|error| format!("{error:?}"))
            .and_then(|environment| shroudforge_api::first_person_startup_demand(&environment)) {
            Ok(value) => u32::from(value),
            Err(error) => {
                tracing::warn!(target: "shroudforge::startup", %error, "First Person startup demand unavailable; early preparation is disabled");
                2
            }
        }
    }).unwrap_or(2)
}

fn vein_mining_startup_demand_code(result: Result<bool, String>) -> u32 {
    match result {
        Ok(demand) => u32::from(demand),
        Err(error) => {
            tracing::warn!(target: "shroudforge::startup", %error,
                "Vein Mining server startup demand unavailable; native preparation remains disabled");
            2
        }
    }
}

/// 0: no enabled qualified server package; 1: enabled `sf-vein-mining` is in
/// the dedicated-server runtime plan; 2: package or plan configuration is unavailable.
/// The bootstrap calls this only for the actual dedicated-server process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_startup_vein_mining_demand(game: *const u16) -> u32 {
    let result = std::panic::catch_unwind(|| {
        if game.is_null() {
            return 2;
        }
        let mut length = 0;
        unsafe {
            while length < 32768 && *game.add(length) != 0 {
                length += 1;
            }
        }
        if length == 32768 {
            return 2;
        }
        let root = PathBuf::from(String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(game, length)
        }));
        if !root.join("enshrouded_server.exe").is_file() {
            return 0;
        }
        let Some(path) = root.to_str() else {
            return vein_mining_startup_demand_code(Err(
                "dedicated-server root is not a UTF-8 path".into(),
            ));
        };
        let demand = shroudforge_package::ModEnvironment::load(path)
            .map_err(|error| format!("{error:?}"))
            .and_then(|environment| shroudforge_api::vein_mining_startup_demand(&environment));
        vein_mining_startup_demand_code(demand)
    });
    match result {
        Ok(demand) => demand,
        Err(payload) => {
            tracing::warn!(target: "shroudforge::startup",
                detail = %panic_detail(&*payload),
                "Vein Mining server startup demand unavailable after a configuration panic; native preparation remains disabled");
            2
        }
    }
}

fn gifting_range_startup_demand_code(result: Result<bool, String>) -> u32 {
    match result {
        Ok(demand) => u32::from(demand),
        Err(error) => {
            tracing::warn!(target: "shroudforge::startup", %error,
                "Unlimited Gifting Range server startup demand unavailable; native preparation remains disabled");
            2
        }
    }
}

/// 0: no enabled server adapter; 1: the explicit Shroudweave gifting package
/// is in the dedicated-server runtime plan; 2: package or plan unavailable.
/// The bootstrap calls this only for the actual dedicated-server process.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_startup_gifting_range_demand(game: *const u16) -> u32 {
    let result = std::panic::catch_unwind(|| {
        if game.is_null() {
            return 2;
        }
        let mut length = 0;
        unsafe {
            while length < 32768 && *game.add(length) != 0 {
                length += 1;
            }
        }
        if length == 32768 {
            return 2;
        }
        let root = PathBuf::from(String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(game, length)
        }));
        if !root.join("enshrouded_server.exe").is_file() {
            return 0;
        }
        let Some(path) = root.to_str() else {
            return gifting_range_startup_demand_code(Err(
                "dedicated-server root is not a UTF-8 path".into(),
            ));
        };
        let demand = shroudforge_package::ModEnvironment::load(path)
            .map_err(|error| format!("{error:?}"))
            .and_then(|environment| shroudforge_api::gifting_range_startup_demand(&environment));
        gifting_range_startup_demand_code(demand)
    });
    match result {
        Ok(demand) => demand,
        Err(payload) => {
            tracing::warn!(target: "shroudforge::startup",
                detail = %panic_detail(&*payload),
                "Unlimited Gifting Range server startup demand unavailable after a configuration panic; native preparation remains disabled");
            2
        }
    }
}

/// 0: disabled or no HD current-assets plan; 1: verified supported startup cap;
/// 2: a required demand could not be admitted. This consumes the preparation's
/// thread-local, root-bound token exactly once before the entrypoint is released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_take_startup_zoom_demand(game: *const u16) -> u32 {
    let result = std::panic::catch_unwind(|| {
        if game.is_null() {
            shroudforge_api::current_assets::clear_startup_zoom_demand();
            return 2;
        }
        let mut length = 0;
        unsafe {
            while length < 32768 && *game.add(length) != 0 {
                length += 1;
            }
        }
        if length == 32768 {
            shroudforge_api::current_assets::clear_startup_zoom_demand();
            return 2;
        }
        let requested = PathBuf::from(String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(game, length)
        }));
        match shroudforge_api::current_assets::take_startup_zoom_export_demand(&requested) {
            Ok(demand) => demand,
            Err(error) => {
                tracing::warn!(target: "shroudforge::startup", %error, "Could not inspect current-assets startup zoom demand");
                2
            }
        }
    });
    match result {
        Ok(demand) => demand,
        Err(_) => {
            shroudforge_api::current_assets::clear_startup_zoom_demand();
            2
        }
    }
}

#[cfg(test)]
mod startup_zoom_demand_tests {
    use super::*;

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "shroudforge-startup-zoom-export-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("enshrouded.exe"), []).unwrap();
            Self(path)
        }

        fn argument(&self) -> Vec<u16> {
            self.0
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect()
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_hd_installation_returns_disabled_demand() {
        let root = TempRoot::new();
        let argument = root.argument();
        assert_eq!(
            unsafe { shroudforge_take_startup_zoom_demand(argument.as_ptr()) },
            0
        );
    }

    #[test]
    fn active_current_assets_without_successful_prepare_fails_closed() {
        let root = TempRoot::new();
        std::fs::create_dir_all(root.0.join("shroudforge/config")).unwrap();
        std::fs::write(
            root.0.join("shroudforge/config/current-assets-state.json"),
            b"{}",
        )
        .unwrap();
        let argument = root.argument();
        assert_eq!(
            unsafe { shroudforge_take_startup_zoom_demand(argument.as_ptr()) },
            2
        );
    }
}

#[cfg(test)]
mod startup_vein_mining_demand_tests {
    use super::{shroudforge_startup_vein_mining_demand, vein_mining_startup_demand_code};

    #[test]
    fn startup_demand_maps_disabled_enabled_and_unavailable_states() {
        assert_eq!(vein_mining_startup_demand_code(Ok(false)), 0);
        assert_eq!(vein_mining_startup_demand_code(Ok(true)), 1);
        assert_eq!(vein_mining_startup_demand_code(Err("plan error".into())), 2);
    }

    #[test]
    fn export_refuses_client_root_and_reports_server_configuration_error() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "shroudforge-vein-demand-export-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("mods")).unwrap();
        std::fs::write(root.join("enshrouded.exe"), []).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let argument = root
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        assert_eq!(
            unsafe { shroudforge_startup_vein_mining_demand(argument.as_ptr()) },
            0
        );

        shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
        std::fs::write(root.join("enshrouded_server.exe"), []).unwrap();
        let mods = root.join("mods");
        std::fs::remove_dir_all(&mods).unwrap();
        std::fs::write(&mods, b"invalid mods root").unwrap();
        assert_eq!(
            unsafe { shroudforge_startup_vein_mining_demand(argument.as_ptr()) },
            2
        );
    }
}

#[cfg(test)]
mod startup_gifting_range_demand_tests {
    use super::{gifting_range_startup_demand_code, shroudforge_startup_gifting_range_demand};

    #[test]
    fn startup_demand_maps_disabled_enabled_and_unavailable_states() {
        assert_eq!(gifting_range_startup_demand_code(Ok(false)), 0);
        assert_eq!(gifting_range_startup_demand_code(Ok(true)), 1);
        assert_eq!(
            gifting_range_startup_demand_code(Err("plan error".into())),
            2
        );
    }

    #[test]
    fn export_refuses_client_roots_and_only_reads_dedicated_server_roots() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "shroudforge-gifting-demand-export-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("mods")).unwrap();
        std::fs::write(root.join("enshrouded.exe"), []).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let argument = root
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        assert_eq!(
            unsafe { shroudforge_startup_gifting_range_demand(argument.as_ptr()) },
            0
        );

        shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
        std::fs::write(root.join("enshrouded_server.exe"), []).unwrap();
        assert_eq!(
            unsafe { shroudforge_startup_gifting_range_demand(argument.as_ptr()) },
            0
        );
    }
}

fn prepare_native_dlls_before_game_entry(game: &Path) -> bool {
    let is_server = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .is_some_and(|name| name.eq_ignore_ascii_case("enshrouded_server.exe"));
    match std::panic::catch_unwind(|| shroudforge_api::prepare_native_dlls(game, is_server)) {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            let _ = shroudforge_logging::append(game, 'E', "dll-manager", &error);
            false
        }
        Err(payload) => {
            let _ = shroudforge_logging::append(
                game,
                'E',
                "dll-manager",
                &format!("Native DLL preparation failed: {}", panic_detail(&*payload)),
            );
            false
        }
    }
}

/// Applies stale pregame asset mods from the bootstrap thread before the live
/// ECS runtime is created. This lets a normal game start use the bundled
/// loader instead of requiring a separate `prepare` command.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_prepare_startup(game: *const u16) -> bool {
    shroudforge_api::current_assets::clear_startup_zoom_demand();
    fn path(value: *const u16) -> Option<PathBuf> {
        if value.is_null() {
            return None;
        }
        let mut length = 0;
        unsafe {
            while *value.add(length) != 0 {
                length += 1;
            }
        }
        Some(PathBuf::from(String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(value, length)
        })))
    }
    match std::panic::catch_unwind(|| {
        let Some(game) = path(game) else {
            return false;
        };
        let _ = shroudforge_logging::initialize(&game, false);
        let started = std::time::Instant::now();
        let migrations_ready = match run_post_install_migrations(&game) {
            Ok(()) => true,
            Err(error) => {
                let _ = shroudforge_logging::append(
                    &game,
                    'E',
                    "updater",
                    &format!("Post-install migrations failed before startup preparation: {error}"),
                );
                false
            }
        };
        let startup_result = pregame::run_startup(&game);
        #[cfg(any(windows, target_os = "linux"))]
        if migrations_ready && let Err(error) = start_updater_controller::start(&game) {
            let _ = shroudforge_logging::append(
                &game,
                'E',
                "config-controller",
                &format!("Could not start headless config controller: {error}"),
            );
        }
        let assets_ready = match startup_result {
            Ok(()) => {
                let message = format!(
                    "Early startup asset pass completed in {} ms",
                    started.elapsed().as_millis()
                );
                let _ = shroudforge_logging::append(&game, 'I', "startup-assets", &message);
                true
            }
            Err(error) => {
                let _ =
                    shroudforge_logging::append(&game, 'W', "startup-assets", &error.to_string());
                pregame::recover_failed_startup(&game, &error.to_string())
            }
        };
        assets_ready && prepare_native_dlls_before_game_entry(&game)
    }) {
        Ok(result) => result,
        Err(payload) => {
            if let Some(game) = path(game) {
                let _ = shroudforge_logging::initialize(&game, false);
                let _ = shroudforge_logging::append(
                    &game,
                    'E',
                    "startup-assets",
                    &format!(
                        "Rust panic during startup asset preparation: {}",
                        panic_detail(&*payload)
                    ),
                );
                let assets_ready = pregame::recover_failed_startup(
                    &game,
                    &format!(
                        "Rust panic during startup asset preparation: {}",
                        panic_detail(&*payload)
                    ),
                );
                return assets_ready && prepare_native_dlls_before_game_entry(&game);
            }
            false
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_create(game: *const u16) -> *mut RuntimeHandle {
    fn path(value: *const u16) -> Option<PathBuf> {
        if value.is_null() {
            return None;
        }
        let mut length = 0;
        unsafe {
            while *value.add(length) != 0 {
                length += 1;
            }
        }
        Some(PathBuf::from(String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(value, length)
        })))
    }
    fn append_runtime_log(game: &Path, message: &str) {
        let _ = shroudforge_logging::append(game, 'E', "runtime", message);
    }
    fn create_runtime(game: PathBuf) -> Result<RuntimeHandle, String> {
        let _ = shroudforge_logging::initialize(&game, false);
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let name = executable
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let is_client = if name.eq_ignore_ascii_case("enshrouded.exe") {
            true
        } else if name.eq_ignore_ascii_case("enshrouded_server.exe") {
            false
        } else {
            return Err(format!("unsupported host process: {name}"));
        };
        let mut loader = ModLoader {
            api: None,
            runtime: None,
            is_client,
        };
        loader
            .load_directory(&game)
            .map_err(|error| format!("mod loading failed at {}: {error}", game.display()))?;
        Ok(RuntimeHandle(loader))
    }
    match std::panic::catch_unwind(|| {
        let Some(game) = path(game) else {
            return std::ptr::null_mut();
        };
        match create_runtime(game.clone()) {
            Ok(handle) => Box::into_raw(Box::new(handle)),
            Err(error) => {
                append_runtime_log(&game, &format!("Runtime initialization failed: {error}"));
                std::ptr::null_mut()
            }
        }
    }) {
        Ok(handle) => handle,
        Err(payload) => {
            if let Some(game) = path(game) {
                let _ = shroudforge_logging::initialize(&game, false);
                append_runtime_log(
                    &game,
                    &format!(
                        "Rust panic during runtime initialization: {}",
                        panic_detail(&*payload)
                    ),
                );
            }
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_update(
    handle: *mut RuntimeHandle,
    delta_seconds: f64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        (*handle).0.update(delta_seconds)
    }))
    .is_ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shroudforge_destroy(handle: *mut RuntimeHandle) {
    if !handle.is_null() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(unsafe { Box::from_raw(handle) })
        }));
    }
}
