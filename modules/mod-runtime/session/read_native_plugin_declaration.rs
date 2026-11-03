use super::{native_plugin_abi::NativePluginMode, *};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativePluginDeclaration {
    pub(super) enabled: bool,
    pub(super) dll: Option<String>,
    pub(super) mode: NativePluginMode,
    pub(super) overlay: bool,
    pub(super) preload_host: bool,
}

pub(super) fn parse_native_plugin_ini(config: &str) -> Result<NativePluginDeclaration, String> {
    let mut in_plugin_section = false;
    let mut enabled = false;
    let mut dll = None;
    let mut lifecycle = None;
    let mut overlay = false;
    let mut lifetime = None;
    for raw_line in config.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_plugin_section = line[1..line.len() - 1]
                .trim()
                .eq_ignore_ascii_case("Plugin");
            continue;
        }
        if !in_plugin_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "enabled" => {
                enabled = matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            }
            "dll" => dll = Some(value.to_owned()),
            "lifecycle" => lifecycle = Some(value.to_owned()),
            "lifetime" => {
                if lifetime.replace(value.to_owned()).is_some() {
                    return Err("duplicate native plugin Lifetime".into());
                }
            }
            "services" => {
                overlay = value.split(',').any(|name| {
                    matches!(name.trim(), "shroudforge.overlay" | "shroudforge.notices")
                })
            }
            _ => {}
        }
    }

    let mode = match lifecycle {
        None => NativePluginMode::LegacyLoadOnly,
        Some(mode) if mode.eq_ignore_ascii_case("shroudforge-v1") => {
            NativePluginMode::ShroudForgeV1
        }
        Some(mode) => return Err(format!("unsupported native plugin lifecycle '{mode}'")),
    };
    let preload_host = match lifetime {
        None => false,
        Some(value)
            if value.eq_ignore_ascii_case("host-preload-v1")
                && mode == NativePluginMode::ShroudForgeV1 =>
        {
            true
        }
        Some(value) => {
            return Err(format!(
                "unsupported or incompatible native plugin Lifetime '{value}'"
            ));
        }
    };
    Ok(NativePluginDeclaration {
        enabled,
        dll,
        mode,
        overlay,
        preload_host,
    })
}

pub(super) fn read_native_plugin_declaration(
    target_mod: &Mod,
) -> Result<Option<NativePluginDeclaration>, String> {
    let config_path = camino::Utf8Path::new("native-plugin.ini");
    let mut filesystem = target_mod.fs();
    if !filesystem.is_file(config_path) {
        return Ok(None);
    }
    let mut config = String::new();
    filesystem
        .read_file(config_path)
        .map_err(|error| format!("could not read native-plugin.ini: {error}"))?
        .read_to_string(&mut config)
        .map_err(|error| format!("could not decode native-plugin.ini: {error}"))?;
    parse_native_plugin_ini(&config).map(Some)
}

