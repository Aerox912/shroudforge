use super::*;
use serde_json::json;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "shroudforge-zoom-contract-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn outputs() -> BTreeMap<String, Stamp> {
    BTreeMap::from([
        ("enshrouded.kfc".into(), (1, "a".repeat(64))),
        ("enshrouded.kfc_resources".into(), (2, "b".repeat(64))),
        ("enshrouded_001.dat".into(), (3, "c".repeat(64))),
    ])
}

fn reviewed_output_bytes(name: &str) -> Vec<u8> {
    format!("reviewed-current-composition:{name}").into_bytes()
}

fn full_reviewed_outputs() -> BTreeMap<String, Stamp> {
    let mut outputs = BTreeMap::new();
    let mut names = vec![
        "enshrouded.kfc".to_owned(),
        "enshrouded.kfc_resources".to_owned(),
    ];
    names.extend((1..=256).map(|index| format!("enshrouded_{index:03}.dat")));
    for name in names {
        let bytes = reviewed_output_bytes(&name);
        outputs.insert(name, (bytes.len() as u64, hash(&bytes)));
    }
    outputs
}

fn owned_194(full: &BTreeMap<String, Stamp>) -> BTreeMap<String, Stamp> {
    full.iter()
        .filter(|(name, _)| {
            matches!(name.as_str(), "enshrouded.kfc" | "enshrouded.kfc_resources")
                || name
                    .strip_prefix("enshrouded_")
                    .and_then(|value| value.strip_suffix(".dat"))
                    .is_some_and(|value| value.parse::<u32>().is_ok_and(|index| index <= 192))
        })
        .map(|(name, stamp)| (name.clone(), stamp.clone()))
        .collect()
}

#[test]
fn legacy_receipt_serialization_remains_schema_one_without_zoom_fields() -> Result<()> {
    let root = Path::new("E:/Scratch/zoom-schema-test");
    let state = Prepared {
        schema_version: 1,
        root: root.to_string_lossy().into(),
        fingerprint: "d".repeat(64),
        enabled: true,
        configured_extended_zoom: None,
        effective_zoom: None,
        previous_state_sha256: None,
        outputs: outputs(),
        target: None,
        resource_contribution: None,
    };
    validate_shape(root, &state)?;
    let value = serde_json::to_value(state)?;
    assert!(value.get("configuredExtendedZoom").is_none());
    assert!(value.get("effectiveZoom").is_none());
    assert!(value.get("previousStateSha256").is_none());
    Ok(())
}

#[test]
fn schema_two_records_parent_and_zoom_selection_separately() -> Result<()> {
    let root = Path::new("E:/Scratch/zoom-schema-test");
    for enabled in [false, true] {
        for configured in [false, true] {
            let state = Prepared {
                schema_version: 2,
                root: root.to_string_lossy().into(),
                fingerprint: "d".repeat(64),
                enabled,
                configured_extended_zoom: Some(configured),
                effective_zoom: Some(enabled && configured),
                previous_state_sha256: Some("e".repeat(64)),
                outputs: outputs(),
                target: None,
                resource_contribution: None,
            };
            validate_shape(root, &state)?;
        }
    }
    let mut malformed = Prepared {
        schema_version: 2,
        root: root.to_string_lossy().into(),
        fingerprint: "d".repeat(64),
        enabled: false,
        configured_extended_zoom: Some(true),
        effective_zoom: Some(true),
        previous_state_sha256: Some("e".repeat(64)),
        outputs: outputs(),
        target: None,
        resource_contribution: None,
    };
    assert!(validate_shape(root, &malformed).is_err());
    malformed.effective_zoom = Some(false);
    malformed.previous_state_sha256 = None;
    assert!(validate_shape(root, &malformed).is_err());
    Ok(())
}

fn zoom_bundle(extended: bool) -> Value {
    fn object(entries: Vec<(&str, Value)>) -> Value {
        Value::Struct(Box::new(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        ))
    }
    let scales = if extended {
        [16.0f32, 8.0, 4.0, 1.0, 0.35]
    } else {
        [16.0f32, 8.0, 4.0, 2.0, 0.7]
    };
    object(vec![
        ("opaque", Value::UInt(u64::MAX)),
        ("optional", Value::None),
        (
            "menu",
            object(vec![(
                "map",
                object(vec![(
                    "zoomConfigs",
                    Value::Array(
                        scales
                            .into_iter()
                            .map(|scale| {
                                object(vec![
                                    ("zoomScale", Value::Float(scale as f64)),
                                    ("isolineConfigs", Value::Array(vec![Value::UInt(u64::MAX)])),
                                ])
                            })
                            .collect(),
                    ),
                )]),
            )]),
        ),
    ])
}

