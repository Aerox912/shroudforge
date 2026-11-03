use super::*;
use crate::session::native_plugin_abi::process_lifetime;
#[cfg(windows)]
pub(crate) mod early_probe;
pub(crate) mod preload_artifact;
use crate::session::native_plugin_abi::{
    NativeNetworkPolicy, NativePluginEntry, NativePluginHostMetadata, NativePluginInstance,
    NativePluginMode,
};
use std::cell::Cell;

fn update_network_policy_snapshot(
    snapshot: &Cell<NativeNetworkPolicy>,
    config: Option<&serde_json::Value>,
) {
    snapshot.set(NativeNetworkPolicy::from_loader_config(config));
}

impl AppState {
    /// Install the runtime-owned service gate before opted-in plugins are
    /// initialized. Render callbacks must use this gate without touching Lua
    /// or the AppState RefCell.
    pub(crate) fn set_native_plugin_service_lifetime(
        &self,
        services: std::sync::Arc<
            dyn crate::session::native_plugin_abi::NativePluginServiceLifetime,
        >,
    ) -> Result<(), String> {
        if self
            .native_dlls
            .borrow()
            .iter()
            .any(NativeDll::is_lifecycle_managed)
            || self
                .pending_native_dlls
                .borrow()
                .iter()
                .any(|item| item.mode == NativePluginMode::ShroudForgeV1)
        {
            return Err(
                "native plugin service lifetime must be installed before plugin initialization"
                    .into(),
            );
        }
        *self.native_plugin_services.borrow_mut() = services;
        Ok(())
    }

