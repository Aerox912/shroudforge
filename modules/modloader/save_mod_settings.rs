use crate::update_mod_configuration::{self, ModConfigurationError};
use std::path::Path;

/// Save a partial settings patch using the canonical installation-wide
/// configuration transaction.
pub fn save_mod_settings(
    root: &Path,
    id: &str,
    values: &serde_json::Value,
    revision: &str,
) -> Result<(), String> {
    save_mod_settings_with_status(root, id, values, revision).map_err(|error| error.to_string())
}

/// Save a partial settings patch while preserving a distinct stale-revision
/// result for native callers.
pub fn save_mod_settings_with_status(
    root: &Path,
    id: &str,
    values: &serde_json::Value,
    revision: &str,
) -> Result<(), ModConfigurationError> {
    update_mod_configuration::update_mod_configuration(root, id, revision, |manifest, extension| {
        apply_settings_patch(root, id, manifest, extension, values)
    })
}

/// Save a partial settings patch against the package identity pinned to a
/// native runtime owner.
pub fn save_bound_mod_settings_with_status(
    root: &Path,
    binding: &crate::package_lookup::ModPackageBinding,
    values: &serde_json::Value,
    revision: &str,
) -> Result<(), ModConfigurationError> {
    update_mod_configuration::update_bound_mod_configuration(
        root,
        binding,
        revision,
        |manifest, extension| apply_settings_patch(root, binding.id(), manifest, extension, values),
    )
}

