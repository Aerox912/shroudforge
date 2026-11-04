use crate::mod_log_macros::{debug, warn};
use crate::mod_registry::ModInner;
use crate::path_types::{Path, PathBuf};
use crate::{
    FileSystem, IoError, Mod, ModEnvironmentErrorReport, ModError, ModErrorReport, ModRegistry,
};
use parking_lot::Mutex;
use std::{collections::HashMap, fs::DirEntry, sync::Arc};

impl ModRegistry {
    /// Strict prepared route. Every package view exists before its manifest or
    /// source inference is read. No directory creation, discovery or skipped
    /// package errors; the caller owns complete graph/identity admission.
    pub(crate) fn from_prepared(
        root:&Path,
        views:Vec<crate::prepared_package_view::PreparedPackageView>,
        policy:crate::PreparedManifestPolicy,
    ) -> Result<Self,ModEnvironmentErrorReport> {
        let mut mods=HashMap::new();
        for view in views {
            let path=view.root().to_owned();
            let mut fs=FileSystem::new_prepared(view);
            let info=crate::read_prepared_manifest(root.as_std_path(),&mut fs,policy).map_err(|message| {
                ModEnvironmentErrorReport::with_mod_errors(vec![ModErrorReport::new(&path,ModError::Json {
                    path:path.join("mod.json").to_string(),
                    source:serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData,message)),
                })],Self::default())
            })?;
            let id=info.id.clone();
            if mods.contains_key(&id) {
                return Err(ModEnvironmentErrorReport::with_mod_errors(vec![ModErrorReport::new(&path,ModError::DuplicateModId(id))],Self::default()));
            }
            mods.insert(id,Mod {inner:Arc::new(ModInner {info,fs:Mutex::new(fs)})});
        }
        Ok(Self {mods})
    }

    pub(crate) fn load(mods_dir: impl AsRef<Path>) -> Result<Self, ModEnvironmentErrorReport> {
        let mods_dir = mods_dir.as_ref().to_path_buf();
        let mut mods = HashMap::new();

        if let Err(e) = std::fs::create_dir_all(&mods_dir) {
            return Err(ModEnvironmentErrorReport::base_io(IoError {
                path: mods_dir.into_string(),
                source: e,
            }));
        }

        let mod_files = match std::fs::read_dir(&mods_dir) {
            Ok(entries) => entries,
            Err(e) => {
                return Err(ModEnvironmentErrorReport::base_io(IoError {
                    path: mods_dir.into_string(),
                    source: e,
                }));
            }
        };

        let mut errors = Vec::new();
        let mut duplicate_ids = std::collections::HashSet::new();

        for entry in mod_files {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        error = %e,
                        path = mods_dir.as_str(),
                        "Error reading entry in mods directory, skipping",
                    );

                    continue;
                }
            };

            let r#mod = match Self::load_mod(&mods, entry, mods_dir.parent().unwrap_or(&mods_dir)) {
                Ok(Some(m)) => m,
                Ok(None) => continue,
                Err(e) => {
                    if let ModError::DuplicateModId(id) = &e.error {
                        duplicate_ids.insert(id.clone());
                    }
                    errors.push(e);
                    continue;
                }
            };

            let mod_id = r#mod.info().id.clone();

            mods.insert(mod_id, r#mod);
        }

        for id in duplicate_ids {
            mods.remove(&id);
        }
        let registry = Self { mods };

        if !errors.is_empty() {
            Err(ModEnvironmentErrorReport::with_mod_errors(errors, registry))
        } else {
            Ok(registry)
        }
    }

    fn load_mod(
        mods: &HashMap<String, Mod>,
        entry: DirEntry,
        root: &Path,
    ) -> Result<Option<Mod>, ModErrorReport> {
        let path = PathBuf::from_path_buf(entry.path()).map_err(|e| {
            ModErrorReport::new(
                entry.path().to_string_lossy().to_string(),
                ModError::Utf8(e),
            )
        })?;

        let file_type = entry.file_type().map_err(|e| {
            ModErrorReport::new(
                path.clone(),
                ModError::Io(IoError {
                    path: path.to_string(),
                    source: e,
                }),
            )
        })?;
        let file_name = path.file_name().unwrap_or_default();

        if file_name.starts_with('.') {
            debug!(path = path.as_str(), "Skipping hidden file or directory",);

            return Ok(None);
        }

        if !file_type.is_dir() {
            debug!(path = path.as_str(), "Skipping non-package file");
            return Ok(None);
        }

        if !path.join("mod.json").exists() {
            debug!(
                path = path.as_str(),
                "Folder has no Lua manifest, native components are managed by the DLL Manager"
            );
            return Ok(None);
        }

        let mut fs = FileSystem::new_disk(&path).map_err(|e| {
            ModErrorReport::new(
                path.clone(),
                ModError::Io(IoError {
                    path: path.to_string(),
                    source: e,
                }),
            )
        })?;

        let mod_info =
            crate::config::read_manifest(root.as_std_path(), &mut fs).map_err(|message| {
                ModErrorReport::new(
                    path.clone(),
                    ModError::Json {
                        path: path.join("mod.json").to_string(),
                        source: serde_json::Error::io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            message,
                        )),
                    },
                )
            })?;

        let mod_id = mod_info.id.clone();

        if mods.contains_key(&mod_id) {
            return Err(ModErrorReport::new(
                path.clone(),
                ModError::DuplicateModId(mod_id.clone()),
            )
            .with_id(mod_id));
        }

        Ok(Some(Mod {
            inner: Arc::new(ModInner {
                info: mod_info,
                fs: Mutex::new(fs),
            }),
        }))
    }
}
