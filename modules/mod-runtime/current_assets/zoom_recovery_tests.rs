use super::*;

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "shroudforge-zoom-recovery-{}",
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

fn state(
    root: &Path,
    schema_version: u32,
    fingerprint: &str,
    enabled: bool,
    configured_zoom: Option<bool>,
    effective_zoom: Option<bool>,
    previous_state_sha256: Option<String>,
    outputs: BTreeMap<String, Stamp>,
) -> Prepared {
    Prepared {
        schema_version,
        root: root.to_string_lossy().into(),
        fingerprint: fingerprint.into(),
        enabled,
        configured_extended_zoom: configured_zoom,
        effective_zoom,
        previous_state_sha256,
        outputs,
        target: None,
        resource_contribution: None,
    }
}

fn reviewed_output_bytes(name: &str) -> Vec<u8> {
    format!("reviewed-current-composition:{name}").into_bytes()
}

fn reviewed_outputs() -> BTreeMap<String, Stamp> {
    let mut names = vec![
        "enshrouded.kfc".to_owned(),
        "enshrouded.kfc_resources".to_owned(),
    ];
    names.extend((1..=256).map(|index| format!("enshrouded_{index:03}.dat")));
    names
        .into_iter()
        .map(|name| {
            let bytes = reviewed_output_bytes(&name);
            (name, (bytes.len() as u64, hash(&bytes)))
        })
        .collect()
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

fn write_reviewed_outputs(root: &Path, outputs: &BTreeMap<String, Stamp>) -> Result<()> {
    for name in outputs.keys() {
        fs::write(root.join(name), reviewed_output_bytes(name))?;
    }
    Ok(())
}

fn migration_policy(
    outputs: BTreeMap<String, Stamp>,
    previous: &Prepared,
    previous_hash: &str,
) -> Policy {
    Policy {
        schema_version: 2,
        mod_id: "esm-real-map".into(),
        package: "mods/esm-real-map".into(),
        generation: zoom::GENERATION.into(),
        executable_sha256: "a".repeat(64),
        resources: Vec::new(),
        retained_contents: Vec::new(),
        composition: Composition {
            fingerprint: "f".repeat(64),
            outputs: outputs.clone(),
            source_fingerprint: None,
            baseline_enabled: true,
            baseline_legacy_pickaxe_reach_migrated: false,
            excluded_packages: Vec::new(),
        },
        legacy_pickaxe_reach: Content {
            file: "legacy-reach.json".into(),
            sha256: "b".repeat(64),
        },
        zoom: Some(zoom::Contract {
            registry_sha256: "c".repeat(64),
        }),
        migration: Some(Migration {
            state_sha256: previous_hash.into(),
            fingerprint: previous.fingerprint.clone(),
            enabled: true,
            configured_extended_zoom: true,
            effective_zoom: true,
            outputs,
        }),
        layout_migration: None,
        resource_contribution: None,
    }
}

#[test]
fn interrupted_194_to_258_adoption_is_pinned_and_removed_contract_refuses_recovery() -> Result<()> {
    let temp = TempRoot::new();
    let root = fs::canonicalize(&temp.0)?;
    fs::create_dir_all(root.join("shroudforge/config"))?;
    let full_outputs = reviewed_outputs();
    write_reviewed_outputs(&root, &full_outputs)?;
    let previous_outputs = owned_194(&full_outputs);
    let previous = state(
        &root,
        2,
        &"a".repeat(64),
        true,
        Some(true),
        Some(true),
        Some("b".repeat(64)),
        previous_outputs.clone(),
    );
    write_json(&joined(&root, STATE)?, &previous)?;
    let predecessor_hash = stamp(&joined(&root, STATE)?)?.1;
    let mut policy = migration_policy(full_outputs.clone(), &previous, &predecessor_hash);
    validate_migration(&policy)?;

    let stage = transaction::directory(&root, STEM);
    fs::create_dir_all(&stage)?;
    fs::write(
        stage.join("generated-inputs.json"),
        serde_json::to_vec(&(
            1u32,
            root.to_string_lossy().to_string(),
            stamp(&root.join("enshrouded.kfc"))?,
            stamp(&root.join("enshrouded.kfc_resources"))?,
        ))?,
    )?;
    let next_index = b"new index";
    let next_resources = b"new resources";
    fs::write(stage.join("enshrouded.kfc"), next_index)?;
    fs::write(stage.join("enshrouded.kfc_resources"), next_resources)?;
    let mut next_outputs = full_outputs.clone();
    next_outputs.insert(
        "enshrouded.kfc".into(),
        stamp(&stage.join("enshrouded.kfc"))?,
    );
    next_outputs.insert(
        "enshrouded.kfc_resources".into(),
        stamp(&stage.join("enshrouded.kfc_resources"))?,
    );
    let next = state(
        &root,
        2,
        &"c".repeat(64),
        true,
        Some(true),
        Some(true),
        Some(predecessor_hash.clone()),
        next_outputs.clone(),
    );
    let records: Vec<_> = ["enshrouded.kfc_resources", "enshrouded.kfc"]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                previous_outputs.get(name).cloned(),
                next_outputs.get(name).unwrap().clone(),
            )
        })
        .collect();
    fs::write(
        stage.join("generated-pending.json"),
        serde_json::to_vec(&(1u32, &next.root, &records))?,
    )?;
    fs::remove_file(root.join("enshrouded.kfc_resources"))?;
    fs::rename(
        stage.join("enshrouded.kfc_resources"),
        root.join("enshrouded.kfc_resources"),
    )?;
    write_json(&joined(&root, PENDING)?, &next)?;

    validate_pending_binding(&root, &next, &"c".repeat(64), true, true, &policy)?;
    validate_pending(&root, &stage, &next)?;
    let (index, resources) = afterimage_paths(&root, &stage, &next)?;
    assert_eq!(index, stage.join("enshrouded.kfc"));
    assert_eq!(resources, root.join("enshrouded.kfc_resources"));
    assert_eq!(previous_outputs.len(), 194);
    assert_eq!(next.outputs.len(), 258);

    transaction::recover(&root, STEM)?;
    assert!(!stage.exists());
    verify_outputs(&root, &next)?;

    assert!(validate_pending_binding(&root, &next, &"d".repeat(64), true, true, &policy).is_err());
    assert!(validate_pending_binding(&root, &next, &"c".repeat(64), false, true, &policy).is_err());
    assert!(validate_pending_binding(&root, &next, &"c".repeat(64), true, false, &policy).is_err());

    let mut ordinary = next.clone();
    ordinary.outputs = previous_outputs.clone();
    for name in ["enshrouded.kfc", "enshrouded.kfc_resources"] {
        ordinary
            .outputs
            .insert(name.into(), next.outputs.get(name).unwrap().clone());
    }
    write_json(&joined(&root, PENDING)?, &ordinary)?;
    assert!(
        validate_pending_binding(&root, &ordinary, &"c".repeat(64), true, true, &policy).is_err()
    );
    let mut ordinary_policy = migration_policy(full_outputs.clone(), &previous, &predecessor_hash);
    ordinary_policy.migration = None;
    validate_pending_binding(
        &root,
        &ordinary,
        &"c".repeat(64),
        true,
        true,
        &ordinary_policy,
    )?;

    let mut unowned_stamp_mismatch = next.clone();
    unowned_stamp_mismatch
        .outputs
        .get_mut("enshrouded_256.dat")
        .unwrap()
        .1 = "0".repeat(64);
    write_json(&joined(&root, PENDING)?, &unowned_stamp_mismatch)?;
    assert!(
        validate_pending_binding(
            &root,
            &unowned_stamp_mismatch,
            &"c".repeat(64),
            true,
            true,
            &policy,
        )
        .is_err()
    );
    write_json(&joined(&root, PENDING)?, &next)?;

    let unowned_path = root.join("enshrouded_256.dat");
    let unowned_before = fs::read(&unowned_path)?;
    fs::write(&unowned_path, b"changed unowned physical output")?;
    assert!(validate_pending_binding(&root, &next, &"c".repeat(64), true, true, &policy).is_err());
    fs::write(&unowned_path, unowned_before)?;
    fs::remove_file(&unowned_path)?;
    assert!(validate_pending_binding(&root, &next, &"c".repeat(64), true, true, &policy).is_err());
    fs::write(&unowned_path, reviewed_output_bytes("enshrouded_256.dat"))?;

    let unexpected_path = root.join("enshrouded_999.dat");
    fs::write(&unexpected_path, b"unexpected output")?;
    assert!(validate_pending_binding(&root, &next, &"c".repeat(64), true, true, &policy).is_err());
    fs::remove_file(unexpected_path)?;

    fs::write(&root.join("enshrouded.kfc_resources"), b"stale after-image")?;
    assert!(afterimage_paths(&root, &stage, &next).is_err());
    fs::write(&root.join("enshrouded.kfc_resources"), next_resources)?;

    validate_pending_binding(&root, &next, &"c".repeat(64), true, true, &policy)?;
    verify_outputs(&root, &next)?;
    let (index, resources) = afterimage_paths(&root, &transaction::directory(&root, STEM), &next)?;
    assert_eq!(index, root.join("enshrouded.kfc"));
    assert_eq!(resources, root.join("enshrouded.kfc_resources"));

    assert_eq!(stamp(&joined(&root, STATE)?)?.1, predecessor_hash);
    let prior_state_bytes = fs::read(joined(&root, STATE)?)?;
    let prior_pending_bytes = fs::read(joined(&root, PENDING)?)?;
    let prior_root_index = fs::read(root.join("enshrouded.kfc"))?;
    let prior_root_resources = fs::read(root.join("enshrouded.kfc_resources"))?;
    policy.zoom = None;
    policy.migration = None;
    assert!(recover_without_zoom_contract(&root).is_err());
    assert_eq!(fs::read(joined(&root, STATE)?)?, prior_state_bytes);
    assert_eq!(fs::read(joined(&root, PENDING)?)?, prior_pending_bytes);
    assert_eq!(fs::read(root.join("enshrouded.kfc"))?, prior_root_index);
    assert_eq!(
        fs::read(root.join("enshrouded.kfc_resources"))?,
        prior_root_resources
    );
    assert!(!stage.exists());
    Ok(())
}

