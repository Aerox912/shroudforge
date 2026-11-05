use super::*;
use prepared_source_admission::{ProfilePolicy, Stamp as SourceStamp};
use serde_json::json;

const CURRENT_KFC_GENERATION: &str = "1287369|^/game38/branches/release_1.0|2026-11-04T11:17:54.051725Z";

#[test]
fn effective_environment_uses_sealed_windows_roots_and_reports_concrete_errors() {
    for invalid_manifest in [false, true] {
        let environment = PathBuf::from(r"E:\Scratch\sf178-effective-environment-root-tests").join(format!(
            "{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(environment.join("mods/fixture")).unwrap();
        let body = if invalid_manifest {b"{".as_slice()} else {br#"{"id":"fixture","name":"Fixture","version":"1.0.0","capabilities":["patch"]}"#.as_slice()};
        fs::write(environment.join("mods/fixture/mod.json"),body).unwrap();
        let extension=br#"{"schemaVersion":1,"enabled":true,"targets":["client"]}"#;
        fs::write(environment.join("mods/fixture/extended.mod.json"),extension).unwrap();
        let mut r = request();
        r.sources.environment_root = environment.clone();
        r.sources.raw_files = [("mods/fixture/mod.json".into(),SourceStamp{bytes:body.len() as u64,sha256:format!("{:x}",Sha256::digest(body))})].into();
        r.sources.raw_files.insert("mods/fixture/extended.mod.json".into(),SourceStamp{bytes:extension.len() as u64,sha256:format!("{:x}",Sha256::digest(extension))});
        r.sources.entries = ["","mods","mods/fixture"].map(String::from).into();
        r.sources.entries.extend(fs::read_dir(environment.join("mods/fixture")).unwrap().map(|entry|format!("mods/fixture/{}",entry.unwrap().file_name().to_str().unwrap())));
        r.sources.raw_inventory_sha256 = r.sources.inventory_digest(false).unwrap();
        r.sources.effective_inventory_sha256 = r.sources.inventory_digest(true).unwrap();
        let limits = ReadLimits{max_bytes:65536,max_calls:1000,final_reserve_bytes:r.sources.required_final_bytes().unwrap(),final_reserve_calls:r.sources.required_final_calls().unwrap()};
        let mut lease = SourceLease::admit(r.sources,limits).unwrap();
        let canonical = super::super::root(&environment).unwrap();
        assert_ne!(canonical,environment);
        let error = prepared_mod_environment(&canonical,&lease).err().unwrap();
        assert!(format!("{error:#}").contains("prepared package root mismatch"));
        match prepared_mod_environment(&environment,&lease) {
            Ok(env) => {
                assert!(!invalid_manifest);
                let (plan,issues)=env.plan_report(false,shroudforge_api::API_VERSION);
                assert!(issues.is_empty());
                assert_eq!(plan.iter().map(|p|p.info().id.as_str()).collect::<Vec<_>>(),vec!["fixture"]);
            },
            Err(error) => {
                assert!(invalid_manifest);
                let detail=format!("{error:#}");
                assert!(detail.contains("fixture")&&detail.contains("JSON error"),"{detail}");
            }
        }
        assert!(lease.finalize().unwrap().verified);
    }
}

fn request() -> EffectiveRequest {
    let sha = "a".repeat(64);
    let environment =
        PathBuf::from("E:/Scratch/nonexistent-effective-protocol-fixture/environment");
    let name = "mods/fixture/mod.json".to_owned();
    let raw = SourceStamp {
        bytes: 1,
        sha256: sha.clone(),
    };
    let mut sources = SourcePlan {
        schema: 1,
        kind: "prepared_effective_source_plan_v1".into(),
        environment_root: environment.clone(),
        raw_files: BTreeMap::from([
            (name.clone(), raw.clone()),
            ("manifest".into(), raw),
            (
                "approval".into(),
                SourceStamp {
                    bytes: 1,
                    sha256: "b".repeat(64),
                },
            ),
        ]),
        entries: vec![
            "".into(),
            "mods".into(),
            "mods/fixture".into(),
            name.clone(),
            "manifest".into(),
            "approval".into(),
        ],
        package_roots: vec!["mods/fixture".into()],
        replacements: vec![],
        profile_policy: ProfilePolicy::RejectAmbientProfiles,
        allow_unknown_manifest_fields: true,
        raw_inventory_sha256: String::new(),
        effective_inventory_sha256: String::new(),
    };
    sources.raw_inventory_sha256 = sources.inventory_digest(false).unwrap();
    sources.effective_inventory_sha256 = sources.inventory_digest(true).unwrap();
    let stamp = Stamp {
        bytes: 1,
        sha256: sha,
    };
    let environment_files = sources
        .raw_files
        .iter()
        .map(|(name, s)| {
            (
                name.clone(),
                Stamp {
                    bytes: s.bytes,
                    sha256: s.sha256.clone(),
                },
            )
        })
        .collect();
    let registry_evidence_files = sources
        .raw_files
        .iter()
        .filter(|(n, _)| !n.starts_with("mods/"))
        .map(|(n, s)| (n.clone(), s.clone()))
        .collect();
    EffectiveRequest {
        schema: 3, kind: "sf178_prepared_effective_source_request_v1".into(),
        base: Request {
            schema: 2, target_root: "E:/Scratch/nonexistent-effective-protocol-fixture/source".into(),
            asset_input_root: "E:/Scratch/nonexistent-effective-protocol-fixture/source".into(),
            environment_root: environment, output_root: "E:/Scratch/nonexistent-effective-protocol-fixture/output".into(),
            selected_ids: vec!["fixture".into()],
            inputs: ["enshrouded.exe", "enshrouded.kfc", "enshrouded.kfc_resources"].into_iter().map(|n| (n.into(),stamp.clone())).collect(),
            environment_files,
            external_registry: Some(serde_json::from_value(json!({"manifest":{"path":"manifest","bytes":1,"sha256":"a".repeat(64)},"producer_use_approval":{"path":"approval","bytes":1,"sha256":"b".repeat(64)}})).unwrap()),
        }, sources,registry_evidence_files,
        source_read_limits: ReadLimits {max_bytes:100, max_calls:100, final_reserve_bytes:1, final_reserve_calls:1},
        expected_load_order: vec!["fixture".into()], expected_generation:CURRENT_KFC_GENERATION.into(),
    }
}

#[test]
fn effective_full_kfc_generation_is_preserved_without_truncation() {
    let original = request();
    original.validate().unwrap();
    let roundtrip: EffectiveRequest = serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    assert_eq!(roundtrip.expected_generation, CURRENT_KFC_GENERATION);
    assert_eq!(original.canonical_sha256().unwrap(), roundtrip.canonical_sha256().unwrap());
    for invalid in [String::new(), "x".repeat(129), "generation\nmetadata".into(), "g\u{00e9}n".into()] {
        let mut changed = request(); changed.expected_generation = invalid;
        assert!(changed.validate().is_err());
    }
    let mut truncated = request(); truncated.expected_generation = "1287369".into();
    assert_ne!(original.canonical_sha256().unwrap(), truncated.canonical_sha256().unwrap());
}

#[test]
fn effective_protocol_rejects_legacy_schema_graph_order_and_generation_mismatches_before_io() {
    let good = request();
    good.validate().unwrap();
    assert!(
        serde_json::from_value::<EffectiveRequest>(serde_json::to_value(&good.base).unwrap())
            .is_err()
    );
    for mode in 0..8 {
        let mut r = request();
        match mode {
            0 => r.schema = 2,
            1 => r.base.external_registry = None,
            2 => {
                r.base.environment_files.clear();
            }
            3 => r.expected_load_order.push("fixture".into()),
            4 => r.expected_load_order = vec!["other".into()],
            5 => {
                r.base.selected_ids = vec!["esm.auto.loot".into()];
                r.expected_load_order = r.base.selected_ids.clone();
            }
            6 => r.expected_generation = "generation\nmetadata".into(),
            7 => r.base.inputs.get_mut("enshrouded.kfc").unwrap().bytes = 0,
            _ => unreachable!(),
        }
        assert!(r.validate().is_err(), "case{mode}");
    }
    let mut changed = request();
    changed.expected_generation.push('2');
    assert_ne!(
        good.canonical_sha256().unwrap(),
        changed.canonical_sha256().unwrap()
    );
}

fn committed_source() -> Value {
    json!({"schema_version":1,"source_root":{"repository":"shroudforge","commit":"1".repeat(40),"tree":"2".repeat(40),"gitlinks":
        (["kfc-parser","modules/game-runtime/native/json","modules/hotkeys/SDL"].map(|path| json!({"path":path,"commit":"3".repeat(40),"tree":"4".repeat(40)})))},
        "lockfiles":(["Cargo.lock","dev/hdmap-prepared-generator/Cargo.lock"].map(|path|json!({"path":path,"sha256":"a".repeat(64)})))})
}

fn refresh_graph(request: &mut EffectiveRequest) {
    request.sources.raw_inventory_sha256 = request.sources.inventory_digest(false).unwrap();
    request.sources.effective_inventory_sha256 = request.sources.inventory_digest(true).unwrap();
    request.base.environment_files = request
        .sources
        .raw_files
        .iter()
        .map(|(n, s)| {
            (
                n.clone(),
                Stamp {
                    bytes: s.bytes,
                    sha256: s.sha256.clone(),
                },
            )
        })
        .collect();
}

#[test]
fn selected_package_projection_and_complete_evidence_control_graph_are_distinct_exact_joins() {
    let mut r = request();
    let control = "shroudforge/config/modloader-config.json";
    r.sources.raw_files.insert(
        control.into(),
        SourceStamp {
            bytes: 1,
            sha256: "c".repeat(64),
        },
    );
    r.sources.entries.extend([
        "shroudforge".into(),
        "shroudforge/config".into(),
        control.into(),
    ]);
    refresh_graph(&mut r);
    r.validate().unwrap();
    // Frozen selected source origin and the expanded physical operation graph
    // may have different roots; unchanged D1 references retain the old origin.
    let origin = "E:/Scratch/frozen-selected-package-origin";
    let package = "mods/fixture/mod.json";
    let s = &r.sources.raw_files[package];
    let base = json!({"environmentRoot":origin,"environmentFiles":{package:{"bytes":s.bytes,"sha256":s.sha256}}});
    let successor = json!({"overrideCount":0,"effectiveEnvironmentFiles":{package:{"kind":"UNCHANGED_BASE_REFERENCE","root":origin,"relativePath":package,"bytes":s.bytes,"sha256":s.sha256}}});
    validate_package_projection(&r, &base, &successor).unwrap();
    assert_eq!(r.sources.raw_files.len(), 4);
    for mode in 0..5 {
        let mut bad: EffectiveRequest =
            serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        match mode {
            0 => {
                bad.sources.raw_files.remove("approval");
                bad.sources.entries.retain(|n| n != "approval");
            }
            1 => {
                bad.sources.raw_files.get_mut("manifest").unwrap().sha256 = "d".repeat(64);
            }
            2 => {
                bad.sources.raw_files.insert(
                    "unrelated-control".into(),
                    SourceStamp {
                        bytes: 1,
                        sha256: "e".repeat(64),
                    },
                );
                bad.sources.entries.push("unrelated-control".into());
            }
            3 => {
                bad.registry_evidence_files.remove("manifest");
            }
            4 => {
                bad.sources.raw_files.get_mut(package).unwrap().sha256 = "f".repeat(64);
            }
            _ => unreachable!(),
        }
        refresh_graph(&mut bad);
        assert!(
            bad.validate()
                .and_then(|()| validate_package_projection(&bad, &base, &successor))
                .is_err(),
            "case{mode}"
        );
    }
    let mut bad = successor;
    bad["effectiveEnvironmentFiles"][package]["root"] = json!(r.sources.environment_root);
    assert!(validate_package_projection(&r, &base, &bad).is_err());
}

#[test]
fn admission_refusal_serializes_partial_finalization_while_handles_remain_held_and_preserves_record_failure()
 {
    for blocked_record in [false, true] {
        let root =
            PathBuf::from("E:/Scratch/sf178-effective-admission-refusal-tests").join(format!(
                "{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let environment = root.join("environment");
        let evidence = root.join("operation");
        fs::create_dir_all(environment.join("mods/fixture")).unwrap();
        fs::create_dir(&evidence).unwrap();
        let mut r = request();
        r.sources.environment_root = environment.clone();
        for (name, stamp) in &mut r.sources.raw_files {
            fs::write(environment.join(name), b"x").unwrap();
            stamp.sha256 = format!("{:x}", Sha256::digest(b"x"));
        }
        // Inventory/order is captured from the actual tiny synthetic graph.
        fn visit(root: &Path, path: &Path, names: &mut Vec<String>) {
            for e in fs::read_dir(path).unwrap() {
                let p = e.unwrap().path();
                names.push(
                    p.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                );
                if p.is_dir() {
                    visit(root, &p, names);
                }
            }
        }
        r.sources.entries = vec![String::new()];
        visit(&environment, &environment, &mut r.sources.entries);
        r.sources.raw_files.get_mut("approval").unwrap().sha256 = "0".repeat(64);
        refresh_graph(&mut r);
        if blocked_record {
            fs::write(
                evidence.join("REFUSAL.json"),
                b"preexisting refusal preserved",
            )
            .unwrap();
        }
        let limits = ReadLimits {
            max_bytes: 1024,
            max_calls: 100,
            final_reserve_bytes: r.sources.required_final_bytes().unwrap(),
            final_reserve_calls: r.sources.required_final_calls().unwrap(),
        };
        let error = admit_sources_with_refusal(r.sources, limits, &evidence)
            .err()
            .unwrap();
        let message = format!("{error:#}");
        assert!(message.contains("held source hash"));
        assert!(
            OpenOptions::new()
                .write(true)
                .open(environment.join("approval"))
                .is_err()
        );
        let failure = error
            .downcast_ref::<prepared_source_admission::SourceAdmissionFailure>()
            .unwrap();
        assert_eq!(failure.finalization.charged_bytes, 2);
        if blocked_record {
            assert!(message.contains("additionally failed to record"));
            assert_eq!(
                fs::read(evidence.join("REFUSAL.json")).unwrap(),
                b"preexisting refusal preserved"
            );
        } else {
            let value: Value =
                serde_json::from_slice(&fs::read(evidence.join("REFUSAL.json")).unwrap()).unwrap();
            assert_eq!(value["sourceAdmissionState"], "failed_lease_retained");
            assert_eq!(
                value["sourceFinalization"],
                serde_json::to_value(&failure.finalization).unwrap()
            );
            assert!(
                value["sourceFinalization"]["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["identity_matched"].is_null() && v["hash_matched"].is_null())
            );
            assert_eq!(value["installationApproved"], false);
        }
        drop(error);
        assert!(
            OpenOptions::new()
                .write(true)
                .open(environment.join("approval"))
                .is_ok()
        );
    }
}

#[test]
fn pre_admission_refusal_reports_unknown_finalization_without_invented_counters() {
    let root = PathBuf::from("E:/Scratch/sf178-effective-pre-admission-tests").join(format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let mut r = request();
    r.sources.schema = 99;
    assert!(admit_sources_with_refusal(r.sources, r.source_read_limits, &root).is_err());
    let value: Value =
        serde_json::from_slice(&fs::read(root.join("REFUSAL.json")).unwrap()).unwrap();
    assert_eq!(value["sourceAdmissionState"], "not_admitted");
    assert!(value["sourceFinalization"].is_null());
}

#[test]
fn committed_source_requires_exact_gitlinks_locks_and_normal_schema() {
    validate_committed_source(&committed_source()).unwrap();
    for mode in 0..6 {
        let mut source = committed_source();
        match mode {
            0 => source["schema_version"] = json!(2),
            1 => source["source_root"]["gitlinks"][1]["path"] = json!("kfc-parser"),
            2 => source["source_root"]["gitlinks"][0]["commit"] = json!("a".repeat(39)),
            3 => source["lockfiles"][1]["path"] = json!("Cargo.lock"),
            4 => source["lockfiles"][0]["sha256"] = json!("A".repeat(64)),
            5 => {
                source = json!({"schemaVersion":1,"kind":"UNCOMMITTED_SYNTHETIC_TEST_INPUT_SNAPSHOT_ONLY"})
            }
            _ => unreachable!(),
        }
        assert!(validate_committed_source(&source).is_err(), "case{mode}");
    }
}

#[test]
fn protocol_reads_are_e_only_capped_and_metadata_never_overwrites_existing_files() {
    for path in [
        "D:/Games/do-not-read.json",
        "E:/Scratch/../unsafe.json",
        "E:/Scratch/NUL.json",
        r"\\?\E:\Scratch\aliased.json",
    ] {
        assert!(read_protocol_json::<Value>(Path::new(path)).is_err());
    }
    let root = PathBuf::from("E:/Scratch/sf178-effective-protocol-tests").join(format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("request.json");
    fs::write(&path, br#"{"fixture":true}"#).unwrap();
    assert_eq!(
        read_protocol_json::<Value>(&path).unwrap(),
        json!({"fixture":true})
    );
    fs::write(&path, vec![b' '; 256 * 1024 + 1]).unwrap();
    assert!(
        read_protocol_json::<Value>(&path)
            .unwrap_err()
            .to_string()
            .contains("oversized")
    );
    assert!(limited_json(&"x".repeat(1024 * 1024)).is_err());
    let finalization = SourceFinalization {
        verified: false,
        files: vec![],
        directories_matched: false,
        membership_matched: None,
        phase_error: None,
        metadata_errors: vec![],
        charged_bytes: 0,
        returned_bytes: 0,
        calls: 0,
        terminal_io: true,
    };
    record_refusal(&root, "original failure", &finalization).unwrap();
    let first = fs::read(root.join("REFUSAL.json")).unwrap();
    assert!(record_refusal(&root, "replacement", &finalization).is_err());
    assert_eq!(fs::read(root.join("REFUSAL.json")).unwrap(), first);
    let value: Value = serde_json::from_slice(&first).unwrap();
    assert_eq!(value["primary"], "original failure");
    assert_eq!(
        value["sourceFinalization"]["membership_matched"],
        Value::Null
    );
    assert_eq!(value["installationApproved"], false);
}
