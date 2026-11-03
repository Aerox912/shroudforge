use crate::config_revision;
use crate::package_files::{package_has_file, read_package_file, write_package_json};
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModConfigurationError {
    RevisionConflict,
    Failed(String),
}

impl fmt::Display for ModConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RevisionConflict => formatter
                .write_str("Mod settings changed since they were displayed. Reload before saving"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ModConfigurationError {}

pub(crate) fn update_mod_configuration(
    root: &Path,
    id: &str,
    expected_revision: &str,
    update: impl FnOnce(&crate::ModManifest, &mut serde_json::Value) -> Result<(), String>,
) -> Result<(), ModConfigurationError> {
    update_mod_configuration_for_package(root, id, None, expected_revision, update)
}

pub(crate) fn update_bound_mod_configuration(
    root: &Path,
    binding: &crate::package_lookup::ModPackageBinding,
    expected_revision: &str,
    update: impl FnOnce(&crate::ModManifest, &mut serde_json::Value) -> Result<(), String>,
) -> Result<(), ModConfigurationError> {
    update_mod_configuration_for_package(
        root,
        binding.id(),
        Some(binding),
        expected_revision,
        update,
    )
}

fn update_mod_configuration_for_package(
    root: &Path,
    id: &str,
    binding: Option<&crate::package_lookup::ModPackageBinding>,
    expected_revision: &str,
    update: impl FnOnce(&crate::ModManifest, &mut serde_json::Value) -> Result<(), String>,
) -> Result<(), ModConfigurationError> {
    let _installation_lock = shroudforge_loader_config::config::installation_lock(root)
        .map_err(ModConfigurationError::Failed)?;
    let package = match binding {
        Some(binding) => crate::package_lookup::validate_mod_package_binding_locked(root, binding),
        None => crate::package_lookup::resolve_mod_package(root, id),
    }
    .map_err(ModConfigurationError::Failed)?;
    let manifest =
        crate::config::read_manifest_path(root, &package).map_err(ModConfigurationError::Failed)?;
    let current_state =
        serde_json::json!({"enabled":manifest.enabled,"settings":manifest.setting_values});
    if config_revision(&current_state) != expected_revision {
        return Err(ModConfigurationError::RevisionConflict);
    }
    let mut extension = match read_package_file(&package, "extended.mod.json") {
        Ok(bytes) => {
            let mut extension = serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|error| ModConfigurationError::Failed(error.to_string()))?;
            prepare_existing_extension(&mut extension).map_err(ModConfigurationError::Failed)?;
            extension
        }
        Err(_error)
            if !package_has_file(&package, "extended.mod.json")
                .map_err(ModConfigurationError::Failed)? =>
        {
            crate::ensure_eml_extension::ensure_eml_extension(&package, &manifest)
                .map_err(ModConfigurationError::Failed)?
        }
        Err(error) => return Err(ModConfigurationError::Failed(error)),
    };
    update(&manifest, &mut extension).map_err(ModConfigurationError::Failed)?;
    crate::parse_mod_manifest::validate_extended_mod_document(root, &extension)
        .map_err(ModConfigurationError::Failed)?;
    let wire_manifest = serde_json::from_slice(
        &read_package_file(&package, "mod.json").map_err(ModConfigurationError::Failed)?,
    )
    .map_err(|error| ModConfigurationError::Failed(error.to_string()))?;
    crate::config::parse_manifest_with_extension(root, wire_manifest, Some(extension.clone()))
        .map_err(ModConfigurationError::Failed)?;
    write_package_json(&package, "extended.mod.json", &extension)
        .map_err(ModConfigurationError::Failed)
}

fn prepare_existing_extension(extension: &mut serde_json::Value) -> Result<(), String> {
    let launcher_is_eml = extension
        .get("launcher")
        .and_then(serde_json::Value::as_str)
        == Some("EML");
    let object = extension
        .as_object_mut()
        .ok_or("extended.mod.json must be an object")?;
    object
        .entry("settings")
        .or_insert_with(|| serde_json::json!({}));
    if !object.contains_key("targets") {
        let targets = if launcher_is_eml {
            serde_json::json!(["client", "server"])
        } else {
            serde_json::json!(["client"])
        };
        let old = std::mem::take(object);
        let mut prepared = serde_json::Map::new();
        for (key, value) in old {
            let insert_targets = key == "enabled";
            prepared.insert(key, value);
            if insert_targets {
                prepared.insert("targets".into(), targets.clone());
            }
        }
        *object = prepared;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_package(extension: Option<&[u8]>) -> (std::path::PathBuf, std::path::PathBuf) {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("shroudforge-mod-configuration-{suffix}"));
        let package = shroudforge_loader_config::paths::mods_dir(&root).join("example");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("mod.json"),
            br#"{"id":"example","name":"Example","version":"1.0.0"}"#,
        )
        .unwrap();
        if let Some(extension) = extension {
            std::fs::write(package.join("extended.mod.json"), extension).unwrap();
        }
        (root, package)
    }

    #[test]
    fn updating_shroudforge_configuration_keeps_its_provenance() {
        let (root, package) = test_package(Some(
            br#"{"schemaVersion":1,"enabled":false,"launcher":"SF"}"#,
        ));
        let revision = config_revision(&serde_json::json!({"enabled":false,"settings":{}}));

        update_mod_configuration(&root, "example", &revision, |_manifest, extension| {
            extension["enabled"] = true.into();
            Ok(())
        })
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(package.join("extended.mod.json")).unwrap())
                .unwrap();
        assert_eq!(saved["enabled"], true);
        assert_eq!(saved["launcher"], "SF");
        assert_eq!(saved["targets"], serde_json::json!(["client"]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn first_change_to_legacy_eml_mod_creates_its_compatibility_extension() {
        let (root, package) = test_package(None);
        let revision = config_revision(&serde_json::json!({"enabled":false,"settings":{}}));

        update_mod_configuration(&root, "example", &revision, |_manifest, extension| {
            extension["enabled"] = true.into();
            Ok(())
        })
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(package.join("extended.mod.json")).unwrap())
                .unwrap();
        assert_eq!(saved["enabled"], true);
        assert_eq!(saved["launcher"], "EML");
        assert_eq!(saved["targets"], serde_json::json!(["client", "server"]));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_configuration_revision_does_not_write() {
        let (root, package) = test_package(Some(
            br#"{"schemaVersion":1,"enabled":false,"launcher":"SF"}"#,
        ));

        let result = update_mod_configuration(&root, "example", "stale", |_manifest, extension| {
            extension["enabled"] = true.into();
            Ok(())
        });

        assert!(result.is_err());
        let saved = std::fs::read(package.join("extended.mod.json")).unwrap();
        assert_eq!(
            saved,
            br#"{"schemaVersion":1,"enabled":false,"launcher":"SF"}"#
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