/// Control-plane declaration only; no plugin loading or frame traffic occurs here.
pub(crate) fn native_overlay_startup_demand(environment: &ModEnvironment) -> Result<bool, String> {
    let (runtime_plan, _) = environment.runtime_plan_report(false, crate::API_VERSION);
    if runtime_plan
        .iter()
        .any(|package| lua_notice_demand(package.info()))
    {
        return Ok(true);
    }
    let root = environment.game_dir();
    let plan = mod_loader::dll_manager::resolve_load_plan(root.as_std_path(), false);
    for package in environment.plan(false, crate::API_VERSION) {
        if !package.info().enabled
            || !package.info().target.supports_process(false)
            || !package
                .info()
                .capabilities
                .contains(&Capability::RuntimeRegisterDll)
        {
            continue;
        }
        if let Some(declaration) = read_native_plugin_declaration(package)? {
            if declaration.enabled
                && declaration.overlay
                && declaration.mode == NativePluginMode::ShroudForgeV1
                && declaration.dll.as_ref().is_some_and(|dll| !dll.is_empty())
            {
                for entry in &plan.entries {
                    if entry.mod_id.as_deref() == Some(package.info().id.as_str())
                        && entry.mode == mod_loader::dll_manager::DllLoadMode::Library
                        && managed_native_path(
                            root.as_std_path(),
                            &entry.package_directory,
                            &entry.path,
                        )?
                    {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}

fn lua_notice_demand(info: &mod_loader::ModManifest) -> bool {
    info.enabled
        && info.target.supports_process(false)
        && info.capabilities.contains(&Capability::Runtime)
        && info.capabilities.contains(&Capability::RuntimeNotices)
}

/// Prepare First Person stubs only when the enabled client runtime plan
/// contains the built-in First Person package. This is a startup-only demand;
/// enabling the package after the entrypoint is released requires a restart.
pub(crate) fn first_person_startup_demand(environment: &ModEnvironment) -> Result<bool, String> {
    let (plan, issues) = environment.runtime_plan_report(false, crate::API_VERSION);
    if !issues.is_empty() {
        tracing::warn!(target: "shroudforge::startup", issues = ?issues,
            "Runtime plan has unavailable packages while resolving First Person startup demand");
    }
    Ok(plan.iter().any(|package| {
        let info = package.info();
        info.enabled
            && info.target.supports_process(false)
            && info.capabilities.contains(&Capability::Runtime)
            && info.id == "shroudweave-first-person"
    }))
}

/// Prepare the Vein Mining native hooks only when the enabled server package
/// belongs to the executable dedicated-server runtime plan. The bootstrap calls
/// this while the real executable entrypoint is still held.
pub(crate) fn vein_mining_startup_demand(environment: &ModEnvironment) -> Result<bool, String> {
    const MOD_ID: &str = "sf-vein-mining";

    let package = match environment.mod_registry().get(MOD_ID) {
        Some(package) => package,
        None => match mod_loader::package_lookup::resolve_mod_package(
            environment.game_dir().as_std_path(),
            MOD_ID,
        ) {
            Ok(_) => {
                return Err(format!(
                    "installed {MOD_ID} package was rejected by the canonical package loader"
                ));
            }
            Err(error) if error == "mod package not found" => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "could not resolve installed {MOD_ID} package: {error}"
                ));
            }
        },
    };
    let info = package.info();
    if !info.enabled || !info.target.supports_process(true) {
        return Ok(false);
    }
    if !info.capabilities.contains(&Capability::Runtime) {
        return Err(format!(
            "enabled {MOD_ID} server package does not declare the required runtime capability"
        ));
    }

    // The enabled plan applies the canonical target, capability, compatibility,
    // dependency-presence, version, and dependency-enabled checks. The runtime
    // plan then confirms this package participates in live runtime work.
    let (enabled_plan, enabled_issues) = environment.plan_report_detailed(true, crate::API_VERSION);
    if !enabled_plan
        .iter()
        .any(|candidate| candidate.info().id == MOD_ID)
    {
        if let Some(issue) = enabled_issues
            .iter()
            .find(|issue| issue.mod_id == MOD_ID || issue.mod_id == "__loader__")
        {
            return Err(format!(
                "enabled {MOD_ID} package is unavailable in the dedicated-server plan: {}",
                issue.detail
            ));
        }
        return Ok(false);
    }

    let (runtime_plan, runtime_issues) =
        environment.runtime_plan_report_detailed(true, crate::API_VERSION);
    if runtime_plan
        .iter()
        .any(|candidate| candidate.info().id == MOD_ID)
    {
        return Ok(true);
    }
    if let Some(issue) = runtime_issues
        .iter()
        .find(|issue| issue.mod_id == MOD_ID || issue.mod_id == "__loader__")
    {
        return Err(format!(
            "enabled {MOD_ID} package is unavailable in the dedicated-server runtime plan: {}",
            issue.detail
        ));
    }

    Err(format!(
        "enabled {MOD_ID} package did not qualify for the dedicated-server runtime plan"
    ))
}

/// Prepare the Shroudweave Unlimited Gifting Range adapter only when its
/// explicit server package participates in the dedicated-server runtime plan.
/// This adapter does not depend on player/world ownership preparation.
pub(crate) fn gifting_range_startup_demand(environment: &ModEnvironment) -> Result<bool, String> {
    const MOD_ID: &str = "shroudweave-unlimited-gifting-range-server";

    let package = match environment.mod_registry().get(MOD_ID) {
        Some(package) => package,
        None => match mod_loader::package_lookup::resolve_mod_package(
            environment.game_dir().as_std_path(),
            MOD_ID,
        ) {
            Ok(_) => {
                return Err(format!(
                    "installed {MOD_ID} package was rejected by the canonical package loader"
                ));
            }
            Err(error) if error == "mod package not found" => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "could not resolve installed {MOD_ID} package: {error}"
                ));
            }
        },
    };
    let info = package.info();
    if !info.enabled || !info.target.supports_process(true) {
        return Ok(false);
    }
    if !info.capabilities.contains(&Capability::Runtime) {
        return Err(format!(
            "enabled {MOD_ID} server package does not declare the required runtime capability"
        ));
    }

    let (enabled_plan, enabled_issues) = environment.plan_report_detailed(true, crate::API_VERSION);
    if !enabled_plan
        .iter()
        .any(|candidate| candidate.info().id == MOD_ID)
    {
        if let Some(issue) = enabled_issues
            .iter()
            .find(|issue| issue.mod_id == MOD_ID || issue.mod_id == "__loader__")
        {
            return Err(format!(
                "enabled {MOD_ID} package is unavailable in the dedicated-server plan: {}",
                issue.detail
            ));
        }
        return Ok(false);
    }

    let (runtime_plan, runtime_issues) =
        environment.runtime_plan_report_detailed(true, crate::API_VERSION);
    if runtime_plan
        .iter()
        .any(|candidate| candidate.info().id == MOD_ID)
    {
        return Ok(true);
    }
    if let Some(issue) = runtime_issues
        .iter()
        .find(|issue| issue.mod_id == MOD_ID || issue.mod_id == "__loader__")
    {
        return Err(format!(
            "enabled {MOD_ID} package is unavailable in the dedicated-server runtime plan: {}",
            issue.detail
        ));
    }

    Err(format!(
        "enabled {MOD_ID} package did not qualify for the dedicated-server runtime plan"
    ))
}

