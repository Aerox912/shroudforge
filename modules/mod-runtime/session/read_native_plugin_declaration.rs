use super::{native_plugin_abi::NativePluginMode, *};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativePluginDeclaration {
    pub(super) enabled: bool,
    pub(super) dll: Option<String>,
    pub(super) mode: NativePluginMode,
    pub(super) overlay: bool,
}

pub(super) fn parse_native_plugin_ini(config: &str) -> Result<NativePluginDeclaration, String> {
    let mut in_plugin_section = false;
    let mut enabled = false;
    let mut dll = None;
    let mut lifecycle = None;
    let mut overlay = false;
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
            "services" => overlay = value.split(',').any(|name| name.trim() == "shroudforge.overlay"),
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
    Ok(NativePluginDeclaration { enabled, dll, mode, overlay })
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
    for package in environment.plan(false, crate::API_VERSION) {
        if !package.info().enabled || !package.info().target.supports_process(false)
            || !package.info().capabilities.contains(&Capability::RuntimeRegisterDll) { continue; }
        if let Some(declaration) = read_native_plugin_declaration(package)? {
            if declaration.enabled && declaration.overlay
                && declaration.mode == NativePluginMode::ShroudForgeV1
                && declaration.dll.as_ref().is_some_and(|dll| !dll.is_empty()) { return Ok(true); }
        }
    }
    Ok(false)
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
            NativePluginMode::LegacyLoadOnly => {
                if !self.has_feature(AppFeatures::RUNTIME_DLL)
                    || !target_mod
                        .info()
                        .capabilities
                        .contains(&Capability::RuntimeRegisterDll)
                    || !target_mod.info().target.supports_process(self.is_server())
                {
                    return Ok(false);
                }
            }
            NativePluginMode::ShroudForgeV1 => {
                self.validate_native_plugin_package(target_mod)?;
            }
        }
        tracing::debug!(target: "shroudforge::runtime", mod_id = %target_mod.info().id,
            dll = %relative_path, lifecycle = ?declaration.mode,
            "Native DLL declaration parsed. Queuing isolated load");
        self.queue_mod_native_dll(target_mod, &relative_path, declaration.mode)?;
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
        self.queue_mod_native_dll(target_mod, &relative_path, NativePluginMode::ShroudForgeV1)?;
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
                );
            }
        }
        self.queue_mod_native_dll(target_mod, relative_path, NativePluginMode::LegacyLoadOnly)
    }
}

#[cfg(test)]
mod tests {
    use super::native_plugin_abi::NativePluginMode;
    use super::{parse_native_plugin_ini, validate_native_plugin_access};

    #[test]
    fn startup_overlay_demand_uses_actual_enabled_capable_client_plan() {
        let root = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir).join(format!("overlay-boot-{}",uuid::Uuid::new_v4()));
        let package=root.join("mods/overlay-fixture");std::fs::create_dir_all(package.join("src")).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
        let _cleanup=Cleanup(root.clone());
        shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
        std::fs::write(package.join("src/mod.lua"),"return {}").unwrap();
        for (enabled,capable,client,ini,expected) in [
            (true,true,true,"Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",true),
            (false,true,true,"Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",false),
            (true,false,true,"Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",false),
            (true,true,false,"Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",false),
            (true,true,true,"Enabled=no\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.overlay",false),
            (true,true,true,"Enabled=yes\nDll=x.dll\nServices=shroudforge.overlay",false),
            (true,true,true,"Enabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1",false),
        ] {
            let caps=if capable {serde_json::json!(["runtime","runtime-register-dll"])}else{serde_json::json!(["runtime"])};
            std::fs::write(package.join("mod.json"),serde_json::to_vec(&serde_json::json!({"id":"overlay-fixture","name":"Overlay fixture","version":"1.0.0","capabilities":caps})).unwrap()).unwrap();
            std::fs::write(package.join("extended.mod.json"),serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"enabled":enabled,"targets":if client{vec!["client"]}else{vec!["server"]}})).unwrap()).unwrap();
            std::fs::write(package.join("native-plugin.ini"),format!("[Plugin]\n{ini}\n")).unwrap();
            let environment=mod_loader::ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap()).unwrap();
            assert_eq!(super::native_overlay_startup_demand(&environment).unwrap(),expected,"enabled={enabled} capable={capable} client={client} ini={ini}");
        }
    }

    #[test]
    fn overlay_boot_demand_is_explicit_and_section_scoped() {
        let plain = parse_native_plugin_ini("[Plugin]\nEnabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1").unwrap();
        assert!(!plain.overlay);
        let opted = parse_native_plugin_ini("[Plugin]\nEnabled=yes\nDll=x.dll\nLifecycle=shroudforge-v1\nServices=shroudforge.map, shroudforge.overlay").unwrap();
        assert!(opted.overlay && opted.enabled && opted.mode == NativePluginMode::ShroudForgeV1);
        assert!(!parse_native_plugin_ini("[Other]\nServices=shroudforge.overlay").unwrap().overlay);
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