fn apply_settings_patch(
    root: &Path,
    id: &str,
    manifest: &crate::ModManifest,
    extension: &mut serde_json::Value,
    values: &serde_json::Value,
) -> Result<(), String> {
    crate::config::validate_settings(&manifest.settings, values)?;
    let patch = values.as_object().ok_or("settings must be an object")?;
    let mut candidate = manifest.setting_values.clone();
    for (key, value) in patch {
        candidate[key] = value.clone();
    }
    let affects_keybind = manifest.settings.iter().any(|setting| {
        matches!(setting.control, Some(crate::SettingControl::Keybind))
            && (candidate.get(&setting.key) != manifest.setting_values.get(&setting.key)
                || crate::setting_is_enabled(&setting.key, &manifest.settings, &candidate)
                    != crate::setting_is_enabled(
                        &setting.key,
                        &manifest.settings,
                        &manifest.setting_values,
                    ))
    });
    if affects_keybind {
        let config = crate::config::read_loader(root)?;
        crate::validate_hotkey_configuration(root, &config, Some((id, values, None)))?;
    }

    let settings = extension
        .get_mut("settings")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("extended.mod.json settings must be an object")?;
    for (key, value) in patch {
        let current = settings
            .get_mut(key)
            .ok_or_else(|| format!("unknown setting '{key}'"))?;
        if current.is_object() {
            current["value"] = value.clone();
        } else {
            *current = value.clone();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModConfigurationError, read_mod_settings_snapshot};
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Child, Command},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    fn new_root(label: &str) -> PathBuf {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = target.join(format!("sf-native-settings-{label}-{suffix}"));
        fs::create_dir_all(&root).unwrap();
        shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
        root
    }

    fn write_mod(root: &Path, id: &str, binding: Option<&str>) {
        write_mod_with_enabled(root, id, binding, true);
    }

    fn write_mod_with_enabled(root: &Path, id: &str, binding: Option<&str>, enabled: bool) {
        let package = shroudforge_loader_config::paths::mods_dir(root).join(id);
        write_mod_at(&package, id, binding, enabled);
    }

    fn write_mod_at(package: &Path, id: &str, binding: Option<&str>, enabled: bool) {
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("mod.json"),
            serde_json::json!({"id":id,"name":id,"version":"1.0.0"}).to_string(),
        )
        .unwrap();
        let mut settings = serde_json::json!({
            "mode": {"value":"vein"},
            "activation_key": {"value":"SDL:70"},
            "toggle_migration_revision": {"value":1},
            "radius_key": {"value":"SDL:82"},
            "radius_multiplier": {"value":1.0}
        });
        if let Some(binding) = binding {
            settings["toggle"] = serde_json::json!({
                "value":binding,
                "control":"keybind",
                "applyAt":"live"
            });
        }
        fs::write(
            package.join("extended.mod.json"),
            serde_json::json!({
                "schemaVersion":1,
                "enabled":enabled,
                "launcher":"SF",
                "targets":["client"],
                "settings":settings
            })
            .to_string(),
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn create_directory_link(target: &Path, link: &Path) -> Result<(), String> {
        std::os::unix::fs::symlink(target, link).map_err(|error| error.to_string())
    }

    #[cfg(windows)]
    fn create_directory_link(target: &Path, link: &Path) -> Result<(), String> {
        let output = Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .map_err(|error| error.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    }

    #[cfg(unix)]
    fn create_file_link(target: &Path, link: &Path) -> Result<(), String> {
        std::os::unix::fs::symlink(target, link).map_err(|error| error.to_string())
    }

    #[cfg(windows)]
    fn create_file_link(target: &Path, link: &Path) -> Result<(), String> {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateSymbolicLinkW(link: *const u16, target: *const u16, flags: u32) -> u8;
        }

        fn wide(value: &OsStr) -> Vec<u16> {
            value.encode_wide().chain(Some(0)).collect()
        }

        const SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE: u32 = 0x2;
        let link_wide = wide(link.as_os_str());
        let target_wide = wide(target.as_os_str());
        for flags in [SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE, 0] {
            if unsafe { CreateSymbolicLinkW(link_wide.as_ptr(), target_wide.as_ptr(), flags) } != 0
            {
                return Ok(());
            }
        }
        fs::hard_link(target, link).map_err(|error| error.to_string())
    }

    #[cfg(unix)]
    fn remove_directory_link(link: &Path) {
        fs::remove_file(link).unwrap();
    }

    #[cfg(windows)]
    fn remove_directory_link(link: &Path) {
        fs::remove_dir(link).unwrap();
    }

    fn result_label(result: Result<(), ModConfigurationError>) -> &'static str {
        match result {
            Ok(()) => "ok",
            Err(ModConfigurationError::RevisionConflict) => "conflict",
            Err(ModConfigurationError::Failed(_)) => "failed",
        }
    }

    fn wait_for_file(path: &Path) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        path.exists()
    }

    fn start_child_writer(
        root: &Path,
        id: &str,
        revision: &str,
        patch: &str,
        control: &Path,
    ) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "save_mod_settings::tests::native_settings_child_process",
                "--nocapture",
            ])
            .env("SF_NATIVE_SETTINGS_ROOT", root)
            .env("SF_NATIVE_SETTINGS_ID", id)
            .env("SF_NATIVE_SETTINGS_REVISION", revision)
            .env("SF_NATIVE_SETTINGS_PATCH", patch)
            .env("SF_NATIVE_SETTINGS_CONTROL", control)
            .spawn()
            .unwrap()
    }

    #[test]
    fn partial_patch_preserves_other_settings_and_stale_revision_does_not_overwrite() {
        let root = new_root("partial");
        write_mod(&root, "example", None);
        let before = read_mod_settings_snapshot(&root, "example").unwrap();
        save_mod_settings_with_status(
            &root,
            "example",
            &serde_json::json!({"radius_multiplier":2.5}),
            &before.revision,
        )
        .unwrap();

        let package = shroudforge_loader_config::paths::mods_dir(&root).join("example");
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(package.join("extended.mod.json")).unwrap()).unwrap();
        assert_eq!(saved["enabled"], true);
        assert_eq!(saved["launcher"], "SF");
        assert_eq!(saved["targets"], serde_json::json!(["client"]));
        assert_eq!(saved["settings"]["radius_multiplier"]["value"], 2.5);
        assert_eq!(saved["settings"]["mode"]["value"], "vein");
        assert_eq!(saved["settings"]["activation_key"]["value"], "SDL:70");
        assert_eq!(saved["settings"]["toggle_migration_revision"]["value"], 1);
        assert_eq!(saved["settings"]["radius_key"]["value"], "SDL:82");

        let after_first_patch = read_mod_settings_snapshot(&root, "example").unwrap();
        save_mod_settings_with_status(
            &root,
            "example",
            &serde_json::json!({"mode":"circle"}),
            &after_first_patch.revision,
        )
        .unwrap();
        let after_second_patch = read_mod_settings_snapshot(&root, "example").unwrap();
        assert_eq!(after_second_patch.values["mode"], "circle");
        assert_eq!(after_second_patch.values["radius_multiplier"], 2.5);

        let bytes_before_stale_write = fs::read(package.join("extended.mod.json")).unwrap();
        let stale = save_mod_settings_with_status(
            &root,
            "example",
            &serde_json::json!({"radius_multiplier":9.0}),
            &before.revision,
        );
        assert_eq!(stale, Err(ModConfigurationError::RevisionConflict));
        assert_eq!(
            fs::read(package.join("extended.mod.json")).unwrap(),
            bytes_before_stale_write
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_directory_junction_cannot_shadow_the_admitted_package() {
        let root = new_root("junction-containment");
        let outside = root.with_file_name(format!(
            "{}-outside",
            root.file_name().unwrap().to_string_lossy()
        ));
        fs::create_dir_all(&outside).unwrap();
        let mods = shroudforge_loader_config::paths::mods_dir(&root);
        fs::create_dir_all(&mods).unwrap();
        let alias = mods.join("00-external-alias");
        create_directory_link(&outside, &alias).unwrap();
        let real_package = mods.join("99-real-package");
        write_mod_at(&outside, "example", None, true);
        write_mod_at(&real_package, "example", None, true);
        let outside_before = fs::read(outside.join("extended.mod.json")).unwrap();
        let binding =
            crate::package_lookup::capture_admitted_mod_package(&root, "example", &real_package)
                .unwrap();

        assert!(!crate::package_lookup::is_admitted_package_directory(
            &root, &alias
        ));
        assert_eq!(
            crate::package_lookup::resolve_mod_package(&root, "example").unwrap(),
            real_package.canonicalize().unwrap()
        );
        let before = crate::read_bound_mod_settings_snapshot(&root, &binding).unwrap();
        save_bound_mod_settings_with_status(
            &root,
            &binding,
            &serde_json::json!({"radius_multiplier":2.0}),
            &before.revision,
        )
        .unwrap();

        let saved_real: serde_json::Value =
            serde_json::from_slice(&fs::read(real_package.join("extended.mod.json")).unwrap())
                .unwrap();
        assert_eq!(saved_real["settings"]["radius_multiplier"]["value"], 2.0);
        assert_eq!(saved_real["settings"]["mode"]["value"], "vein");
        assert_eq!(
            fs::read(outside.join("extended.mod.json")).unwrap(),
            outside_before
        );

        remove_directory_link(&alias);
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn duplicate_installed_ids_fail_closed_without_writing_either_package() {
        let root = new_root("duplicate-id");
        let mods = shroudforge_loader_config::paths::mods_dir(&root);
        let first = mods.join("first");
        let second = mods.join("second");
        write_mod_at(&first, "example", None, true);
        write_mod_at(&second, "example", None, true);
        let binding =
            crate::package_lookup::capture_admitted_mod_package(&root, "example", &first).unwrap();
        let first_before = fs::read(first.join("extended.mod.json")).unwrap();
        let second_before = fs::read(second.join("extended.mod.json")).unwrap();

        assert!(
            crate::package_lookup::resolve_mod_package(&root, "example")
                .unwrap_err()
                .contains("duplicate installed mod ID")
        );
        assert!(crate::read_mod_settings_snapshot(&root, "example").is_err());
        assert!(crate::read_bound_mod_settings_snapshot(&root, &binding).is_err());
        assert!(
            save_mod_settings_with_status(
                &root,
                "example",
                &serde_json::json!({"radius_multiplier":2.0}),
                "revision",
            )
            .is_err()
        );
        assert!(
            save_bound_mod_settings_with_status(
                &root,
                &binding,
                &serde_json::json!({"radius_multiplier":2.0}),
                "revision",
            )
            .is_err()
        );
        assert_eq!(
            fs::read(first.join("extended.mod.json")).unwrap(),
            first_before
        );
        assert_eq!(
            fs::read(second.join("extended.mod.json")).unwrap(),
            second_before
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn replaced_or_removed_package_cannot_rebind_a_live_settings_owner() {
        let root = new_root("package-rebind");
        let mods = shroudforge_loader_config::paths::mods_dir(&root);
        let package = mods.join("example");
        write_mod_at(&package, "example", None, true);
        let binding =
            crate::package_lookup::capture_admitted_mod_package(&root, "example", &package)
                .unwrap();
        let before = crate::read_bound_mod_settings_snapshot(&root, &binding).unwrap();
        let old_extension = fs::read(package.join("extended.mod.json")).unwrap();
        let removed_original = root.join("removed-original-package");
        fs::rename(&package, &removed_original).unwrap();
        assert!(crate::read_bound_mod_settings_snapshot(&root, &binding).is_err());

        write_mod_at(&package, "example", None, true);
        let replacement_extension = fs::read(package.join("extended.mod.json")).unwrap();
        assert!(crate::read_bound_mod_settings_snapshot(&root, &binding).is_err());
        assert!(
            save_bound_mod_settings_with_status(
                &root,
                &binding,
                &serde_json::json!({"radius_multiplier":9.0}),
                &before.revision,
            )
            .is_err()
        );
        assert_eq!(
            fs::read(removed_original.join("extended.mod.json")).unwrap(),
            old_extension
        );
        assert_eq!(
            fs::read(package.join("extended.mod.json")).unwrap(),
            replacement_extension
        );

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn linked_manifest_leaves_fail_closed_for_snapshot_revision_and_patch() {
        for linked_name in ["mod.json", "extended.mod.json"] {
            let root = new_root(if linked_name == "mod.json" {
                "linked-mod-manifest"
            } else {
                "linked-extension-manifest"
            });
            let outside = root.with_file_name(format!(
                "{}-outside",
                root.file_name().unwrap().to_string_lossy()
            ));
            let package = shroudforge_loader_config::paths::mods_dir(&root).join("example");
            write_mod_at(&package, "example", None, true);
            write_mod_at(&outside, "example", None, true);
            let binding =
                crate::package_lookup::capture_admitted_mod_package(&root, "example", &package)
                    .unwrap();
            let outside_manifest = fs::read(outside.join(linked_name)).unwrap();
            let local_extension = fs::read(package.join("extended.mod.json")).unwrap();
            let leaf = package.join(linked_name);
            fs::remove_file(&leaf).unwrap();
            create_file_link(&outside.join(linked_name), &leaf).unwrap();

            assert!(!crate::package_lookup::is_admitted_package_directory(
                &root, &package
            ));
            assert!(crate::read_mod_settings_snapshot(&root, "example").is_err());
            assert!(crate::read_bound_mod_settings_snapshot(&root, &binding).is_err());
            assert!(
                save_mod_settings_with_status(
                    &root,
                    "example",
                    &serde_json::json!({"radius_multiplier":9.0}),
                    "revision",
                )
                .is_err()
            );
            assert!(
                save_bound_mod_settings_with_status(
                    &root,
                    &binding,
                    &serde_json::json!({"radius_multiplier":9.0}),
                    "revision",
                )
                .is_err()
            );
            assert_eq!(
                fs::read(outside.join(linked_name)).unwrap(),
                outside_manifest
            );
            if linked_name == "mod.json" {
                assert_eq!(
                    fs::read(package.join("extended.mod.json")).unwrap(),
                    local_extension
                );
            }
            assert_eq!(fs::read(&leaf).unwrap(), outside_manifest);

            fs::remove_file(&leaf).unwrap();
            fs::remove_dir_all(&root).unwrap();
            fs::remove_dir_all(&outside).unwrap();
        }
    }

    #[test]
    fn native_settings_child_process() {
        let Ok(root) = std::env::var("SF_NATIVE_SETTINGS_ROOT") else {
            return;
        };
        let id = std::env::var("SF_NATIVE_SETTINGS_ID").unwrap();
        let revision = std::env::var("SF_NATIVE_SETTINGS_REVISION").unwrap();
        let patch = std::env::var("SF_NATIVE_SETTINGS_PATCH").unwrap();
        let control = PathBuf::from(std::env::var_os("SF_NATIVE_SETTINGS_CONTROL").unwrap());
        fs::write(control.join("native-ready"), b"ready").unwrap();
        assert!(
            wait_for_file(&control.join("go")),
            "parent did not release native writer"
        );
        let values: serde_json::Value = serde_json::from_str(&patch).unwrap();
        let result = save_mod_settings_with_status(Path::new(&root), &id, &values, &revision);
        fs::write(control.join("native-result"), result_label(result)).unwrap();
    }

    #[test]
    fn cross_process_stale_writer_cannot_overwrite_the_winning_patch() {
        let root = new_root("cross-process-revision");
        write_mod(&root, "example", None);
        let revision = read_mod_settings_snapshot(&root, "example")
            .unwrap()
            .revision;
        let control = root.join("cross-process");
        fs::create_dir_all(&control).unwrap();
        let mut child = start_child_writer(
            &root,
            "example",
            &revision,
            r#"{"radius_multiplier":2.0}"#,
            &control,
        );
        let child_ready = wait_for_file(&control.join("native-ready"));
        fs::write(control.join("go"), b"go").unwrap();
        assert!(child_ready, "native writer did not reach its start barrier");
        let ui_result = save_mod_settings_with_status(
            &root,
            "example",
            &serde_json::json!({"radius_multiplier":3.0}),
            &revision,
        );
        let child_status = child.wait().unwrap();
        assert!(child_status.success());
        let native_result = fs::read_to_string(control.join("native-result")).unwrap();
        let ui_status = result_label(ui_result);
        assert!(
            (ui_status == "ok" && native_result == "conflict")
                || (ui_status == "conflict" && native_result == "ok")
        );
        let after = read_mod_settings_snapshot(&root, "example").unwrap();
        assert!(
            after.values["radius_multiplier"] == 2.0 || after.values["radius_multiplier"] == 3.0
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cross_process_hotkey_conflict_validation_has_one_winner() {
        let root = new_root("cross-process-hotkey");
        write_mod(&root, "alpha", Some("SDL:67"));
        write_mod(&root, "beta", Some("SDL:68"));
        let alpha_revision = read_mod_settings_snapshot(&root, "alpha").unwrap().revision;
        let beta_revision = read_mod_settings_snapshot(&root, "beta").unwrap().revision;
        let control = root.join("cross-process");
        fs::create_dir_all(&control).unwrap();
        let mut child = start_child_writer(
            &root,
            "beta",
            &beta_revision,
            r#"{"toggle":"SDL:70"}"#,
            &control,
        );
        let child_ready = wait_for_file(&control.join("native-ready"));
        fs::write(control.join("go"), b"go").unwrap();
        assert!(child_ready, "native writer did not reach its start barrier");
        let ui_result = save_mod_settings_with_status(
            &root,
            "alpha",
            &serde_json::json!({"toggle":"SDL:70"}),
            &alpha_revision,
        );
        let child_status = child.wait().unwrap();
        assert!(child_status.success());
        let native_result = fs::read_to_string(control.join("native-result")).unwrap();
        let ui_status = result_label(ui_result);
        assert!(
            (ui_status == "ok" && native_result == "failed")
                || (ui_status == "failed" && native_result == "ok")
        );

        let alpha = read_mod_settings_snapshot(&root, "alpha").unwrap();
        let beta = read_mod_settings_snapshot(&root, "beta").unwrap();
        let same_binding = alpha.values["toggle"] == beta.values["toggle"];
        assert!(!same_binding);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enabling_a_mod_checks_hotkey_conflicts_before_writing_enabled_state() {
        let root = new_root("enable-hotkey-conflict");
        write_mod(&root, "alpha", Some("SDL:70"));
        write_mod_with_enabled(&root, "beta", Some("SDL:70"), false);
        let before = read_mod_settings_snapshot(&root, "beta").unwrap();
        let package = shroudforge_loader_config::paths::mods_dir(&root).join("beta");
        let bytes_before = fs::read(package.join("extended.mod.json")).unwrap();

        let result = crate::set_mod_enabled(&root, "beta", true, &before.revision);

        assert!(result.is_err());
        assert_eq!(
            fs::read(package.join("extended.mod.json")).unwrap(),
            bytes_before
        );
        let after = read_mod_settings_snapshot(&root, "beta").unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.values["toggle"], "SDL:70");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(package.join("extended.mod.json")).unwrap()
            )
            .unwrap()["enabled"],
            false
        );
        fs::remove_dir_all(root).unwrap();
    }
}