#[test]
fn typed_zoom_transitions_preserve_non_zoom_values_and_repeated_state() -> Result<()> {
    for before_parent in [false, true] {
        for before_configured in [false, true] {
            for parent in [false, true] {
                for configured in [false, true] {
                    let before = before_parent && before_configured;
                    let after = parent && configured;
                    let result = zoom::transform(&zoom_bundle(before), before, after)?;
                    assert_eq!(result, zoom_bundle(after));
                    assert_eq!(zoom::transform(&result, after, after)?, result);
                }
            }
        }
    }
    assert!(zoom::transform(&zoom_bundle(true), false, true).is_err());
    assert!(zoom::transform(&zoom_bundle(false), true, false).is_err());
    Ok(())
}

#[test]
fn only_the_explicit_zoom_preference_is_normalized_from_package_identity() -> Result<()> {
    let base = json!({"settings":{"ExtendedZoom":{"value":true,"applyAt":"restart",
        "restartRequired":true,"assetPreparation":true,"label":"Extended zoom"}}});
    let mut on = base.clone();
    normalize_zoom_setting(&mut on)?;
    let mut off = base.clone();
    off["settings"]["ExtendedZoom"]["value"] = false.into();
    normalize_zoom_setting(&mut off)?;
    assert_eq!(on, off);

    let mut metadata = base.clone();
    metadata["settings"]["ExtendedZoom"]["label"] = "Changed label".into();
    normalize_zoom_setting(&mut metadata)?;
    assert_ne!(metadata, on);

    let mut unknown = base.clone();
    unknown["settings"]["Other"] = true.into();
    assert!(normalize_zoom_setting(&mut unknown).is_err());
    let mut invalid = base.clone();
    invalid["settings"]["ExtendedZoom"]["value"] = "true".into();
    assert!(normalize_zoom_setting(&mut invalid).is_err());
    let mut timing = base;
    timing["settings"]["ExtendedZoom"]["applyAt"] = "live".into();
    assert!(normalize_zoom_setting(&mut timing).is_err());
    Ok(())
}

fn migration_policy(effective_zoom: bool) -> Policy {
    let outputs = full_reviewed_outputs();
    Policy {
        schema_version: 2,
        mod_id: "esm-real-map".into(),
        package: "mods/esm-real-map".into(),
        generation: zoom::GENERATION.into(),
        executable_sha256: "a".repeat(64),
        resources: Vec::new(),
        retained_contents: Vec::new(),
        composition: Composition {
            fingerprint: "b".repeat(64),
            outputs: outputs.clone(),
            source_fingerprint: None,
            baseline_enabled: true,
            baseline_legacy_pickaxe_reach_migrated: false,
            excluded_packages: Vec::new(),
        },
        legacy_pickaxe_reach: Content {
            file: "legacy-reach.json".into(),
            sha256: "c".repeat(64),
        },
        zoom: Some(zoom::Contract {
            registry_sha256: "d".repeat(64),
        }),
        migration: Some(Migration {
            state_sha256: "e".repeat(64),
            fingerprint: "f".repeat(64),
            enabled: true,
            configured_extended_zoom: true,
            effective_zoom,
            outputs,
        }),
        layout_migration: None,
        resource_contribution: None,
    }
}

#[test]
fn migration_requires_exact_parent_on_zoom_on_and_full_reviewed_inventory() {
    assert!(validate_migration(&migration_policy(true)).is_ok());
    assert!(validate_migration(&migration_policy(false)).is_err());
    let mut incomplete = migration_policy(true);
    incomplete
        .migration
        .as_mut()
        .unwrap()
        .outputs
        .remove("enshrouded_001.dat");
    assert!(validate_migration(&incomplete).is_err());

    let mut unowned_stamp_mismatch = migration_policy(true);
    let unowned = unowned_stamp_mismatch
        .composition
        .outputs
        .get_mut("enshrouded_256.dat")
        .unwrap();
    unowned.1 = "0".repeat(64);
    assert!(validate_migration(&unowned_stamp_mismatch).is_err());
}