impl AppState {
    pub(crate) fn load_native_dll_declaration(&self, target_mod: &Mod) -> Result<bool, String> {
        let Some(declaration) = read_native_plugin_declaration(target_mod)? else {
            return Ok(false);
        };
        if !declaration.enabled {
            return Ok(false);
        }
        let relative_path = declaration.dll.ok_or_else(|| {
            "native-plugin.ini enables DLL loading but does not name a DLL".to_owned()
        })?;
        match declaration.mode {
            NativePluginMode::LegacyLoadOnly => return Ok(false),
            NativePluginMode::ShroudForgeV1 => {
                self.validate_native_plugin_package(target_mod)?;
            }
        }
        tracing::debug!(target: "shroudforge::runtime", mod_id = %target_mod.info().id,
            dll = %relative_path, lifecycle = ?declaration.mode,
            "Native DLL declaration parsed. Queuing isolated load");
        self.queue_mod_native_dll(
            target_mod,
            &relative_path,
            declaration.mode,
            declaration.preload_host,
        )?;
        Ok(true)
    }

    pub(crate) fn restart_native_plugin(&self, mod_id: &str) -> Result<(), String> {
        let Some(target_mod) = self.env.mod_registry().get(mod_id) else {
            return Ok(());
        };
        let Some(declaration) = read_native_plugin_declaration(target_mod)? else {
            return Ok(());
        };
        if declaration.mode != NativePluginMode::ShroudForgeV1 || !declaration.enabled {
            return Ok(());
        }
        let relative_path = declaration.dll.ok_or_else(|| {
            "native-plugin.ini enables DLL loading but does not name a DLL".to_owned()
        })?;
        self.validate_native_plugin_package(target_mod)?;
        self.queue_mod_native_dll(
            target_mod,
            &relative_path,
            NativePluginMode::ShroudForgeV1,
            declaration.preload_host,
        )?;
        Ok(())
    }