    pub(super) fn managed_native_dll_state(&self, id: &str) -> Option<&'static str> {
        if self
            .native_dlls
            .borrow()
            .iter()
            .any(|dll| dll.mod_id == id && dll.is_retained())
        {
            Some("retained")
        } else if self.native_dlls.borrow().iter().any(|dll| dll.mod_id == id) {
            Some("loaded")
        } else if self
            .pending_native_dlls
            .borrow()
            .iter()
            .any(|dll| dll.mod_id == id)
        {
            Some("loading")
        } else {
            process_lifetime::preload_is_sticky(id).then_some("retained")
        }
    }

    pub(super) fn queue_mod_native_dll(
        &self,
        target_mod: &Mod,
        relative_path: &str,
        mode: NativePluginMode,
        preload_host: bool,
    ) -> Result<(), String> {
        if !self.has_feature(AppFeatures::RUNTIME_DLL) {
            return Err("native DLL loading is unavailable in this execution phase".into());
        }
        let package_directory = target_mod
            .fs()
            .root()
            .file_name()
            .unwrap_or_default()
            .to_owned();
        let mod_id = target_mod.info().id.to_string();
        crate::native_dlls::reject_reserved_greeneye_plugin_registration(
            self.env.game_dir().as_std_path(),
            &mod_id,
            &package_directory,
            std::path::Path::new(""),
        )?;
        if mode == NativePluginMode::ShroudForgeV1 {
            let root = self.env.game_dir();
            mod_loader::package_lookup::validate_mod_package_binding(
                root.as_std_path(),
                target_mod.package_binding(),
            )
            .map_err(|error| format!("native plugin package is no longer admitted: {error}"))?;
            let directory = package_directory.clone();
            let path = mod_loader::dll_manager::resolve_package_dll_path(
                root.as_std_path(),
                &directory,
                &mod_loader::dll_manager::DllProfileFile {
                    path: relative_path.to_owned(),
                    mode: mod_loader::dll_manager::DllLoadMode::Library,
                },
            )?;
            let path = path.canonicalize().map_err(|e| e.to_string())?;
            let allowed =
                mod_loader::dll_manager::resolve_load_plan(root.as_std_path(), self.is_server())
                    .entries
                    .into_iter()
                    .any(|entry| {
                        entry.mode == mod_loader::dll_manager::DllLoadMode::Library
                            && entry.path.canonicalize().ok().as_ref() == Some(&path)
                    });
            if !allowed {
                return Err("Lifecycle plugin is disabled, filtered, unapproved, or targets another process in DLL Manager".into());
            }
        }
        let package_path = camino::Utf8Path::new(relative_path);
        if package_path.as_str().is_empty()
            || package_path.is_absolute()
            || relative_path.contains(':')
            || package_path.components().any(|part| part.as_str() == "..")
        {
            return Err("DLL path must stay inside the mod package".into());
        }
        let mut filesystem = target_mod.fs();
        let (load_path, temporary_directory) = if let Some(path) = filesystem
            .absolute_path(package_path)
            .map_err(|error| error.to_string())?
        {
            if !path.is_file() {
                return Err(format!("registered DLL does not exist: {path}"));
            }
            (path.into_std_path_buf(), None)
        } else {
            let directory = std::env::temp_dir()
                .join("shroudforge")
                .join("native-mods")
                .join(target_mod.info().id.as_str())
                .join(uuid::Uuid::new_v4().to_string());
            std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let extraction = (|| {
                for file in filesystem.files() {
                    let file = file.map_err(|error| error.to_string())?;
                    if file.is_absolute() || file.components().any(|part| part.as_str() == "..") {
                        continue;
                    }
                    let destination = directory.join(file.as_std_path());
                    if let Some(parent) = destination.parent() {
                        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                    }
                    let mut reader = filesystem
                        .read_file(&file)
                        .map_err(|error| error.to_string())?;
                    let mut output =
                        std::fs::File::create(&destination).map_err(|error| error.to_string())?;
                    std::io::copy(&mut reader, &mut output).map_err(|error| error.to_string())?;
                }
                let destination = directory.join(package_path.as_std_path());
                if !destination.is_file() {
                    return Err("registered DLL was not found in the mod archive".into());
                }
                Ok(destination)
            })();
            match extraction {
                Ok(path) => (path, Some(directory.clone())),
                Err(error) => {
                    let _ = std::fs::remove_dir_all(directory);
                    return Err(error);
                }
            }
        };
        let canonical = match load_path.canonicalize() {
            Ok(path) => path,
            Err(error) => {
                if let Some(directory) = temporary_directory {
                    let _ = std::fs::remove_dir_all(directory);
                }
                return Err(format!("could not resolve DLL path: {error}"));
            }
        };
        if let Err(error) = crate::native_dlls::reject_reserved_greeneye_plugin_registration(
            self.env.game_dir().as_std_path(),
            &mod_id,
            &package_directory,
            &canonical,
        ) {
            if let Some(directory) = temporary_directory.as_ref() {
                let _ = std::fs::remove_dir_all(directory);
            }
            return Err(error);
        }
        let is_duplicate = {
            let loaded = self.native_dlls.borrow();
            let pending = self.pending_native_dlls.borrow();
            loaded.iter().any(|dll| {
                is_duplicate_request(
                    mode,
                    &mod_id,
                    &canonical,
                    dll.mode(),
                    &dll.mod_id,
                    &dll.path,
                )
            }) || pending.iter().any(|dll| {
                is_duplicate_request(mode, &mod_id, &canonical, dll.mode, &dll.mod_id, &dll.path)
            })
        };
        if is_duplicate {
            if let Some(directory) = temporary_directory {
                let _ = std::fs::remove_dir_all(directory);
            }
            return if preload_host {
                Err("preload package already has a pending/active owner; restart required".into())
            } else {
                Ok(())
            };
        }

        let settings_json = serde_json::to_vec(&target_mod.info().setting_values)
            .map_err(|error| format!("could not encode native plugin settings: {error}"))?;
        if settings_json.len() > crate::session::native_plugin_abi::NATIVE_MAX_SETTINGS_JSON_BYTES {
            if let Some(directory) = temporary_directory {
                let _ = std::fs::remove_dir_all(directory);
            }
            return Err("native plugin settings JSON exceeds 65536 bytes".into());
        }
        let package_generation = match process_lifetime::next_generation() {
            Ok(generation) => generation,
            Err(error) => {
                if let Some(directory) = &temporary_directory {
                    let _ = std::fs::remove_dir_all(directory);
                }
                return Err(error);
            }
        };
        // Reserve before the worker can call LoadLibrary/DllMain. Keep this
        // lease through cancellation, asynchronous completion and the loaded
        // library lifetime, so another AppState cannot race retention.
        let admission = if mode == NativePluginMode::ShroudForgeV1 {
            let reserve = if preload_host {
                preload_artifact::PreloadArtifact::open(&canonical).and_then(|artifact| {
                    process_lifetime::reserve_preload(
                        &mod_id,
                        package_generation,
                        process_lifetime::PreloadIntent {
                            artifact,
                            package_binding: target_mod.package_binding().clone(),
                            relative_path: relative_path.to_owned(),
                            process_target: if self.is_server() { 2 } else { 1 },
                        },
                    )
                })
            } else {
                process_lifetime::reserve(&mod_id, package_generation)
            };
            match reserve {
                Ok(owner) => Some(owner),
                Err(error) => {
                    if let Some(directory) = &temporary_directory {
                        let _ = std::fs::remove_dir_all(directory);
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        let preload_pending = admission
            .as_ref()
            .filter(|o| o.is_preload())
            .map(|o| PendingPreload::new(o.clone()));
        tracing::debug!(target:"shroudforge::runtime",mod_id,dll=%canonical.display(),lifecycle=?mode,"Queuing native DLL load on isolated worker");
        let load = spawn_admitted_native_load(
            mod_id.clone(),
            canonical.clone(),
            temporary_directory.clone(),
            admission.clone(),
            || {},
        )?;
        let NativeLoadTask { receiver, worker } = load;
        drop(worker); // Preserve the existing detached isolated-worker behavior.
        self.pending_native_dlls
            .borrow_mut()
            .push(PendingNativeDll {
                mod_id,
                path: canonical,
                package_relative_path: relative_path.to_owned(),
                mode,
                package: target_mod.clone(),
                package_generation,
                settings_json,
                queued_at: std::time::Instant::now(),
                warned_stall: false,
                receiver,
                admission,
                preload_pending,
            });
        Ok(())
    }

    pub(crate) fn poll_native_dll_loads(&self) {
        self.report_process_native_dlls();
        let mut completed = Vec::new();
        {
            let mut pending = self.pending_native_dlls.borrow_mut();
            let mut index = 0;
            while index < pending.len() {
                match pending[index].receiver.try_recv() {
                    Ok(result) => completed.push((pending.remove(index), Some(result))),
                    Err(TryRecvError::Disconnected) => {
                        completed.push((pending.remove(index), None))
                    }
                    Err(TryRecvError::Empty) => {
                        if !pending[index].warned_stall
                            && pending[index].queued_at.elapsed()
                                >= std::time::Duration::from_secs(5)
                        {
                            tracing::warn!(target: "shroudforge::runtime", mod_id = %pending[index].mod_id,
                                dll = %pending[index].path.display(), elapsed_ms = pending[index].queued_at.elapsed().as_millis(),
                                "Native DLL load is still running on an isolated worker. Runtime updates continue");
                            pending[index].warned_stall = true;
                        }
                        index += 1;
                    }
                }
            }
        }
        for (item, result) in completed {
            let result = result
                .unwrap_or_else(|| Err("native DLL loader worker exited without a result".into()));
            match result {
                Ok(library) => self.finish_native_library_load(item, library),
                Err(error) => {
                    tracing::error!(target: "shroudforge::runtime", mod_id = %item.mod_id,
                        dll = %item.path.display(), "Native DLL load failed: {error}");
                    let state = if item
                        .admission
                        .as_ref()
                        .is_some_and(|o| o.is_preload() && o.is_retained())
                    {
                        "retained"
                    } else {
                        "error"
                    };
                    self.report_runtime_effect(&item.mod_id, state, &error);
                }
            }
        }
    }

    fn finish_native_library_load(&self, mut item: PendingNativeDll, mut library: NativeLibrary) {
        if item.mode == NativePluginMode::LegacyLoadOnly {
            self.native_dlls.borrow_mut().push(NativeDll {
                mod_id: item.mod_id.clone(),
                path: item.path.clone(),
                library,
                module: NativeModule::LegacyLoadOnly,
            });
            tracing::info!(target: "shroudforge::runtime", mod_id = %item.mod_id,
                dll = %item.path.display(), "Legacy native DLL load completed without lifecycle initialization");
            self.report_runtime_effect(
                &item.mod_id,
                "loaded",
                "legacy native DLL loaded; lifecycle callbacks were not negotiated",
            );
            return;
        }

        let current = library
            .admission
            .as_ref()
            .ok_or_else(|| "native lifecycle load has no process admission".to_owned())
            .and_then(|owner| {
                if item.admission.as_ref().is_some_and(|pending| {
                    pending.root_cookie() == owner.root_cookie()
                        && std::sync::Arc::ptr_eq(pending, owner)
                }) {
                    Ok(owner)
                } else {
                    Err("native completion belongs to another root".into())
                }
            })
            .and_then(|owner| owner.validate(&item.mod_id, item.package_generation))
            .and_then(|()| self.pending_plugin_is_current(&item));
        match current {
            Ok(true) => {}
            Ok(false) => {
                if let Err(error) = self.release_uninitialized_library(library) {
                    self.report_runtime_effect(
                        &item.mod_id,
                        "retained",
                        &format!("stale native plugin load could not be released: {error}"),
                    );
                } else {
                    self.report_runtime_effect(
                        &item.mod_id,
                        "stopped",
                        "native plugin was disabled or changed before load completion",
                    );
                }
                return;
            }
            Err(error) => {
                if let Err(unload_error) = self.release_uninitialized_library(library) {
                    self.report_runtime_effect(
                        &item.mod_id,
                        "retained",
                        &format!("{error}; {unload_error}"),
                    );
                } else {
                    self.report_runtime_effect(&item.mod_id, "error", &error);
                }
                return;
            }
        }

        let entry = match library.entrypoint() {
            Ok(Some(entry)) => entry,
            Ok(None) => {
                let error = "opt-in native plugin is missing ShroudForgePluginEntry";
                tracing::error!(target: "shroudforge::runtime", mod_id = %item.mod_id, "{error}");
                match self.release_uninitialized_library(library) {
                    Ok(()) => self.report_runtime_effect(&item.mod_id, "error", error),
                    Err(unload_error) => self.report_runtime_effect(
                        &item.mod_id,
                        "retained",
                        &format!("{error}; {unload_error}"),
                    ),
                }
                return;
            }
            Err(error) => {
                match self.release_uninitialized_library(library) {
                    Ok(()) => self.report_runtime_effect(&item.mod_id, "error", &error),
                    Err(unload_error) => self.report_runtime_effect(
                        &item.mod_id,
                        "retained",
                        &format!("{error}; {unload_error}"),
                    ),
                }
                return;
            }
        };
        let services = self.native_plugin_services.borrow().clone();
        let preload_owner = library
            .admission
            .as_ref()
            .filter(|o| o.is_preload())
            .cloned();
        if let Some(pending) = item.preload_pending.as_mut() {
            pending.armed = false;
        }
        let package_binding = item.package.package_binding().clone();
        let metadata = NativePluginHostMetadata {
            package_id: item.mod_id.clone(),
            package: item.package,
            package_binding,
            package_generation: item.package_generation,
            is_server: self.is_server(),
            installation_root: self.env.game_dir().as_std_path().to_path_buf(),
            settings_json: item.settings_json,
            services,
        };
        let loaded = if let Some(owner) = preload_owner {
            NativePluginInstance::load_preloaded(entry, metadata, owner, library.handle as usize)
        } else {
            NativePluginInstance::load(entry, metadata)
        };
        let mut instance = match loaded {
            Ok(instance) => instance,
            Err(error) => {
                tracing::error!(target: "shroudforge::runtime", mod_id = %item.mod_id, "Native plugin ABI negotiation failed: {error}");
                match self.release_uninitialized_library(library) {
                    Ok(()) => self.report_runtime_effect(&item.mod_id, "error", &error),
                    Err(unload_error) => self.report_runtime_effect(
                        &item.mod_id,
                        "retained",
                        &format!("{error}; {unload_error}"),
                    ),
                }
                return;
            }
        };

        match instance.start() {
            Ok(()) => {
                self.native_dlls.borrow_mut().push(NativeDll {
                    mod_id: item.mod_id.clone(),
                    path: item.path.clone(),
                    library,
                    module: NativeModule::Lifecycle(Box::new(instance)),
                });
                tracing::info!(target: "shroudforge::runtime", mod_id = %item.mod_id,
                    dll = %item.path.display(), "ShroudForge native plugin initialized on the runtime owner thread");
                self.report_runtime_effect(
                    &item.mod_id,
                    "loaded",
                    "ShroudForge native plugin lifecycle is active",
                );
            }
            Err(failure) => {
                let detail = format!(
                    "native plugin initialization failed with status {}; {}",
                    failure.status,
                    failure.cleanup.description()
                );
                match failure.cleanup {
                    crate::session::native_plugin_lifecycle::TeardownOutcome::Quiescent => {
                        if let Err(error) = library.try_free() {
                            self.retain_native_module(
                                item.mod_id.clone(),
                                item.path.clone(),
                                library,
                                instance,
                                "library unload",
                                crate::session::native_plugin_abi::NATIVE_STATUS_FAILED,
                            );
                            self.report_runtime_effect(
                                &item.mod_id,
                                "retained",
                                &format!("{detail}; {error}"),
                            );
                            return;
                        }
                        drop(instance);
                        self.report_runtime_effect(&item.mod_id, "error", &detail);
                    }
                    crate::session::native_plugin_lifecycle::TeardownOutcome::Retain {
                        stage,
                        status,
                    } => {
                        self.retain_native_module(
                            item.mod_id.clone(),
                            item.path.clone(),
                            library,
                            instance,
                            stage,
                            status,
                        );
                        self.report_runtime_effect(&item.mod_id, "retained", &detail);
                    }
                }
                tracing::error!(target: "shroudforge::runtime", mod_id = %item.mod_id, "{detail}");
            }
        }
    }

    fn pending_plugin_is_current(&self, item: &PendingNativeDll) -> Result<bool, String> {
        if item.mode != NativePluginMode::ShroudForgeV1 {
            return Ok(true);
        }
        mod_loader::package_lookup::validate_mod_package_binding(
            self.env.game_dir().as_std_path(),
            item.package.package_binding(),
        )
        .map_err(|error| format!("native plugin package changed before initialization: {error}"))?;
        let package_root = item.package.fs().root().as_std_path().to_path_buf();
        let current_manifest = mod_loader::config::read_manifest_path(
            self.env.game_dir().as_std_path(),
            &package_root,
        )
        .map_err(|error| {
            format!("could not revalidate native plugin package before init: {error}")
        })?;
        if current_manifest.id != item.mod_id || !current_manifest.enabled {
            return Ok(false);
        }
        if !current_manifest
            .capabilities
            .contains(&Capability::RuntimeRegisterDll)
            || !current_manifest.target.supports_process(self.is_server())
        {
            return Ok(false);
        }
        let Some(declaration) =
            super::read_native_plugin_declaration::read_native_plugin_declaration(&item.package)?
        else {
            return Ok(false);
        };
        Ok(declaration.enabled
            && declaration.mode == NativePluginMode::ShroudForgeV1
            && declaration.dll.as_deref() == Some(item.package_relative_path.as_str()))
        .and_then(|current| {
            if let Some(owner) = item.admission.as_ref().filter(|o| o.is_preload()) {
                let approved = mod_loader::dll_manager::resolve_load_plan(
                    self.env.game_dir().as_std_path(),
                    self.is_server(),
                )
                .entries
                .into_iter()
                .any(|entry| {
                    entry.mode == mod_loader::dll_manager::DllLoadMode::Library
                        && entry.path.canonicalize().ok().as_ref() == Some(&item.path)
                });
                Ok(current
                    && approved
                    && declaration.preload_host
                    && owner.preload_target() == Some(if self.is_server() { 2 } else { 1 })
                    && owner.preload_package_matches(
                        item.package.package_binding(),
                        &item.package_relative_path,
                    ))
            } else {
                Ok(current && !declaration.preload_host)
            }
        })
    }

    fn release_uninitialized_library(&self, mut library: NativeLibrary) -> Result<(), String> {
        if let Err(error) = library.try_free() {
            tracing::error!(target: "shroudforge::runtime", "Could not unload uninitialized native DLL: {error}");
            std::mem::forget(library);
            Err(error)
        } else {
            Ok(())
        }
    }

    fn retain_native_module(
        &self,
        mod_id: String,
        path: NativePathBuf,
        library: NativeLibrary,
        instance: NativePluginInstance,
        stage: &'static str,
        status: i32,
    ) {
        self.native_dlls.borrow_mut().push(NativeDll {
            mod_id,
            path,
            library,
            module: NativeModule::Retained {
                instance: Some(Box::new(instance)),
                stage,
                status,
            },
        });
    }

    pub(crate) fn update_native_plugins(&self, delta_seconds: f64) {
        let elapsed_microseconds = if delta_seconds.is_finite() && delta_seconds > 0.0 {
            (delta_seconds * 1_000_000.0).min(u64::MAX as f64) as u64
        } else {
            0
        };
        let failed = {
            let mut dlls = self.native_dlls.borrow_mut();
            let mut failed = Vec::new();
            for dll in dlls.iter_mut() {
                if let NativeModule::Lifecycle(instance) = &mut dll.module {
                    if instance.update(elapsed_microseconds)
                        != crate::session::native_plugin_abi::NATIVE_STATUS_OK
                    {
                        failed.push(dll.mod_id.clone());
                    }
                }
            }
            failed
        };
        for mod_id in failed {
            let detail = "native plugin update failed; lifecycle teardown was requested";
            match self.stop_native_plugin(&mod_id) {
                Ok(_) => self.report_runtime_effect(&mod_id, "error", detail),
                Err(error) => {
                    self.report_runtime_effect(&mod_id, "retained", &format!("{detail}; {error}"))
                }
            }
        }
    }

    pub(crate) fn update_native_network_policy(&self, config: Option<&serde_json::Value>) {
        update_network_policy_snapshot(&self.network_policy, config);
        let mut dlls = self.native_dlls.borrow_mut();
        for dll in dlls.iter_mut() {
            let NativeModule::Lifecycle(instance) = &mut dll.module else {
                continue;
            };
            match instance.update_network_policy(config) {
                Ok(true) => tracing::debug!(target: "shroudforge::runtime", mod_id = %dll.mod_id,
                    "Native network permission policy refreshed"),
                Ok(false) => {}
                Err(error) => tracing::error!(target: "shroudforge::runtime", mod_id = %dll.mod_id,
                    "Native network permission policy update failed: {error}"),
            }
        }
    }

    pub(crate) fn update_native_plugin_settings(
        &self,
        mod_id: &str,
        setting_values: &serde_json::Value,
    ) -> Result<(), String> {
        let pending_index = self
            .pending_native_dlls
            .borrow()
            .iter()
            .position(|pending| {
                pending.mod_id == mod_id && pending.mode == NativePluginMode::ShroudForgeV1
            });
        let active =
            self.native_dlls.borrow().iter().any(|dll| {
                dll.mod_id == mod_id && matches!(&dll.module, NativeModule::Lifecycle(_))
            });
        if pending_index.is_none() && !active {
            return Ok(());
        }
        let settings_json = serde_json::to_vec(setting_values)
            .map_err(|error| format!("could not encode native plugin settings: {error}"))?;
        if settings_json.len() > crate::session::native_plugin_abi::NATIVE_MAX_SETTINGS_JSON_BYTES {
            return Err("native plugin settings JSON exceeds 65536 bytes".into());
        }
        if let Some(index) = pending_index {
            self.pending_native_dlls.borrow_mut()[index].settings_json = settings_json.clone();
        }
        let mut dlls = self.native_dlls.borrow_mut();
        let Some(dll) = dlls.iter_mut().find(|dll| dll.mod_id == mod_id) else {
            return Ok(());
        };
        match &mut dll.module {
            NativeModule::Lifecycle(instance) => instance.update_settings_json(settings_json),
            _ => Ok(()),
        }
    }

    /// Stop one opted-in plugin. Legacy DLLs remain load-only and are retained
    /// until their session is dropped.
    pub(crate) fn stop_native_plugin(&self, mod_id: &str) -> Result<bool, String> {
        let cancelled = {
            let mut pending = self.pending_native_dlls.borrow_mut();
            cancel_pending_plugin_load(&mut pending, mod_id)
        };

        let active_index = self
            .native_dlls
            .borrow()
            .iter()
            .position(|dll| dll.mod_id == mod_id && dll.is_lifecycle_managed());
        let Some(index) = active_index else {
            if cancelled && process_lifetime::preload_is_sticky(mod_id) {
                let error = "preload load cancelled; owner/mapping retained until process exit";
                self.report_runtime_effect(mod_id, "retained", error);
                return Err(error.into());
            }
            if cancelled {
                self.report_runtime_effect(
                    mod_id,
                    "stopped",
                    "pending native plugin load was cancelled",
                );
            }
            return Ok(cancelled);
        };

        let result = {
            let mut dlls = self.native_dlls.borrow_mut();
            let dll = &mut dlls[index];
            let previous = std::mem::replace(&mut dll.module, NativeModule::Transitioning);
            match previous {
                NativeModule::Lifecycle(mut instance) => match instance.stop() {
                    crate::session::native_plugin_lifecycle::TeardownOutcome::Quiescent => {
                        match dll.library.try_free() {
                            Ok(()) => {
                                drop(instance);
                                dlls.remove(index);
                                Ok(())
                            }
                            Err(error) => {
                                dll.module = NativeModule::Retained {
                                    instance: Some(instance),
                                    stage: "library unload",
                                    status: crate::session::native_plugin_abi::NATIVE_STATUS_FAILED,
                                };
                                Err(error)
                            }
                        }
                    }
                    crate::session::native_plugin_lifecycle::TeardownOutcome::Retain {
                        stage,
                        status,
                    } => {
                        dll.module = NativeModule::Retained {
                            instance: Some(instance),
                            stage,
                            status,
                        };
                        Err(format!(
                            "native plugin retained at {stage} with status {status}"
                        ))
                    }
                },
                NativeModule::Retained {
                    instance,
                    stage,
                    status,
                } => {
                    dll.module = NativeModule::Retained {
                        instance,
                        stage,
                        status,
                    };
                    Err(format!(
                        "native plugin remains retained at {stage} with status {status}"
                    ))
                }
                other => {
                    dll.module = other;
                    Ok(())
                }
            }
        };
        match result {
            Ok(()) => {
                self.report_runtime_effect(
                    mod_id,
                    "stopped",
                    "native plugin services drained and DLL unloaded",
                );
                Ok(true)
            }
            Err(error) => {
                self.report_runtime_effect(mod_id, "retained", &error);
                Err(error)
            }
        }
    }

    pub(crate) fn restart_native_plugins(&self, mod_ids: &[String]) -> Result<(), String> {
        for mod_id in mod_ids {
            self.restart_native_plugin(mod_id)?;
        }
        Ok(())
    }

    pub(crate) fn shutdown_native_dlls(&mut self) {
        self.pending_native_dlls.get_mut().clear();
        let dlls = self.native_dlls.get_mut();
        for mut dll in dlls.drain(..).rev() {
            let module = std::mem::replace(&mut dll.module, NativeModule::Transitioning);
            match module {
                NativeModule::Lifecycle(mut instance) => match instance.stop() {
                    crate::session::native_plugin_lifecycle::TeardownOutcome::Quiescent => {
                        if let Err(error) = dll.library.try_free() {
                            tracing::error!(target: "shroudforge::runtime", mod_id = %dll.mod_id,
                                    "Native plugin services drained but DLL could not be unloaded: {error}");
                            std::mem::forget((dll, instance));
                        }
                    }
                    crate::session::native_plugin_lifecycle::TeardownOutcome::Retain {
                        stage,
                        status,
                    } => {
                        tracing::error!(target: "shroudforge::runtime", mod_id = %dll.mod_id,
                                "Retaining native plugin through process exit: teardown stopped at {stage} with status {status}");
                        std::mem::forget((dll, instance));
                    }
                },
                NativeModule::Retained {
                    instance,
                    stage,
                    status,
                } => {
                    tracing::error!(target: "shroudforge::runtime", mod_id = %dll.mod_id,
                        "Retaining native plugin through process exit: teardown stopped at {stage} with status {status}");
                    std::mem::forget((dll, instance));
                }
                NativeModule::LegacyLoadOnly => {
                    if let Err(error) = dll.library.try_free() {
                        tracing::warn!(target: "shroudforge::runtime", mod_id = %dll.mod_id,
                            "Could not unload legacy native DLL at session shutdown: {error}");
                        std::mem::forget(dll);
                    }
                }
                NativeModule::Transitioning => {
                    tracing::error!(target: "shroudforge::runtime", mod_id = %dll.mod_id,
                        "Retaining native plugin in an incomplete lifecycle transition");
                    std::mem::forget(dll);
                }
            }
        }
    }
}

fn is_duplicate_request(
    mode: NativePluginMode,
    mod_id: &str,
    path: &std::path::Path,
    existing_mode: NativePluginMode,
    existing_mod_id: &str,
    existing_path: &std::path::Path,
) -> bool {
    if mode == NativePluginMode::ShroudForgeV1 && existing_mode == NativePluginMode::ShroudForgeV1 {
        mod_id == existing_mod_id
    } else {
        path == existing_path
    }
}

fn cancel_pending_plugin_load(pending: &mut Vec<PendingNativeDll>, mod_id: &str) -> bool {
    let mut cancelled = false;
    pending.retain(|item| {
        let remove = item.mod_id == mod_id && item.mode == NativePluginMode::ShroudForgeV1;
        cancelled |= remove;
        !remove
    });
    cancelled
}

pub(super) struct NativeDll {
    pub(super) mod_id: String,
    pub(super) path: NativePathBuf,
    library: NativeLibrary,
    module: NativeModule,
}

impl NativeDll {
    fn mode(&self) -> NativePluginMode {
        if self.is_lifecycle_managed() {
            NativePluginMode::ShroudForgeV1
        } else {
            NativePluginMode::LegacyLoadOnly
        }
    }

    fn is_lifecycle_managed(&self) -> bool {
        matches!(
            &self.module,
            NativeModule::Lifecycle(_) | NativeModule::Retained { .. }
        )
    }

    fn is_retained(&self) -> bool {
        matches!(&self.module, NativeModule::Retained { .. })
    }
}

enum NativeModule {
    LegacyLoadOnly,
    Lifecycle(Box<NativePluginInstance>),
    Retained {
        instance: Option<Box<NativePluginInstance>>,
        stage: &'static str,
        status: i32,
    },
    Transitioning,
}

pub(super) struct PendingNativeDll {
    pub(super) mod_id: String,
    pub(super) path: NativePathBuf,
    package_relative_path: String,
    mode: NativePluginMode,
    package: Mod,
    package_generation: u64,
    settings_json: Vec<u8>,
    queued_at: std::time::Instant,
    warned_stall: bool,
    receiver: Receiver<Result<NativeLibrary, String>>,
    admission: Option<std::sync::Arc<process_lifetime::Admission>>,
    preload_pending: Option<PendingPreload>,
}

pub(crate) struct PendingPreload {
    owner: std::sync::Arc<process_lifetime::Admission>,
    armed: bool,
}
impl PendingPreload {
    pub(crate) fn new(owner: std::sync::Arc<process_lifetime::Admission>) -> Self {
        Self { owner, armed: true }
    }
}
impl Drop for PendingPreload {
    fn drop(&mut self) {
        if self.armed {
            self.owner.cancel_preload();
        }
    }
}

pub(crate) struct NativeLibrary {
    pub(crate) handle: isize,
    temporary_directory: Option<NativePathBuf>,
    admission: Option<std::sync::Arc<process_lifetime::Admission>>,
}

impl NativeLibrary {
    fn new(handle: isize, temporary_directory: Option<NativePathBuf>) -> Self {
        Self {
            handle,
            temporary_directory,
            admission: None,
        }
    }

    pub(crate) fn entrypoint(&self) -> Result<Option<NativePluginEntry>, String> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
            let symbol = unsafe {
                GetProcAddress(
                    self.handle as windows_sys::Win32::Foundation::HMODULE,
                    b"ShroudForgePluginEntry\0".as_ptr(),
                )
            };
            Ok(symbol.map(|symbol| unsafe {
                std::mem::transmute::<unsafe extern "system" fn() -> isize, NativePluginEntry>(
                    symbol,
                )
            }))
        }
        #[cfg(not(windows))]
        {
            let _ = self;
            Err("native plugin lifecycle lookup is supported only on Windows".into())
        }
    }

    fn try_free(&mut self) -> Result<(), String> {
        if self
            .admission
            .as_ref()
            .is_some_and(|owner| owner.is_retained())
        {
            return Err("native plugin is process-retained; restart required".into());
        }
        if self.handle == 0 {
            self.remove_temporary_directory();
            return Ok(());
        }
        #[cfg(windows)]
        {
            let result = unsafe {
                windows_sys::Win32::Foundation::FreeLibrary(
                    self.handle as windows_sys::Win32::Foundation::HMODULE,
                )
            };
            if result == 0 {
                return Err(format!(
                    "FreeLibrary failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            self.handle = 0;
            self.remove_temporary_directory();
            Ok(())
        }
        #[cfg(not(windows))]
        {
            Err("native DLL unloading is supported only on Windows".into())
        }
    }

    fn remove_temporary_directory(&mut self) {
        if let Some(directory) = self.temporary_directory.take() {
            if let Err(error) = std::fs::remove_dir_all(&directory) {
                tracing::warn!(target: "shroudforge::runtime", %error, path = %directory.display(),
                    "Could not remove extracted native DLL files");
            }
        }
    }
}

impl Drop for NativeLibrary {
    fn drop(&mut self) {
        if self
            .admission
            .as_ref()
            .is_some_and(|owner| owner.is_retained())
        {
            // Preserve the mapped image and extracted files even on an
            // unexpected owner drop. Normal retention also keeps the full
            // plugin instance/host via NativeModule::Retained.
            return;
        }
        if self.handle != 0 {
            #[cfg(windows)]
            {
                let result = unsafe {
                    windows_sys::Win32::Foundation::FreeLibrary(
                        self.handle as windows_sys::Win32::Foundation::HMODULE,
                    )
                };
                if result == 0 {
                    if let Some(directory) = self.temporary_directory.take() {
                        // Keep extracted code available while Windows still maps it.
                        std::mem::forget(directory);
                    }
                    return;
                }
                self.handle = 0;
            }
            #[cfg(not(windows))]
            {
                return;
            }
        }
        self.remove_temporary_directory();
    }
}

pub(crate) fn load_admitted_native_library(
    mod_id: &str,
    path: &std::path::Path,
    temporary_directory: Option<NativePathBuf>,
    admission: Option<std::sync::Arc<process_lifetime::Admission>>,
) -> Result<NativeLibrary, String> {
    if let Some(owner) = admission.as_ref().filter(|o| o.is_preload()) {
        owner.claim_preload_worker()?;
    }
    let result = (|| {
        let mut library = load_native_library(mod_id, path, temporary_directory)?;
        if let Some(owner) = admission.as_ref() {
            // Attach before any fallible post-load check. A committed root owns this
            // returned mapping reference even when identity fails or receiver drops.
            library.admission = Some(owner.clone());
            if owner.is_preload() {
                owner.record_preload_mapping(library.handle as usize)?;
                owner.bind_preload_mapping(library.handle as usize)?;
            } else {
                owner.bind_module(library.handle as usize)?;
            }
        }
        Ok(library)
    })();
    if result.is_err() {
        if let Some(owner) = &admission {
            owner.fail_preload();
        }
    }
    result
}

/// The actual queue worker. The continuation runs after LoadLibrary and all
/// mapping checks, outside the Windows loader lock and before channel delivery.
/// Production supplies an inert continuation. Offline fixtures may synchronize
/// here to observe cancellation/send failure without blocking inside DllMain.
pub(crate) fn spawn_admitted_native_load(
    mod_id: String,
    path: NativePathBuf,
    temporary_directory: Option<NativePathBuf>,
    admission: Option<std::sync::Arc<process_lifetime::Admission>>,
    after_load: impl FnOnce() + Send + 'static,
) -> Result<NativeLoadTask, String> {
    let (sender, receiver) = mpsc::channel();
    let worker_owner = admission.clone();
    let worker_temp = temporary_directory.clone();
    let worker=std::thread::Builder::new().name(format!("sf-native-{}",mod_id.chars().take(24).collect::<String>())).spawn(move||{
        let result=load_admitted_native_library(&mod_id,&path,worker_temp.clone(),worker_owner.clone());
        if let Err(error)=&result {
            if let Some(directory)=worker_temp.filter(|_|!worker_owner.as_ref().is_some_and(|o|o.is_preload()&&o.is_retained())){let _=std::fs::remove_dir_all(directory);}
            tracing::error!(target:"shroudforge::runtime",mod_id=%mod_id,dll=%path.display(),"Isolated native DLL load failed: {error}");
        }
        after_load();
        let _=sender.send(result);
    }).map_err(|error|{
        if let Some(owner)=&admission{owner.fail_preload();}
        if let Some(directory)=temporary_directory.as_ref().filter(|_|!admission.as_ref().is_some_and(|o|o.is_preload()&&o.is_retained())){let _=std::fs::remove_dir_all(directory);}
        format!("could not start isolated DLL loader thread: {error}")
    })?;
    Ok(NativeLoadTask { receiver, worker })
}

pub(crate) struct NativeLoadTask {
    pub(crate) receiver: Receiver<Result<NativeLibrary, String>>,
    pub(crate) worker: std::thread::JoinHandle<()>,
}
impl std::ops::Deref for NativeLoadTask {
    type Target = Receiver<Result<NativeLibrary, String>>;
    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}

#[cfg(windows)]
fn load_native_library(
    mod_id: &str,
    path: &std::path::Path,
    temporary_directory: Option<NativePathBuf>,
) -> Result<NativeLibrary, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LoadLibraryExW,
    };

    tracing::debug!(target: "shroudforge::runtime", mod_id, dll = %path.display(),
        "Calling LoadLibraryExW on isolated worker");
    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        LoadLibraryExW(
            wide_path.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        )
    };
    if handle.is_null() {
        Err(format!(
            "Windows could not load native DLL for mod '{mod_id}': {}",
            std::io::Error::last_os_error()
        ))
    } else {
        tracing::debug!(target: "shroudforge::runtime", mod_id, dll = %path.display(),
            "LoadLibraryExW returned on isolated worker");
        Ok(NativeLibrary::new(handle as isize, temporary_directory))
    }
}

#[cfg(not(windows))]
fn load_native_library(
    mod_id: &str,
    _path: &std::path::Path,
    _temporary_directory: Option<NativePathBuf>,
) -> Result<NativeLibrary, String> {
    Err(format!(
        "native DLL loading for mod '{mod_id}' is supported only on Windows"
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        NativeLibrary, NativeNetworkPolicy, NativePluginMode, PendingNativeDll,
        cancel_pending_plugin_load, is_duplicate_request, update_network_policy_snapshot,
    };
    use std::{
        cell::Cell,
        fs,
        path::{Path, PathBuf},
        sync::mpsc,
        time::Instant,
    };

    #[test]
    fn network_policy_snapshot_refreshes_without_native_plugin_instances() {
        let snapshot = Cell::new(NativeNetworkPolicy::Unavailable);
        let disabled = serde_json::json!({"modules":{"network":{"enabled":false}}});
        let enabled = serde_json::json!({"modules":{"network":{"enabled":true}}});

        update_network_policy_snapshot(&snapshot, Some(&disabled));
        assert_eq!(snapshot.get(), NativeNetworkPolicy::Disabled);
        update_network_policy_snapshot(&snapshot, None);
        assert_eq!(snapshot.get(), NativeNetworkPolicy::Unavailable);
        update_network_policy_snapshot(&snapshot, Some(&enabled));
        assert_eq!(snapshot.get(), NativeNetworkPolicy::Enabled);
    }

    #[test]
    fn duplicate_opted_in_start_is_rejected_by_package_identity() {
        assert!(is_duplicate_request(
            NativePluginMode::ShroudForgeV1,
            "example",
            Path::new("one.dll"),
            NativePluginMode::ShroudForgeV1,
            "example",
            Path::new("other.dll"),
        ));
        assert!(is_duplicate_request(
            NativePluginMode::ShroudForgeV1,
            "example",
            Path::new("one.dll"),
            NativePluginMode::LegacyLoadOnly,
            "example",
            Path::new("one.dll"),
        ));
    }

    #[test]
    fn disable_cancels_completed_pending_load_before_owner_thread_initialization() {
        let root = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("native-plugin-cancel-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(NativeLibrary::new(0, Some(root.clone()))))
            .unwrap();
        let package_root = root.join("mods").join("example");
        fs::create_dir_all(&package_root).unwrap();
        fs::write(
            package_root.join("mod.json"),
            r#"{"id":"example","name":"Example","version":"1.0.0","capabilities":["runtime-register-dll"]}"#,
        )
        .unwrap();
        fs::write(
            package_root.join("extended.mod.json"),
            r#"{"schemaVersion":1,"enabled":true,"targets":["client"],"settings":{}}"#,
        )
        .unwrap();
        let environment =
            mod_loader::ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap()).unwrap();
        let package = environment.mod_registry().get("example").unwrap().clone();
        let (other_sender, other_receiver) = mpsc::channel();
        let mut pending = vec![
            PendingNativeDll {
                mod_id: "example".into(),
                path: PathBuf::from("example.dll"),
                package_relative_path: "example.dll".into(),
                mode: NativePluginMode::ShroudForgeV1,
                package,
                package_generation: 1,
                settings_json: b"{}".to_vec(),
                queued_at: Instant::now(),
                warned_stall: false,
                receiver,
                admission: None,
                preload_pending: None,
            },
            PendingNativeDll {
                mod_id: "example".into(),
                path: PathBuf::from("legacy.dll"),
                package_relative_path: "legacy.dll".into(),
                mode: NativePluginMode::LegacyLoadOnly,
                package: environment.mod_registry().get("example").unwrap().clone(),
                package_generation: 2,
                settings_json: b"{}".to_vec(),
                queued_at: Instant::now(),
                warned_stall: false,
                receiver: other_receiver,
                admission: None,
                preload_pending: None,
            },
        ];

        assert!(cancel_pending_plugin_load(&mut pending, "example"));
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].mode, NativePluginMode::LegacyLoadOnly);
        assert!(
            !root.exists(),
            "dropping the queued library cleans its extraction directory"
        );
        drop(other_sender);
        drop(pending);
        let _ = fs::remove_dir_all(root);
    }
}

