use crate::{
    logging::{debug, error, warn},
    type_aliases::Path,
};
use kfc::{container::KFCFile, reflection::TypeRegistry};
use sha2::{Digest, Sha256};
use std::{fs::File, io::BufReader};
pub fn load_type_registry(
    game_dir: &Path,
    cache_dir: &Path,
    file_name: &str,
) -> Result<(TypeRegistry, bool), ()> {
    load_with_cache_policy(game_dir, cache_dir, file_name, true)
}

/// Reuse a validated cache or extract an operation-local registry. Never create
/// a cache directory or alter the shared registry/identity documents.
pub(crate) fn load_current_composition_registry(
    game_dir: &Path,
    cache_dir: &Path,
    file_name: &str,
) -> Result<(TypeRegistry, bool), ()> {
    load_with_cache_policy(game_dir, cache_dir, file_name, false)
}

fn load_with_cache_policy(
    game_dir: &Path,
    cache_dir: &Path,
    file_name: &str,
    persist_cache: bool,
) -> Result<(TypeRegistry, bool), ()> {
    if persist_cache {
        if let Err(e) = std::fs::create_dir_all(cache_dir) {
            error!(
                error = %e,
                path = ?cache_dir,
                "Failed to create cache directory, unable to load or save type registry",
            );
            return Err(());
        }
    }

    let types_path = cache_dir.join(format!("types-{file_name}.json"));
    let identity_path = cache_dir.join(format!("types-{file_name}.identity.json"));
    let exe_path = game_dir.join(file_name).with_extension("exe");
    let kfc_path = game_dir.join(file_name).with_extension("kfc");
    // File length and mtime are not an executable identity (restores and
    // repackaging can preserve both). Schema 2 invalidates the old cache once.
    let identity = std::fs::read(&exe_path).ok().map(|bytes| {
        serde_json::json!({"schemaVersion": 2, "executableSha256": format!("{:x}", Sha256::digest(bytes)),
            "reflectionExtractor": "kfc-a404b15cde40fc7bd43d812bb460ce40d2a9d3b1"})
    });
    let cached_identity = std::fs::read(&identity_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let identity_matches = identity.is_some() && identity == cached_identity;

    let type_registry = match File::open(&types_path).and_then(|file| {
        if identity_matches {
            Ok(file)
        } else {
            Err(std::io::Error::other("game executable changed"))
        }
    }) {
        Ok(file) => {
            let reader = BufReader::new(file);

            match serde_json::from_reader::<_, TypeRegistry>(reader) {
                Ok(registry) => Some(registry),
                Err(e) => {
                    debug!(
                        error = %e,
                        path = ?types_path,
                        "Failed to read type registry from file, attempting to extract types...",
                    );

                    None
                }
            }
        }
        Err(e) => {
            debug!(
                error = %e,
                path = ?types_path,
                "Failed to read type registry from file, attempting to extract types...",
            );

            None
        }
    };

    let version_tag = match KFCFile::get_version_tag(&kfc_path) {
        Ok(tag) => Some(tag),
        Err(e) => {
            warn!(
                error = %e,
                path = ?types_path,
                kfc_path = ?kfc_path,
                "Failed to get version tag from KFC file, assuming types are outdated, attempting to extract types...",
            );

            None
        }
    };

    let type_registry = match type_registry {
        Some(type_registry) => {
            if let Some(version_tag) = &version_tag {
                if version_tag != &type_registry.version {
                    debug!(
                        path = ?types_path,
                        kfc_path = ?kfc_path,
                        "Type registry is outdated, attempting to extract types again..."
                    );

                    None
                } else {
                    Some(type_registry)
                }
            } else {
                None
            }
        }
        None => None,
    };

    let (type_registry, is_dirty) = match type_registry {
        Some(type_registry) => (type_registry, false),
        None => match TypeRegistry::load_from_executable(&exe_path) {
            Ok(mut registry) => {
                if let Some(version_tag) = version_tag {
                    registry.version = version_tag;

                    if persist_cache {
                        match serde_json::to_value(&registry) {
                            Ok(json) => {
                                if let Err(e) = shroudforge_loader_config::config::write_json(
                                    types_path.as_std_path(),
                                    &json,
                                ) {
                                    debug!(
                                        error = %e,
                                        path = ?types_path,
                                        "Failed to write type registry to file",
                                    );
                                } else if let Some(identity) = &identity {
                                    if let Err(error) =
                                        shroudforge_loader_config::config::write_json(
                                            identity_path.as_std_path(),
                                            identity,
                                        )
                                    {
                                        debug!(%error, "Could not save executable identity for type cache");
                                    }
                                }
                            }
                            Err(e) => {
                                debug!(
                                    error = %e,
                                    path = ?types_path,
                                    "Failed to serialize type registry to JSON",
                                );
                            }
                        }
                    }
                } else {
                    debug!(
                        path = ?types_path,
                        kfc_path = ?kfc_path,
                        "No version tag found, type registry cannot be saved",
                    );
                }

                (registry, true)
            }
            Err(e) => {
                error!(
                    error = %e,
                    path = ?types_path,
                    exe_path = ?exe_path,
                    "Failed to load type registry",
                );

                return Err(());
            }
        },
    };

    debug!(
        path = ?types_path,
        exe_path = ?exe_path,
        kfc_path = ?kfc_path,
        version = type_registry.version,
        is_dirty = is_dirty,
        "Type registry loaded successfully",
    );

    Ok((type_registry, is_dirty))
}