#[test]
fn layout_migration_pins_all_four_parent_and_zoom_selections() {
    for enabled in [false, true] {
        for configured in [false, true] {
            let mut policy = migration_policy(enabled && configured);
            policy.resources = vec![Record {
                id: "7df551f7-ddd1-52b0-8ba6-d09be35dbccc".into(),
                type_hash: 0x647628d6,
                original: "gpc".into(),
                original_sha256: "2".repeat(64),
                applied_sha256: "72c086eecc5a1852c50367109c0126f86102a1e472b7c07546fa4885c078ebe8"
                    .into(),
            }];
            let migration = policy.migration.as_mut().unwrap();
            migration.enabled = enabled;
            migration.configured_extended_zoom = configured;
            migration.effective_zoom = enabled && configured;
            policy.layout_migration = Some(layout_migration::Contract {});
            assert!(validate_migration(&policy).is_ok());
            policy.migration.as_mut().unwrap().effective_zoom = !(enabled && configured);
            assert!(validate_migration(&policy).is_err());
        }
    }

    let mut missing_admission = migration_policy(false);
    missing_admission.layout_migration = Some(layout_migration::Contract {});
    missing_admission.migration = None;
    assert!(validate_migration(&missing_admission).is_err());

    let mut old_postimage = migration_policy(true);
    old_postimage.resources = vec![Record {
        id: "7df551f7-ddd1-52b0-8ba6-d09be35dbccc".into(),
        type_hash: 0x647628d6,
        original: "gpc".into(),
        original_sha256: "2".repeat(64),
        applied_sha256: "bf43423e8f39870cce1cdbbdfcbef76337367f889c93bf782019155d1fb5a51b".into(),
    }];
    old_postimage.layout_migration = Some(layout_migration::Contract {});
    assert!(validate_migration(&old_postimage).is_err());

    let mut wrong_postimage = migration_policy(true);
    wrong_postimage.resources = vec![Record {
        id: "7df551f7-ddd1-52b0-8ba6-d09be35dbccc".into(),
        type_hash: 0x647628d6,
        original: "gpc".into(),
        original_sha256: "2".repeat(64),
        applied_sha256: "0".repeat(64),
    }];
    wrong_postimage.layout_migration = Some(layout_migration::Contract {});
    assert!(validate_migration(&wrong_postimage).is_err());
}

#[test]
fn migration_rebinds_194_owned_outputs_to_a_verified_258_output_baseline() -> Result<()> {
    let temp = TempRoot::new("migration");
    let root = fs::canonicalize(&temp.0)?;
    fs::write(root.join("enshrouded.kfc"), [])?;
    fs::write(root.join("enshrouded.kfc_resources"), [])?;
    let registry = TypeRegistry::default();
    let writer = KFCWriter::<KFCFile, _>::new(&root, STEM, &registry, zoom::GENERATION)?;
    writer.finalize()?;
    let empty_dat = root.join("enshrouded_000.dat");
    if empty_dat.exists() {
        fs::remove_file(empty_dat)?;
    }

    let mut policy = migration_policy(true);
    let mut full_outputs = full_reviewed_outputs();
    for name in ["enshrouded.kfc", "enshrouded.kfc_resources"] {
        full_outputs.insert(name.into(), stamp(&root.join(name))?);
    }
    policy.composition.outputs = full_outputs.clone();
    policy.migration.as_mut().unwrap().outputs = full_outputs.clone();
    for name in full_outputs
        .keys()
        .filter(|name| !matches!(name.as_str(), "enshrouded.kfc" | "enshrouded.kfc_resources"))
    {
        fs::write(root.join(name), reviewed_output_bytes(name))?;
    }
    let state_path = root.join("current-assets-state.json");
    let migration = policy.migration.as_ref().unwrap();
    let state = Prepared {
        schema_version: 1,
        root: root.to_string_lossy().into(),
        fingerprint: migration.fingerprint.clone(),
        enabled: true,
        configured_extended_zoom: None,
        effective_zoom: None,
        previous_state_sha256: None,
        outputs: owned_194(&migration.outputs),
        target: None,
        resource_contribution: None,
    };
    write_json(&state_path, &state)?;
    let original_receipt_hash = stamp(&state_path)?.1;
    let migration = Migration {
        state_sha256: original_receipt_hash.clone(),
        fingerprint: migration.fingerprint.clone(),
        enabled: migration.enabled,
        configured_extended_zoom: migration.configured_extended_zoom,
        effective_zoom: migration.effective_zoom,
        outputs: migration.outputs.clone(),
    };
    validate_migration_identity(&state_path, &state, &migration, true, true)?;
    let mut wrong_pin = migration.clone();
    wrong_pin.state_sha256 = "0".repeat(64);
    assert!(validate_migration_identity(&state_path, &state, &wrong_pin, true, true).is_err());
    assert!(validate_migration_identity(&state_path, &state, &migration, true, false).is_err());
    let baseline = validate_migration_source(
        &root,
        &state_path,
        &state,
        &policy,
        &migration,
        true,
        true,
        &"9".repeat(64),
        false,
    )?;
    assert_eq!(state.outputs.len(), 194);
    assert_eq!(baseline.outputs.len(), 258);
    assert_eq!(stamp(&state_path)?.1, original_receipt_hash);

    let mut wrong_owned_stamp = state.clone();
    wrong_owned_stamp
        .outputs
        .get_mut("enshrouded_001.dat")
        .unwrap()
        .1 = "1".repeat(64);
    assert!(reviewed_migration_baseline(&root, &policy, &wrong_owned_stamp).is_err());

    fs::write(root.join("enshrouded_256.dat"), b"changed unowned file")?;
    assert!(reviewed_migration_baseline(&root, &policy, &state).is_err());
    assert_eq!(stamp(&state_path)?.1, original_receipt_hash);
    Ok(())
}