impl AppState {
    pub(super) fn native_dll_state(&self, id: &str) -> Option<crate::native_dlls::NativeDllStatus> {
        if let Some(state) = self.managed_native_dll_state(id) {
            return Some(crate::native_dlls::NativeDllStatus {
                id: id.to_owned(),
                state,
                detail: "Lifecycle-managed native plugin".into(),
                source: "native-plugin.ini lifecycle".into(),
            });
        }
        crate::native_dlls::statuses()
            .into_iter()
            .find(|status| status.id == id)
    }

    /// Lua registrations retain their EML execution phase and package boundary.
    /// Loading and lifetime are shared with INI, profile and manual selections.
    pub(crate) fn load_process_native_dll(
        &self,
        target_mod: &Mod,
        relative_path: &str,
    ) -> Result<(), String> {
        if !self.has_feature(AppFeatures::RUNTIME_DLL) {
            return Err("native DLL loading is unavailable in this execution phase".into());
        }
        if !target_mod.info().enabled {
            self.report_runtime_effect(&target_mod.info().id, "restart-required",
                "Native DLL was skipped because the mod is disabled. Enable it and restart the game");
            return Ok(());
        }
        let directory = target_mod
            .fs()
            .root()
            .file_name()
            .unwrap_or_default()
            .to_owned();
        if mod_loader::dll_manager::read_selections(self.env.game_dir().as_std_path())
            .iter()
            .find(|selection| selection.package_directory.eq_ignore_ascii_case(&directory))
            .is_some_and(|selection| !selection.target.applies_to(self.is_server()))
        {
            return Err("The DLL Manager selection targets a different game process".into());
        }
        let relative_path = relative_path.replace('\\', "/");
        let relative_path = relative_path.trim_start_matches("./").to_owned();
        let path = mod_loader::dll_manager::resolve_package_dll_path(
            self.env.game_dir().as_std_path(),
            &directory,
            &mod_loader::dll_manager::DllProfileFile {
                path: relative_path.clone(),
                mode: mod_loader::dll_manager::DllLoadMode::Library,
            },
        )?;
        let package_directory = directory;
        let mod_id = target_mod.info().id.clone();
        let status_id = format!(
            "dll:{}:{}",
            package_directory.to_ascii_lowercase(),
            relative_path.to_ascii_lowercase()
        );
        crate::native_dlls::queue(
            self.env.game_dir().as_std_path(),
            mod_loader::dll_manager::DllLoadEntry {
                status_id,
                mod_id: Some(mod_id),
                package_directory,
                path,
                mode: mod_loader::dll_manager::DllLoadMode::Library,
                source: "Lua register_dll".into(),
            },
        )?;
        self.poll_native_dll_loads();
        Ok(())
    }

    pub(crate) fn report_process_native_dlls(&self) {
        for status in crate::native_dlls::statuses() {
            self.report_native_dll_effect(&status);
        }
    }

    pub(super) fn report_native_dll_effect(&self, status: &crate::native_dlls::NativeDllStatus) {
        self.report_runtime_effect(&status.id, status.state, &status.detail);
        if let Some(effect) = self.runtime_effects.borrow_mut().get_mut(&status.id) {
            effect["nativeSource"] = status.source.clone().into();
        }
    }
}
