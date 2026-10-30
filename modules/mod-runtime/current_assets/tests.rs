use super::*;

#[test]
fn schema_two_records_every_configured_combination() -> Result<()> {
    for enabled in [false, true] {
        for configured in [false, true] {
            let root = Path::new("E:/Scratch/zoom-schema-test");
            let state: Prepared = serde_json::from_value(serde_json::json!({
                "schemaVersion":2,"root":root,"fingerprint":"a".repeat(64),
                "enabled":enabled,"configuredExtendedZoom":configured,
                "effectiveZoom":enabled && configured,
                "previousStateSha256":"d".repeat(64),
                "outputs":{"enshrouded.kfc":[1,"b".repeat(64)],"enshrouded.kfc_resources":[1,"c".repeat(64)]}
            }))?;
            validate_shape(root, &state)?;
        }
    }
    Ok(())
}

#[test]
fn legacy_receipt_remains_readable() -> Result<()> {
    let root = Path::new("E:/Scratch/zoom-schema-test");
    let state: Prepared = serde_json::from_value(serde_json::json!({
        "schemaVersion":1,"root":root,"fingerprint":"a".repeat(64),"enabled":true,
        "outputs":{"enshrouded.kfc":[1,"b".repeat(64)],"enshrouded.kfc_resources":[1,"c".repeat(64)]}
    }))?;
    validate_shape(root, &state)
}

fn bundle(extended: bool) -> kfc::resource::value::Value {
    use kfc::resource::value::Value as V;
    fn obj(entries: Vec<(&str, V)>) -> V {
        V::Struct(Box::new(
            entries.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        ))
    }
    let scales = if extended {
        [16., 8., 4., 1., 0.35f32]
    } else {
        [16., 8., 4., 2., 0.7f32]
    };
    obj(vec![
        ("opaque", V::UInt(u64::MAX)),
        ("optional", V::None),
        (
            "menu",
            obj(vec![(
                "map",
                obj(vec![(
                    "zoomConfigs",
                    V::Array(
                        scales
                            .into_iter()
                            .map(|v| {
                                obj(vec![
                                    ("zoomScale", V::Float(v as f64)),
                                    ("isolineConfigs", V::Array(vec![V::UInt(u64::MAX)])),
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
fn all_sixteen_transitions_preserve_every_other_typed_value() -> Result<()> {
    for before_parent in [false, true] {
        for before_configured in [false, true] {
            for parent in [false, true] {
                for configured in [false, true] {
                    let input = bundle(before_parent && before_configured);
                    let result = zoom::transform(
                        &input,
                        before_parent && before_configured,
                        parent && configured,
                    )?;
                    assert_eq!(result, bundle(parent && configured));
                    assert_eq!(
                        zoom::transform(&result, parent && configured, parent && configured)?,
                        result
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn zoom_conflicting_preimage_is_refused() {
    assert!(zoom::transform(&bundle(true), false, true).is_err());
    assert!(zoom::transform(&bundle(false), true, false).is_err());
}

#[test]
fn malformed_selection_states_are_rejected() -> Result<()> {
    let root = Path::new("E:/Scratch/zoom-schema-test");
    let base = serde_json::json!({"schemaVersion":2,"root":root,"fingerprint":"a".repeat(64),"enabled":false,
        "configuredExtendedZoom":true,"effectiveZoom":false,
        "previousStateSha256":"d".repeat(64),
        "outputs":{"enshrouded.kfc":[1,"b".repeat(64)],"enshrouded.kfc_resources":[1,"c".repeat(64)]}});
    for (key, value) in [
        ("schemaVersion", 3.into()),
        ("effectiveZoom", true.into()),
        ("configuredExtendedZoom", serde_json::Value::Null),
    ] {
        let mut bad = base.clone();
        bad[key] = value;
        assert!(
            serde_json::from_value::<Prepared>(bad)
                .and_then(|s| validate_shape(root, &s).map_err(serde::de::Error::custom))
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn only_declared_boolean_value_is_removed_from_identity() -> Result<()> {
    let base = serde_json::json!({"settings":{"ExtendedZoom":{"value":true,"applyAt":"restart","restartRequired":true,"assetPreparation":true,"label":"Extended zoom"}}});
    let mut on = base.clone();
    normalize_zoom_setting(&mut on)?;
    let mut off = base.clone();
    off["settings"]["ExtendedZoom"]["value"] = false.into();
    normalize_zoom_setting(&mut off)?;
    assert_eq!(on, off);
    let mut metadata = base.clone();
    metadata["settings"]["ExtendedZoom"]["label"] = "Changed".into();
    normalize_zoom_setting(&mut metadata)?;
    assert_ne!(metadata, on);
    let mut unknown = base.clone();
    unknown["settings"]["Other"] = true.into();
    assert!(normalize_zoom_setting(&mut unknown).is_err());
    let mut invalid = base.clone();
    invalid["settings"]["ExtendedZoom"]["value"] = "true".into();
    assert!(normalize_zoom_setting(&mut invalid).is_err());
    let mut timing = base.clone();
    timing["settings"]["ExtendedZoom"]["applyAt"] = "live".into();
    assert!(normalize_zoom_setting(&mut timing).is_err());
    Ok(())
}

#[path = "integration_tests.rs"]
mod integration;

#[test]
fn successor_pending_cannot_publish_without_before_state_binding() -> Result<()> {
    let root = PathBuf::from(format!(
        "E:/Scratch/hdmap-zoom-pending-binding-{}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("shroudforge/config"))?;
    let root = fs::canonicalize(root)?;
    fs::write(root.join("enshrouded.kfc"), b"synthetic-index")?;
    fs::write(
        root.join("enshrouded.kfc_resources"),
        b"synthetic-resources",
    )?;
    let mut outputs = BTreeMap::new();
    for n in ["enshrouded.kfc", "enshrouded.kfc_resources"] {
        outputs.insert(n, stamp(&root.join(n))?);
    }
    let receipt = serde_json::json!({"schemaVersion":2,"root":root,"fingerprint":"a".repeat(64),"enabled":true,
        "configuredExtendedZoom":true,"effectiveZoom":true,"outputs":outputs});
    fs::write(root.join(STATE), b"unrelated-current-state")?;
    fs::write(root.join(PENDING), serde_json::to_vec(&receipt)?)?;
    assert!(
        recover(&root, &"a".repeat(64), 2).is_err(),
        "unbound pending overwrote unrelated current state"
    );
    assert_eq!(fs::read(root.join(STATE))?, b"unrelated-current-state");
    assert!(root.join(PENDING).exists());
    Ok(())
}

// Typed forward-recovery and refusal coverage is in recovery_semantics_tests.rs.