    pub(super) fn validate_native_plugin_package(&self, target_mod: &Mod) -> Result<(), String> {
        validate_native_plugin_access(
            self.has_feature(AppFeatures::RUNTIME_DLL),
            target_mod
                .info()
                .capabilities
                .contains(&Capability::RuntimeRegisterDll),
            target_mod.info().target.supports_process(self.is_server()),
            if self.is_server() { "server" } else { "client" },
        )
    }
}

fn validate_native_plugin_access(
    runtime_available: bool,
    has_capability: bool,
    target_supports_process: bool,
    process_name: &str,
) -> Result<(), String> {
    if !runtime_available {
        return Err(
            "ShroudForge native plugin lifecycle is unavailable in this execution phase".into(),
        );
    }
    if !has_capability {
        return Err("ShroudForge native plugins require runtime-register-dll capability".into());
    }
    if !target_supports_process {
        return Err(format!(
            "native plugin package target does not include the {process_name} process"
        ));
    }
    Ok(())
}

impl AppState {
    pub(crate) fn load_mod_native_dll(
        &self,
        target_mod: &Mod,
        relative_path: &str,
    ) -> Result<(), String> {
        if !target_mod.info().enabled {
            self.report_runtime_effect(
                &target_mod.info().id,
                "restart-required",
                "Native DLL was skipped because the mod is disabled. Enable it and restart the game to load the DLL.",
            );
            tracing::info!(target: "shroudforge::runtime", mod_id = %target_mod.info().id,
                dll = %relative_path, "Native DLL skipped because the mod is disabled. Restart required after enabling");
            return Ok(());
        }
        let declaration = read_native_plugin_declaration(target_mod)?;
        if let Some(declaration) = declaration {
            if declaration.mode == NativePluginMode::ShroudForgeV1 {
                if !declaration.enabled {
                    self.report_runtime_effect(
                        &target_mod.info().id,
                        "disabled",
                        "ShroudForge native plugin is disabled in native-plugin.ini",
                    );
                    return Ok(());
                }
                let declared_path = declaration.dll.ok_or_else(|| {
                    "native-plugin.ini enables DLL loading but does not name a DLL".to_owned()
                })?;
                if declared_path != relative_path {
                    return Err(format!(
                        "register_dll path '{relative_path}' does not match opted-in native-plugin.ini DLL '{declared_path}'"
                    ));
                }
                self.validate_native_plugin_package(target_mod)?;
                return self.queue_mod_native_dll(
                    target_mod,
                    &declared_path,
                    NativePluginMode::ShroudForgeV1,
                    declaration.preload_host,
                );
            }
        }
        self.load_process_native_dll(target_mod, relative_path)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn notices_demand_requires_explicit_capabilities_and_enabled_client_plan() {
        for (enabled, runtime, notices, client, expected) in [
            (true, true, true, true, true),
            (false, true, true, true, false),
            (true, false, true, true, false),
            (true, true, false, true, false),
            (true, true, true, false, false),
        ] {
            let mut capabilities = vec![];
            if runtime {
                capabilities.push("runtime");
            }
            if notices {
                capabilities.push("runtime-notices");
            }
            let mut manifest: mod_loader::ModManifest = serde_json::from_value(serde_json::json!({
                "id":"arbitrary-owner","name":"Notice fixture","version":"1.0.0","enabled":enabled,
                "capabilities":capabilities,
            }))
            .unwrap();
            manifest.target = if client {
                mod_loader::ModTarget::Client
            } else {
                mod_loader::ModTarget::Server
            };
            assert_eq!(super::lua_notice_demand(&manifest), expected);
        }
        assert!(
            super::parse_native_plugin_ini("[Plugin]\nServices=shroudforge.notices")
                .unwrap()
                .overlay
        );
        assert!(
            !super::parse_native_plugin_ini("[Other]\nServices=shroudforge.notices")
                .unwrap()
                .overlay
        );
    }
    #[test]
    fn notices_startup_demand_uses_real_lua_plan_without_native_dll() {
        let root = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("notices-boot-{}", uuid::Uuid::new_v4()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        let package = root.join("mods/arbitrary-notice-owner");
        std::fs::create_dir_all(package.join("src")).unwrap();
        shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
        std::fs::write(package.join("src/mod.lua"), "return {}").unwrap();
        for (enabled, runtime, notices, client, expected) in [
            (true, true, true, true, true),
            (false, true, true, true, false),
            (true, false, true, true, false),
            (true, true, false, true, false),
            (true, true, true, false, false),
        ] {
            let mut capabilities = vec![];
            if runtime {
                capabilities.push("runtime");
            }
            if notices {
                capabilities.push("runtime-notices");
            }
            std::fs::write(package.join("mod.json"),serde_json::to_vec(&serde_json::json!({
                "id":"arbitrary-notice-owner","name":"Notices fixture","version":"1.0.0","capabilities":capabilities,
            })).unwrap()).unwrap();
            std::fs::write(package.join("extended.mod.json"),serde_json::to_vec(&serde_json::json!({
                "schemaVersion":1,"enabled":enabled,"targets":if client {vec!["client"]}else{vec!["server"]},
            })).unwrap()).unwrap();
            let environment =
                mod_loader::ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap())
                    .unwrap();
            assert_eq!(
                super::native_overlay_startup_demand(&environment).unwrap(),
                expected
            );
        }
    }
    use super::ModEnvironment;
    use super::native_plugin_abi::NativePluginMode;
    use super::{
        first_person_startup_demand, gifting_range_startup_demand, parse_native_plugin_ini,
        validate_native_plugin_access, vein_mining_startup_demand,
    };

    #[test]
    fn first_person_startup_demand_uses_enabled_client_runtime_plan() {
        let root = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("first-person-boot-{}", uuid::Uuid::new_v4()));
        let package = root.join("mods/shroudweave-first-person");
        std::fs::create_dir_all(package.join("src")).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
        std::fs::write(package.join("src/mod.lua"), "return {}").unwrap();

        for (enabled, runtime_capability, client, first_person_package, expected) in [
            (true, true, true, true, true),
            (false, true, true, true, false),
            (true, false, true, true, false),
            (true, true, false, true, false),
            (true, true, true, false, false),
        ] {
            let capabilities = if runtime_capability {
                serde_json::json!(["runtime"])
            } else {
                serde_json::json!([])
            };
            std::fs::write(package.join("mod.json"), serde_json::to_vec(&serde_json::json!({
                "id": if first_person_package { "shroudweave-first-person" } else { "unrelated-runtime-mod" },
                "name": "First Person", "version": "1.0.0",
                "capabilities": capabilities
            })).unwrap()).unwrap();
            std::fs::write(
                package.join("extended.mod.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schemaVersion": 1, "enabled": enabled,
                    "targets": if client { vec!["client"] } else { vec!["server"] }
                }))
                .unwrap(),
            )
            .unwrap();
            let environment =
                ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap()).unwrap();
            assert_eq!(
                first_person_startup_demand(&environment).unwrap(),
                expected,
                "enabled={enabled} runtime_capability={runtime_capability} client={client} first_person_package={first_person_package}"
            );
        }
    }

    #[test]
    fn vein_mining_demand_requires_enabled_server_runtime_package_and_valid_dependencies() {
        let root = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("vein-mining-boot-{}", uuid::Uuid::new_v4()));
        let package = root.join("mods/sf-vein-mining");
        std::fs::create_dir_all(package.join("src")).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
        std::fs::write(package.join("src/mod.lua"), "return {}\n").unwrap();

        let environment =
            ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap()).unwrap();
        assert!(
            !vein_mining_startup_demand(&environment).unwrap(),
            "missing package is no demand"
        );

        std::fs::write(
            package.join("mod.json"),
            serde_json::to_vec(&serde_json::json!({
                "id": "sf-vein-mining",
                "name": "Shroudweave Vein Mining",
                "version": "0.1.0",
                "capabilities": ["runtime"]
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(package.join("extended.mod.json"), b"{").unwrap();
        let malformed_environment =
            ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap()).unwrap();
        assert!(
            malformed_environment
                .mod_registry()
                .get("sf-vein-mining")
                .is_none()
        );
        assert!(
            vein_mining_startup_demand(&malformed_environment)
                .unwrap_err()
                .contains("rejected by the canonical package loader")
        );

        for (enabled, target, runtime, missing_dependency, expected, error_text) in [
            (false, "server", true, false, Some(false), None),
            (true, "server", true, false, Some(true), None),
            (true, "client", true, false, Some(false), None),
            (
                true,
                "server",
                false,
                false,
                None,
                Some("runtime capability"),
            ),
            (true, "server", true, true, None, Some("missing")),
        ] {
            let dependencies = if missing_dependency {
                serde_json::json!([{ "id": "missing-vein-test-dependency", "version": "^1.0.0" }])
            } else {
                serde_json::json!([])
            };
            let capabilities = if runtime {
                serde_json::json!(["runtime"])
            } else {
                serde_json::json!([])
            };
            std::fs::write(
                package.join("mod.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id": "sf-vein-mining",
                    "name": "Shroudweave Vein Mining",
                    "version": "0.1.0",
                    "dependencies": dependencies,
                    "capabilities": capabilities,
                }))
                .unwrap(),
            )
            .unwrap();
            std::fs::write(
                package.join("extended.mod.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schemaVersion": 1,
                    "enabled": enabled,
                    "targets": [target],
                }))
                .unwrap(),
            )
            .unwrap();

            let environment =
                ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap()).unwrap();
            match (
                vein_mining_startup_demand(&environment),
                expected,
                error_text,
            ) {
                (Ok(actual), Some(expected), None) => assert_eq!(actual, expected),
                (Err(error), None, Some(expected)) => {
                    assert!(error.contains(expected), "unexpected demand error: {error}")
                }
                (actual, expected, error_text) => panic!(
                    "unexpected demand result: {actual:?}, expected={expected:?}, error_text={error_text:?}"
                ),
            }
        }
    }

    #[test]
    fn gifting_range_demand_is_explicit_opt_in_server_only_and_runtime_capable() {
        let root = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("gifting-range-boot-{}", uuid::Uuid::new_v4()));
        let package = root.join("mods/shroudweave-unlimited-gifting-range-server");
        std::fs::create_dir_all(package.join("src")).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
        std::fs::write(package.join("src/mod.lua"), "return {}\n").unwrap();

        let environment =
            ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap()).unwrap();
        assert!(
            !gifting_range_startup_demand(&environment).unwrap(),
            "missing package is no demand"
        );

        std::fs::write(
            package.join("mod.json"),
            serde_json::to_vec(&serde_json::json!({
                "id": "shroudweave-unlimited-gifting-range-server",
                "name": "Shroudweave Unlimited Gifting Range Server",
                "version": "0.1.0",
                "capabilities": ["runtime"]
            }))
            .unwrap(),
        )
        .unwrap();
        for (enabled, target, runtime, expected, error_text) in [
            (false, "server", true, Some(false), None),
            (true, "server", true, Some(true), None),
            (true, "client", true, Some(false), None),
            (true, "server", false, None, Some("runtime capability")),
        ] {
            let capabilities = if runtime {
                serde_json::json!(["runtime"])
            } else {
                serde_json::json!([])
            };
            std::fs::write(
                package.join("mod.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id": "shroudweave-unlimited-gifting-range-server",
                    "name": "Shroudweave Unlimited Gifting Range Server",
                    "version": "0.1.0",
                    "capabilities": capabilities
                }))
                .unwrap(),
            )
            .unwrap();
            std::fs::write(
                package.join("extended.mod.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schemaVersion": 1,
                    "enabled": enabled,
                    "targets": [target]
                }))
                .unwrap(),
            )
            .unwrap();

            let environment =
                ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap()).unwrap();
            match (
                gifting_range_startup_demand(&environment),
                expected,
                error_text,
            ) {
                (Ok(actual), Some(expected), None) => assert_eq!(actual, expected),
                (Err(error), None, Some(expected)) => {
                    assert!(error.contains(expected), "unexpected demand error: {error}")
                }
                (actual, expected, error_text) => panic!(
                    "unexpected gifting demand result: {actual:?}, expected={expected:?}, error_text={error_text:?}"
                ),
            }
        }
    }

    #[test]
    fn startup_overlay_demand_uses_actual_enabled_capable_client_plan() {
        let root = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("overlay-boot-{}", uuid::Uuid::new_v4()));
        let package = root.join("mods/overlay-fixture");
        std::fs::create_dir_all(package.join("src")).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
        std::fs::write(package.join("x.dll"), b"test-only inert fixture").unwrap();
        std::fs::write(package.join("src/mod.lua"), "return {}").unwrap();
        for (enabled, capable, client, ini, expected) in [
            (
                true,
                true,
                true,
                "Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",
                true,
            ),
            (
                false,
                true,
                true,
                "Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",
                false,
            ),
            (
                true,
                false,
                true,
                "Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",
                false,
            ),
            (
                true,
                true,
                false,
                "Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",
                false,
            ),
            (
                true,
                true,
                true,
                "Enabled=no\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",
                false,
            ),
            (
                true,
                true,
                true,
                "Enabled=yes\nDll=x.dll\nServices=shroudforge.overlay",
                false,
            ),
            (
                true,
                true,
                true,
                "Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1",
                false,
            ),
        ] {
            let caps = if capable {
                serde_json::json!(["runtime", "runtime-register-dll"])
            } else {
                serde_json::json!(["runtime"])
            };
            std::fs::write(package.join("mod.json"),serde_json::to_vec(&serde_json::json!({"id":"overlay-fixture","name":"Overlay fixture","version":"1.0.0","capabilities":caps})).unwrap()).unwrap();
            std::fs::write(package.join("extended.mod.json"),serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"enabled":enabled,"targets":if client{vec!["client"]}else{vec!["server"]}})).unwrap()).unwrap();
            std::fs::write(
                package.join("native-plugin.ini"),
                format!("[Plugin]\n{ini}\n"),
            )
            .unwrap();
            let environment =
                mod_loader::ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap())
                    .unwrap();
            assert_eq!(
                super::native_overlay_startup_demand(&environment).unwrap(),
                expected,
                "enabled={enabled} capable={capable} client={client} ini={ini}"
            );
        }
        std::fs::write(package.join("native-plugin.ini"), "[Plugin]\nEnabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay").unwrap();
        let environment =
            mod_loader::ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap()).unwrap();
        assert!(
            super::managed_native_path(&root, "overlay-fixture", &package.join("x.dll")).unwrap()
        );
        std::fs::write(package.join("other.dll"), b"test-only inert fixture").unwrap();
        assert!(
            !super::managed_native_path(&root, "overlay-fixture", &package.join("other.dll"))
                .unwrap()
        );
        crate::native_dlls::queue(
            &root,
            mod_loader::dll_manager::DllLoadEntry {
                status_id: "fixture-lifecycle-route".into(),
                mod_id: Some("overlay-fixture".into()),
                package_directory: "overlay-fixture".into(),
                path: package.join("x.dll"),
                mode: mod_loader::dll_manager::DllLoadMode::Library,
                source: "fixture".into(),
            },
        )
        .unwrap();
        assert!(
            !crate::native_dlls::statuses()
                .iter()
                .any(|s| s.id == "fixture-lifecycle-route"),
            "Lifecycle DLL must not also enter the process loader"
        );
        let mut config = shroudforge_loader_config::config::read_loader(&root).unwrap();
        config["modules"]["dllManager"]["enabled"] = false.into();
        std::fs::write(
            shroudforge_loader_config::paths::loader_config(&root),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        assert!(
            !super::native_overlay_startup_demand(&environment).unwrap(),
            "DLL Manager disable must prevent early overlay hooks"
        );
    }

    #[test]
    fn overlay_boot_demand_is_explicit_and_section_scoped() {
        let plain =
            parse_native_plugin_ini("[Plugin]\nEnabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1")
                .unwrap();
        assert!(!plain.overlay);
        let opted = parse_native_plugin_ini("[Plugin]\nEnabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.map, shroudforge.overlay").unwrap();
        assert!(opted.overlay && opted.enabled && opted.mode == NativePluginMode::ShroudForgeV1);
        assert!(
            !parse_native_plugin_ini("[Other]\nServices=shroudforge.overlay")
                .unwrap()
                .overlay
        );
    }

    #[test]
    fn missing_lifecycle_keeps_legacy_load_only_mode() {
        let declaration =
            parse_native_plugin_ini("[Plugin]\nEnabled=true\nDll=plugin.dll\n").unwrap();
        assert!(declaration.enabled);
        assert_eq!(declaration.dll.as_deref(), Some("plugin.dll"));
        assert_eq!(declaration.mode, NativePluginMode::LegacyLoadOnly);
    }

    #[test]
    fn explicit_shroudforge_lifecycle_is_case_insensitive() {
        let declaration = parse_native_plugin_ini(
            "[Plugin]\nEnabled=yes\nDll=plugin.dll\nLifecycle=SHROUDFORGE-V1\n",
        )
        .unwrap();
        assert_eq!(declaration.mode, NativePluginMode::ShroudForgeV1);
    }

    #[test]
    fn unknown_declared_lifecycle_is_rejected() {
        let error = parse_native_plugin_ini(
            "[Plugin]\nEnabled=true\nDll=plugin.dll\nLifecycle=shroudforge-v2\n",
        )
        .unwrap_err();
        assert!(error.contains("unsupported native plugin lifecycle"));
    }
    #[test]
    fn preload_lifetime_unknown_or_legacy_request_fails_closed() {
        for config in [
            "[Plugin]\nLifetime=unknown\nLifecycle=shroudforge-v1",
            "[Plugin]\nLifetime=host-preload-v1",
        ] {
            assert!(parse_native_plugin_ini(config).is_err());
        }
    }
    #[test]
    fn preload_lifetime_explicit_policy_preserves_absent_defaults() {
        assert!(
            !parse_native_plugin_ini("[Plugin]\nEnabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1")
                .unwrap()
                .preload_host
        );
        assert!(parse_native_plugin_ini("[Plugin]\nEnabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nLifetime=host-preload-v1").unwrap().preload_host);
        assert!(
            !parse_native_plugin_ini("[Plugin]\nEnabled=yes\nDll=x.dll")
                .unwrap()
                .preload_host
        );
        for config in [
            "[Plugin]\nLifecycle=shroudforge-v1\nLifetime=",
            "[Plugin]\nLifecycle=shroudforge-v1\nLifetime=host-preload-v1\nLifetime=host-preload-v1",
        ] {
            assert!(parse_native_plugin_ini(config).is_err());
        }
    }

    #[test]
    fn opt_in_package_requires_runtime_feature_capability_and_matching_target() {
        assert!(
            validate_native_plugin_access(false, true, true, "client")
                .unwrap_err()
                .contains("unavailable")
        );
        assert!(
            validate_native_plugin_access(true, false, true, "client")
                .unwrap_err()
                .contains("runtime-register-dll")
        );
        assert!(
            validate_native_plugin_access(true, true, false, "server")
                .unwrap_err()
                .contains("server")
        );
        assert!(validate_native_plugin_access(true, true, true, "client").is_ok());
    }
}

/// Match only the declared lifecycle DLL, never another DLL in the same package.
pub(crate) fn managed_native_path(
    root: &std::path::Path,
    directory: &str,
    path: &std::path::Path,
) -> Result<bool, String> {
    let package = shroudforge_loader_config::paths::mods_dir(root).join(directory);
    let ini = package.join("native-plugin.ini");
    if !ini.is_file() {
        return Ok(false);
    }
    let declaration =
        parse_native_plugin_ini(&std::fs::read_to_string(ini).map_err(|e| e.to_string())?)?;
    if declaration.mode != NativePluginMode::ShroudForgeV1 {
        return Ok(false);
    }
    let Some(dll) = declaration.dll else {
        return Err("Lifecycle plugin must name a DLL".into());
    };
    let declared = mod_loader::dll_manager::resolve_package_dll_path(
        root,
        directory,
        &mod_loader::dll_manager::DllProfileFile {
            path: dll,
            mode: mod_loader::dll_manager::DllLoadMode::Library,
        },
    )?;
    Ok(declared.canonicalize().map_err(|e| e.to_string())?
        == path.canonicalize().map_err(|e| e.to_string())?)
}
