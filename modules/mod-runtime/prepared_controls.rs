//! Explicit immutable controls for the private effective-source producer route.
//! These are admitted source bytes or a qualified absence, not a filesystem
//! resolver. Construction itself grants no source or operation authority.
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub struct PreparedControls {
    loader: Arc<Value>,
    expected_generation: Option<String>,
}

impl PreparedControls {
    pub fn from_admitted_bytes(bytes: Option<&[u8]>) -> Result<Self, String> {
        if bytes.is_some_and(|raw| raw.len() > 1024 * 1024) {
            return Err("prepared loader control exceeds1MiB".into());
        }
        let loader = shroudforge_loader_config::config::read_loader_from_admitted(bytes)?;
        Ok(Self {
            loader: Arc::new(loader),
            expected_generation: None,
        })
    }
    pub(crate) fn bind_generation(&mut self, generation: &str) -> Result<(), String> {
        if generation.is_empty()
            || generation.len() > 128
            || !generation.is_ascii()
            || generation.chars().any(char::is_control)
        {
            return Err("invalid prepared generation identity".into());
        }
        self.expected_generation = Some(generation.to_owned());
        Ok(())
    }
    /// Bind the complete KFC generation before the caller opens original assets.
    /// The branch and timestamp separators are part of the identity, not delimiters
    /// for a shorter build-only identity. Actual asset equality is checked later.
    pub fn with_generation(mut self, generation: &str) -> Result<Self, String> {
        self.bind_generation(generation)?;
        Ok(self)
    }
    pub(crate) fn expected_generation(&self) -> Option<&str> {
        self.expected_generation.as_deref()
    }
    pub fn manifest_policy(&self) -> mod_loader::PreparedManifestPolicy {
        let detection = &self.loader["modules"]["modDetection"];
        mod_loader::PreparedManifestPolicy {
            allow_unknown_manifest_fields: detection["enabled"].as_bool().unwrap_or(true)
                && detection["tolerateUnknownManifestFields"]
                    .as_bool()
                    .unwrap_or(true),
        }
    }
    pub fn export_directory(&self, root: &Path) -> PathBuf {
        match self
            .loader
            .pointer("/paths/exports")
            .and_then(Value::as_str)
        {
            Some(text) => {
                let path = PathBuf::from(text);
                if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                }
            }
            None => shroudforge_loader_config::paths::default_directory(root, "exports")
                .expect("embedded exports path"),
        }
    }
    pub fn locale(&self) -> String {
        self.loader["general"]["locale"]
            .as_str()
            .filter(|locale| !locale.is_empty())
            .unwrap_or("en")
            .to_owned()
    }
    /// Registered but unadmitted side-effect/control callbacks refuse before
    /// entering their ordinary implementation. No success/no-op substitution.
    pub(crate) fn refuse_functions(
        lua: &mlua::Lua,
        table: &mlua::Table,
        domain: &'static str,
    ) -> mlua::Result<()> {
        let keys = table
            .clone()
            .pairs::<mlua::Value, mlua::Value>()
            .filter_map(|pair| match pair {
                Ok((key, mlua::Value::Function(_))) => Some(Ok(key)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<mlua::Result<Vec<_>>>()?;
        for key in keys {
            table.raw_set(
                key,
                lua.create_function(
                    move |_, _: mlua::MultiValue| -> mlua::Result<mlua::MultiValue> {
                        Err(mlua::Error::runtime(format!(
                            "{domain} is not admitted in prepared source generation"
                        )))
                    },
                )?,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn complete_kfc_generation_survives_prepared_control_admission() {
        let generation = "1287369|^/game38/branches/release_1.0|2026-11-04T11:17:54.051725Z";
        let controls = PreparedControls::from_admitted_bytes(None)
            .unwrap()
            .with_generation(generation)
            .unwrap();
        assert_eq!(controls.expected_generation(), Some(generation));
        assert_ne!(controls.expected_generation(), Some("1287369"));
        assert_ne!(controls.expected_generation(), Some("1287369|other|2026-11-04T11:17:54.051725Z"));
        assert_ne!(controls.expected_generation(), Some("1287369|^/game38/branches/release_1.0|2026-11-04T11:17:54.051726Z"));
    }

    #[test]
    fn generation_limits_stay_closed_and_failed_binding_keeps_previous_identity() {
        let mut controls = PreparedControls::from_admitted_bytes(None).unwrap();
        controls.bind_generation("legacy-build").unwrap();
        for bad in ["", "a\nb", "a\rb", "a\0b", "a\tb", "a\u{7f}b", "génération", &"a".repeat(129)] {
            assert_eq!(controls.bind_generation(bad).unwrap_err(), "invalid prepared generation identity");
            assert_eq!(controls.expected_generation(), Some("legacy-build"));
            assert!(PreparedControls::from_admitted_bytes(None).unwrap().with_generation(bad).is_err());
        }
        let maximum = "a".repeat(128);
        controls.bind_generation(&maximum).unwrap();
        assert_eq!(controls.expected_generation(), Some(maximum.as_str()));
    }

    #[test]
    fn present_and_known_absent_snapshots_never_consult_later_disk_settings() {
        let root =
            PathBuf::from(r"E:\Build\sf178-prepared-controls-synthetic-fixtures-20261104-r1")
                .join(uuid::Uuid::new_v4().to_string());
        let absent = PreparedControls::from_admitted_bytes(None).unwrap();
        let admitted=br#"{"schemaVersion":1,"general":{"locale":"nl"},"paths":{"exports":"admitted-export"}}"#;
        let present = PreparedControls::from_admitted_bytes(Some(admitted)).unwrap();
        let path = shroudforge_loader_config::paths::loader_config(&root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path,br#"{"schemaVersion":1,"general":{"locale":"fr"},"paths":{"exports":"ambient-export"}}"#).unwrap();
        assert_eq!(present.locale(), "nl");
        assert_eq!(
            present.export_directory(&root),
            root.join("admitted-export")
        );
        let defaults = shroudforge_loader_config::config::read_loader_defaults().unwrap();
        assert_eq!(
            absent.locale(),
            defaults["general"]["locale"].as_str().unwrap()
        );
        assert_eq!(
            absent.export_directory(&root),
            root.join("shroudforge/exports")
        );
        assert_eq!(
            shroudforge_loader_config::paths::export_dir(&root),
            root.join("ambient-export")
        );
    }

    #[test]
    fn admitted_parser_matches_ordinary_valid_values_but_never_hides_present_errors() {
        let root =
            PathBuf::from(r"E:\Build\sf178-prepared-controls-synthetic-fixtures-20261104-r1")
                .join(uuid::Uuid::new_v4().to_string());
        let path = shroudforge_loader_config::paths::loader_config(&root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for raw in [
            br#"{"schemaVersion":1}"#.as_slice(),
            br#"{"schemaVersion":1,"general":{"locale":"de"}}"#,
            br#"{"schemaVersion":1,"modules":{"network":{"enabled":false}}}"#,
        ] {
            std::fs::write(&path, raw).unwrap();
            assert_eq!(
                shroudforge_loader_config::config::read_loader(&root).unwrap(),
                shroudforge_loader_config::config::read_loader_from_admitted(Some(raw)).unwrap()
            );
        }
        for raw in [
            b"not json".as_slice(),
            b"false",
            br#"{"schemaVersion":999}"#,
        ] {
            assert!(PreparedControls::from_admitted_bytes(Some(raw)).is_err());
        }
        assert!(PreparedControls::from_admitted_bytes(Some(&vec![b' '; 1024 * 1024 + 1])).is_err());
    }

    #[test]
    fn unadmitted_callbacks_refuse_before_running_and_preserve_nonfunction_metadata() {
        let lua = mlua::Lua::new();
        let table = lua.create_table().unwrap();
        let calls = Rc::new(Cell::new(0));
        let body_calls = calls.clone();
        table
            .raw_set(
                "status",
                lua.create_function(move |_, ()| {
                    body_calls.set(body_calls.get() + 1);
                    Ok("unadmitted body")
                })
                .unwrap(),
            )
            .unwrap();
        table.raw_set("phase", "pregame").unwrap();
        PreparedControls::refuse_functions(&lua, &table, "runtime.network").unwrap();
        let error = table
            .raw_get::<mlua::Function>("status")
            .unwrap()
            .call::<mlua::Value>(())
            .unwrap_err();
        assert!(error.to_string().contains("not admitted"));
        assert_eq!(calls.get(), 0);
        assert_eq!(table.raw_get::<String>("phase").unwrap(), "pregame");
    }
}