#[test]
fn interrupted_layout_transition_with_same_inventory_requires_exact_layout_contract() -> Result<()>
{
    let temp = TempRoot::new();
    let root = fs::canonicalize(&temp.0)?;
    fs::create_dir_all(root.join("shroudforge/config"))?;
    let full_outputs = reviewed_outputs();
    write_reviewed_outputs(&root, &full_outputs)?;
    let previous = state(
        &root,
        2,
        &"a".repeat(64),
        true,
        Some(true),
        Some(true),
        Some("b".repeat(64)),
        full_outputs.clone(),
    );
    write_json(&joined(&root, STATE)?, &previous)?;
    let predecessor_hash = stamp(&joined(&root, STATE)?)?.1;
    let mut policy = migration_policy(full_outputs.clone(), &previous, &predecessor_hash);
    policy.layout_migration = Some(layout_migration::Contract {});
    policy.resources = vec![Record {
        id: "7df551f7-ddd1-52b0-8ba6-d09be35dbccc".into(),
        type_hash: 0x647628d6,
        original: "gpc".into(),
        original_sha256: "2".repeat(64),
        applied_sha256: "72c086eecc5a1852c50367109c0126f86102a1e472b7c07546fa4885c078ebe8".into(),
    }];
    validate_migration(&policy)?;

    let stage = transaction::directory(&root, STEM);
    fs::create_dir_all(&stage)?;
    fs::write(
        stage.join("generated-inputs.json"),
        serde_json::to_vec(&(
            1u32,
            root.to_string_lossy().to_string(),
            stamp(&root.join("enshrouded.kfc"))?,
            stamp(&root.join("enshrouded.kfc_resources"))?,
        ))?,
    )?;
    let next_index = b"layout-upgraded index";
    let next_resources = b"layout-upgraded resources";
    fs::write(stage.join("enshrouded.kfc"), next_index)?;
    fs::write(stage.join("enshrouded.kfc_resources"), next_resources)?;
    let mut next_outputs = full_outputs.clone();
    next_outputs.insert(
        "enshrouded.kfc".into(),
        stamp(&stage.join("enshrouded.kfc"))?,
    );
    next_outputs.insert(
        "enshrouded.kfc_resources".into(),
        stamp(&stage.join("enshrouded.kfc_resources"))?,
    );
    let next = state(
        &root,
        2,
        &"c".repeat(64),
        true,
        Some(true),
        Some(true),
        Some(predecessor_hash),
        next_outputs.clone(),
    );
    let records: Vec<_> = ["enshrouded.kfc_resources", "enshrouded.kfc"]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                previous.outputs.get(name).cloned(),
                next_outputs.get(name).unwrap().clone(),
            )
        })
        .collect();
    fs::write(
        stage.join("generated-pending.json"),
        serde_json::to_vec(&(1u32, &next.root, &records))?,
    )?;
    fs::remove_file(root.join("enshrouded.kfc"))?;
    fs::rename(stage.join("enshrouded.kfc"), root.join("enshrouded.kfc"))?;
    write_json(&joined(&root, PENDING)?, &next)?;

    validate_pending_binding(&root, &next, &"c".repeat(64), true, true, &policy)?;
    let mut wrong_selection = next.clone();
    wrong_selection.effective_zoom = Some(false);
    assert!(
        validate_pending_binding(
            &root,
            &wrong_selection,
            &"c".repeat(64),
            true,
            true,
            &policy
        )
        .is_err()
    );
    let mut missing_layout = policy;
    missing_layout.layout_migration = None;
    assert!(
        validate_pending_binding(&root, &next, &"c".repeat(64), true, true, &missing_layout)
            .is_err()
    );
    Ok(())
}

#[test]
fn schema_one_state_still_uses_legacy_recovery_without_a_zoom_contract() -> Result<()> {
    let temp = TempRoot::new();
    let root = fs::canonicalize(&temp.0)?;
    fs::create_dir_all(root.join("shroudforge/config"))?;
    let outputs = reviewed_outputs();
    write_reviewed_outputs(&root, &outputs)?;
    let previous = state(
        &root,
        1,
        &"a".repeat(64),
        true,
        None,
        None,
        None,
        outputs.clone(),
    );
    let next = state(&root, 1, &"b".repeat(64), true, None, None, None, outputs);
    write_json(&joined(&root, STATE)?, &previous)?;
    write_json(&joined(&root, PENDING)?, &next)?;

    recover_without_zoom_contract(&root)?;

    assert_eq!(
        serde_json::to_value(read_json::<Prepared>(&joined(&root, STATE)?)?)?,
        serde_json::to_value(next)?
    );
    assert!(!joined(&root, PENDING)?.exists());
    Ok(())
}
