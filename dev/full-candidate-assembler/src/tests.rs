use super::*;
use sf_xhl_tree_adapter::{DirectoryPin, FilePin, Identity, InputFile};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn greeneye_profile_pin_matches_reviewed_definition() {
    let definition = include_bytes!("../../../docs/runtime/GREENEYE-ENTRY-CONTINUATION.md");
    assert!(valid_digest(GREENEYE_PROFILE_SHA256));
    assert_eq!(digest(definition), GREENEYE_PROFILE_SHA256);
}

struct Fixture {
    base: PathBuf,
    xhl_source: PathBuf,
    manifest: InputManifest,
    request_path: PathBuf,
    asset_receipt_path: PathBuf,
    xhl_plan_path: PathBuf,
    xhl_receipt_path: PathBuf,
    output_root: PathBuf,
    receipt_sha256: String,
}
fn selected_feature_rows() -> Vec<SelectedFeature> {
    [
        "Minimap",
        "First Person",
        "Bundled SF Unlimited Flight",
        "Vein Mining support now",
        "Witcher Signs",
        "Combat Tweaks",
        "Electric Blink",
        "EMBER",
        "Selected building tweaks",
        "Hoarder's Helper",
        "Grappling Hook Pull",
        "Max Stack",
        "Rested Enhanced",
        "Workshop Speed",
        "HD Map",
        "Creative Mode",
        "Find / Base Storage Search",
        "Map Teleport",
        "Remote Altar Close",
        "Unlimited Gifting Range",
    ]
    .into_iter()
    .enumerate()
    .map(|(index, name)| SelectedFeature {
        row: (index + 1) as u8,
        name: name.into(),
        enabled: true,
    })
    .collect()
}

#[test]
fn cli_input_reader_checks_owned_ancestry_and_size_before_reading() {
    let fixture = Fixture::new(false);
    let expected = fs::read(&fixture.request_path).unwrap();
    assert_eq!(
        read_input_file(&fixture.request_path, expected.len() as u64).unwrap(),
        expected
    );
    assert!(read_input_file(&fixture.request_path, 0).is_err());
    assert!(read_input_file(Path::new(r"D:\not-read\input.json"), 1024).is_err());
    let traversed = fixture.base.join(r"..\outside.json");
    let error = read_input_file(&traversed, 1024).unwrap_err().to_string();
    assert!(error.contains("noncanonical path"), "{error}");
}
impl Fixture {
    fn new(with_dinput: bool) -> Self {
        let root = PathBuf::from(
            std::env::var_os("SF_FULL_CANDIDATE_FIXTURE_ROOT")
                .expect("explicit E: fixture root is required"),
        );
        assert!(is_e_path(&root));
        let root_text = path_key(&root.to_string_lossy());
        assert!(root_text.starts_with("e:/scratch/sf177-full-candidate-assembler-tests"));
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = root.join(format!("case-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let env_root = base.join("environment");
        let output_root = base.join("candidate");
        let generator_root = base.join("prepared-output");
        let stage = generator_root.join(".enshrouded.shroudforge-stage");
        let selected_source = base.join("selected-manifest.md");
        let request_path = base.join("request.json");
        let receipt_path = base.join("asset-receipt.json");
        let plan_path = base.join("approved-xhl-plan.json");
        let xhl_source = base.join("xhl-source");
        fs::create_dir_all(&env_root).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::create_dir_all(&xhl_source).unwrap();

        let pinned_xhl_sources = [
            ("xhl-core.dll", "SF_XHL_CORE_FIXTURE_SOURCE"),
            (
                "mods/XHL-Remote-Altar-Close/XHL-Remote-Altar-Close.dll",
                "SF_XHL_REMOTE_FIXTURE_SOURCE",
            ),
            (
                "mods/XHL-Grappling-Hook-Pull-2x/XHL-Grappling-Hook-Pull-2x.dll",
                "SF_XHL_GRAPPLING_FIXTURE_SOURCE",
            ),
            (
                "mods/XHL-Unlimited-Gifting-Range/XHL-Unlimited-Gifting-Range.dll",
                "SF_XHL_GIFTING_FIXTURE_SOURCE",
            ),
        ];
        for (relative, variable) in pinned_xhl_sources {
            let source = PathBuf::from(std::env::var_os(variable).unwrap_or_else(|| {
                panic!("explicit E: source fixture binding {variable} is required")
            }));
            assert!(is_e_path(&source), "{variable} must bind an E: file");
            let destination = xhl_source.join(relative.replace('/', "\\"));
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(&source, &destination).unwrap_or_else(|error| {
                panic!("copy pinned inert XHL fixture source from {variable}: {error}")
            });
        }
        for (plugin, _) in sf_xhl_tree_adapter::pins::PLUGINS {
            let source = xhl_source.join(format!("mods/{plugin}").replace('/', "\\"));
            let parent = source.parent().unwrap();
            fs::write(
                parent.join("native-plugin.ini"),
                format!(
                    "[Plugin]\r\nEnabled=true\r\nDll={}\r\n",
                    source.file_name().unwrap().to_string_lossy()
                ),
            )
            .unwrap();
        }

        let selected_features = selected_feature_rows();
        let mut components = Vec::new();
        let mut packages = Vec::new();
        let mut environment_files = BTreeMap::new();
        let mut input_files = BTreeMap::new();
        let provider_specs = [
            ("aldrich-witcher-signs", vec![5]),
            ("combat_tweaks", vec![6]),
            ("ElectricBlink", vec![7]),
            ("esm-real-map", vec![15]),
            ("esm.buff-refresh", vec![8]),
            ("esm.build-zones", vec![8, 9]),
            ("esm.magic-factories", vec![8, 9]),
            ("esm.magic-furniture", vec![8, 9]),
            ("esm.shroud-building", vec![8, 9]),
            ("esm.stack-size", vec![12]),
            ("HoardersHelper", vec![10]),
            ("xhl-rested-enhanced", vec![13]),
            ("xhl-workshop-speed-20x", vec![14]),
        ];
        let provider_ids: Vec<_> = provider_specs.iter().map(|(id, _)| (*id).into()).collect();
        for (index, (id, feature_rows)) in provider_specs.iter().enumerate() {
            let directory = format!("provider-{index:02}");
            let package_root = env_root.join("mods").join(&directory);
            fs::create_dir_all(&package_root).unwrap();
            let manifest_bytes = serde_json::to_vec(&json!({
                "id": *id,
                "name": *id,
                "version": "1.0.0",
                "enabled": true,
                "capabilities": ["patch"],
            }))
            .unwrap();
            fs::write(package_root.join("mod.json"), &manifest_bytes).unwrap();
            let value = stamp(&package_root.join("mod.json")).unwrap();
            let rel = format!("mods/{directory}/mod.json");
            environment_files.insert(rel.clone(), value.clone());
            input_files.insert(rel, value.clone());
            // Canonical activation is persisted in the extension, not mod.json.
            let extension = package_root.join("extended.mod.json");
            fs::write(
                &extension,
                br#"{"schemaVersion":1,"enabled":true,"targets":["client"]}"#,
            )
            .unwrap();
            let extension_stamp = stamp(&extension).unwrap();
            let extension_rel = format!("mods/{directory}/extended.mod.json");
            environment_files.insert(extension_rel.clone(), extension_stamp.clone());
            input_files.insert(extension_rel, extension_stamp.clone());
            let config_bytes = match *id {
                "xhl-rested-enhanced" => Some(b"Enabled=true\r\n".as_slice()),
                "xhl-workshop-speed-20x" => Some(b"Enabled=true\r\nMultiplier=20\r\n".as_slice()),
                _ => None,
            };
            let config_stamp = config_bytes.map(|bytes| {
                fs::write(package_root.join("config.txt"), bytes).unwrap();
                let value = stamp(&package_root.join("config.txt")).unwrap();
                let relative = format!("mods/{directory}/config.txt");
                environment_files.insert(relative.clone(), value.clone());
                input_files.insert(relative, value.clone());
                value
            });
            components.push(SelectedComponent {
                feature_rows: feature_rows.clone(),
                dependencies: vec!["sf-host-runtime".into()],
                id: (*id).into(),
                version: "1.0.0".into(),
                kind: ComponentKind::AssetProvider,
                enabled: true,
                package_directory: Some(directory.clone()),
                service_id: None,
                xhl_paths: Vec::new(),
                requires_greeneye: false,
                target: "client".into(),
            });
            let mut source_files: BTreeMap<String, Stamp> = [
                ("mod.json".into(), value),
                ("extended.mod.json".into(), extension_stamp),
            ]
            .into_iter()
            .collect();
            if let Some(config_stamp) = config_stamp {
                source_files.insert("config.txt".into(), config_stamp);
            }
            packages.push(PackageInput {
                component_id: (*id).into(),
                package_directory: directory,
                origin: PackageOrigin::AssetEnvironment,
                source_root: None,
                source_files,
                overlays: Vec::new(),
            });
        }
        for (id, row, directory) in [
            ("minimap-frontend", 1, "minimap-frontend"),
            ("first-person-view", 2, "first-person-view"),
            ("flight-scalar-sdk", 3, "flight-scalar-sdk"),
            ("vein-support", 4, "vein-support"),
            ("creative-mode", 16, "creative-mode"),
            ("find-base-storage", 17, "find-base-storage"),
            ("map-teleport", 18, "map-teleport"),
        ] {
            let source_root = base.join(format!("approved-{id}"));
            fs::create_dir_all(&source_root).unwrap();
            fs::write(source_root.join("native.dll"), format!("fixture {id}")).unwrap();
            let source_files = inventory(&source_root).unwrap();
            components.push(SelectedComponent {
                feature_rows: vec![row],
                dependencies: vec!["sf-host-runtime".into()],
                id: id.into(),
                version: "1.0.0".into(),
                kind: ComponentKind::NativeOnly,
                enabled: true,
                package_directory: Some(directory.into()),
                service_id: None,
                xhl_paths: Vec::new(),
                requires_greeneye: false,
                target: "client".into(),
            });
            packages.push(PackageInput {
                component_id: id.into(),
                package_directory: directory.into(),
                origin: PackageOrigin::ApprovedE,
                source_root: Some(source_root),
                source_files,
                overlays: Vec::new(),
            });
        }
        components.push(SelectedComponent {
            feature_rows: Vec::new(),
            dependencies: Vec::new(),
            id: "metadata-service".into(),
            version: "1.0.0".into(),
            kind: ComponentKind::RuntimeService,
            enabled: true,
            package_directory: None,
            service_id: Some("metadata-init-v1".into()),
            xhl_paths: Vec::new(),
            requires_greeneye: false,
            target: "client".into(),
        });
        components.push(SelectedComponent {
            feature_rows: Vec::new(),
            dependencies: vec!["metadata-service".into()],
            id: "sf-host-runtime".into(),
            version: "1.7.7".into(),
            kind: ComponentKind::NativeOnly,
            enabled: true,
            package_directory: None,
            service_id: None,
            xhl_paths: Vec::new(),
            requires_greeneye: false,
            target: "client".into(),
        });
        let fake_base_stamp = Stamp {
            bytes: 1,
            sha256: digest(b"target preimage identity only; never read by assembler"),
        };
        for name in [
            "enshrouded.exe",
            "enshrouded.kfc",
            "enshrouded.kfc_resources",
        ] {
            input_files.insert(name.into(), fake_base_stamp.clone());
        }
        let mut request = AssetRequest {
            schema: 1,
            target_root: PathBuf::from(r"D:\InertFixture\Enshrouded"),
            asset_input_root: PathBuf::from(r"D:\InertFixture\Enshrouded"),
            environment_root: env_root.clone(),
            output_root: generator_root.clone(),
            selected_ids: provider_ids.clone(),
            inputs: input_files,
            environment_files: environment_files.clone(),
        };
        let request_bytes = serde_json::to_vec_pretty(&request).unwrap();
        let request_canonical_sha = digest(&serde_json::to_vec(&request).unwrap());
        fs::write(&request_path, &request_bytes).unwrap();

        let mut staged = BTreeMap::new();
        for (name, body) in [
            ("enshrouded.kfc", b"synthetic kfc postimage".as_slice()),
            (
                "enshrouded.kfc_resources",
                b"synthetic resources postimage".as_slice(),
            ),
            (
                "generated-inputs.json",
                b"synthetic generator input record".as_slice(),
            ),
        ] {
            fs::write(stage.join(name), body).unwrap();
            staged.insert(name.to_owned(), stamp(&stage.join(name)).unwrap());
        }
        fs::write(
            stage.join("prepared-only.json"),
            b"synthetic prepared marker",
        )
        .unwrap();
        let membership: BTreeMap<String, Stamp> = BTreeMap::new();
        let asset_receipt = AssetReceipt {
            schema: 2,
            source_closure: "a".repeat(64),
            request_sha256: request_canonical_sha,
            target_root: request.target_root.clone(),
            output_stage: stage.clone(),
            staged,
            prepared_marker: stamp(&stage.join("prepared-only.json")).unwrap(),
            source_dat_membership: membership.clone(),
            source_dat_membership_sha256: digest(&serde_json::to_vec(&membership).unwrap()),
        };
        let asset_receipt_bytes = serde_json::to_vec_pretty(&asset_receipt).unwrap();
        let asset_receipt_path = base.join("asset-receipt.json");
        fs::write(&asset_receipt_path, &asset_receipt_bytes).unwrap();
        let mut selected_matrix = String::from(
            "# Selected Components\n\n| # | Selected feature | Source | Integration | Acceptance |\n| ---: | --- | --- | --- | --- |\n",
        );
        for feature in &selected_features {
            selected_matrix.push_str(&format!(
                "| {} | {} | fixture source | fixture integration | fixture acceptance |\n",
                feature.row, feature.name
            ));
        }
        fs::write(&selected_source, selected_matrix.as_bytes()).unwrap();

        let target_root = request.target_root.clone();
        let mut patch_qualification_rows = Vec::new();
        let mut patch_bindings = Vec::new();
        for (provider_id, effective_values, observed_bytes, id_byte) in [
            (
                "xhl-rested-enhanced",
                PreservedPatchEffectiveValues::Rested(RestedPatchEffectiveValues { enabled: true }),
                b"inert existing Rested target bytes".as_slice(),
                0xa1,
            ),
            (
                "xhl-workshop-speed-20x",
                PreservedPatchEffectiveValues::Workshop(WorkshopPatchEffectiveValues {
                    enabled: true,
                    multiplier: 20,
                }),
                b"inert existing Workshop target bytes".as_slice(),
                0xb2,
            ),
        ] {
            let package = packages
                .iter()
                .find(|package| package.component_id == provider_id)
                .unwrap();
            let target = format!("mods/{}/config.txt", package.package_directory);
            let source_stamp = package.source_files.get("config.txt").unwrap().clone();
            let observed_target_before = PatchFilePin {
                bytes: observed_bytes.len() as u64,
                sha256: digest(observed_bytes),
                identity: PatchFileIdentity {
                    volume: 7,
                    id: [id_byte; 16],
                },
            };
            patch_qualification_rows.push(PatchConfigQualificationRow {
                component_id: provider_id.into(),
                relative_path: target.clone(),
                source: PatchConfigSourceProjection {
                    stamp: source_stamp.clone(),
                    effective_values: effective_values.clone(),
                },
                selected: PatchConfigValueProjection {
                    effective_values: effective_values.clone(),
                },
                target: PatchConfigTargetProjection {
                    observed_before: observed_target_before.clone(),
                    effective_values: effective_values.clone(),
                },
            });
            patch_bindings.push((
                provider_id.to_owned(),
                target,
                source_stamp,
                effective_values,
                observed_target_before,
            ));
        }
        let patch_qualification_path = base.join("patch-config-qualification.json");
        let patch_qualification = PatchConfigQualification {
            schema_version: 1,
            target_root: target_root.clone(),
            rows: patch_qualification_rows,
        };
        let patch_qualification_bytes = serde_json::to_vec_pretty(&patch_qualification).unwrap();
        fs::write(&patch_qualification_path, &patch_qualification_bytes).unwrap();
        let patch_evidence = EvidencePin {
            path: patch_qualification_path,
            sha256: digest(&patch_qualification_bytes),
        };
        let preserved_patch_configs: Vec<_> = patch_bindings
            .into_iter()
            .map(
                |(component_id, target, source_stamp, effective_values, observed_target_before)| {
                    PreservedPatchConfig {
                        component_id,
                        target,
                        source_stamp,
                        effective_values,
                        required_poststate: observed_target_before.clone(),
                        observed_target_before,
                        evidence: patch_evidence.clone(),
                    }
                },
            )
            .collect();
        let patch_preservation = preserved_patch_configs
            .iter()
            .map(|entry| PreservationEntry {
                component_id: entry.component_id.clone(),
                target: entry.target.clone(),
                evidence_sha256: patch_evidence.sha256.clone(),
                fields: PRESERVED_PATCH_CONFIG_FIELDS
                    .iter()
                    .map(|field| (*field).to_owned())
                    .collect(),
                disposition: PRESERVED_PATCH_CONFIG_DISPOSITION.to_owned(),
            })
            .collect();
        let target_parent = target_root.join("mods");
        let mut xhl_inputs = BTreeMap::new();
        let mut add_source = |name: String, source: String| {
            let path = xhl_source.join(source.replace('/', "\\"));
            let value = stamp(&path).unwrap();
            let file = std::fs::File::open(&path).unwrap();
            let (volume, id) = sf_xhl_tree_adapter::tree::held_file_identity(&file).unwrap();
            xhl_inputs.insert(
                name,
                InputFile {
                    source: Some(source),
                    source_identity: Some(Identity { volume, id }),
                    bytes: value.bytes,
                    sha256: value.sha256,
                },
            );
        };
        add_source("xhl-core.dll".into(), "xhl-core.dll".into());
        for (plugin, _) in sf_xhl_tree_adapter::pins::PLUGINS {
            let path = format!("mods/{plugin}");
            add_source(path.clone(), path.clone());
            let parent = plugin.rsplit_once('/').unwrap().0;
            let declaration = format!("mods/{parent}/native-plugin.ini");
            add_source(declaration.clone(), declaration);
        }
        xhl_inputs.insert(
            "dinput8-chain.ini".into(),
            InputFile {
                source: None,
                source_identity: None,
                bytes: sf_xhl_tree_adapter::tree::DISABLED_CHAIN.len() as u64,
                sha256: digest(sf_xhl_tree_adapter::tree::DISABLED_CHAIN),
            },
        );
        let source_root_identity = sf_xhl_tree_adapter::tree::pin_directory_chain(&xhl_source)
            .unwrap()
            .first()
            .unwrap()
            .identity();
        let plan = ApprovedPlan {
            schema_version: 1,
            transaction: "rb65-synthetic-transaction-001".into(),
            source_root: xhl_source.clone(),
            source_root_identity: Identity::directory(&source_root_identity),
            target_parent,
            target_parent_identity: Identity {
                volume: 2,
                id: [4; 16],
            },
            files: xhl_inputs,
        };
        components.push(SelectedComponent {
            feature_rows: Vec::new(),
            dependencies: vec!["sf-host-runtime".into()],
            id: "xhl-core".into(),
            version: "2.0.6".into(),
            kind: ComponentKind::XhlNative,
            enabled: true,
            package_directory: None,
            service_id: None,
            xhl_paths: vec!["xhl-core.dll".into(), "dinput8-chain.ini".into()],
            requires_greeneye: false,
            target: "client".into(),
        });
        for (plugin, _) in sf_xhl_tree_adapter::pins::PLUGINS {
            let (id, row, version) = if plugin.to_ascii_lowercase().contains("grappl") {
                ("xhl-grappling-hook", 11, "2.0.0")
            } else if plugin.to_ascii_lowercase().contains("remote") {
                ("xhl-remote-altar", 19, "2.0.1")
            } else if plugin.to_ascii_lowercase().contains("gifting") {
                ("xhl-unlimited-gifting", 20, "1.0.2")
            } else {
                panic!("fixture XHL plugin is outside selected feature scope: {plugin}");
            };
            let parent = plugin.rsplit_once('/').unwrap().0;
            components.push(SelectedComponent {
                feature_rows: vec![row],
                dependencies: vec!["xhl-core".into()],
                id: id.into(),
                version: version.into(),
                kind: ComponentKind::XhlNative,
                enabled: true,
                package_directory: None,
                service_id: None,
                xhl_paths: vec![
                    format!("mods/{plugin}"),
                    format!("mods/{parent}/native-plugin.ini"),
                ],
                requires_greeneye: false,
                target: "client".into(),
            });
        }
        let plan_bytes = serde_json::to_vec_pretty(&plan).unwrap();
        fs::write(&plan_path, &plan_bytes).unwrap();

        let mut loader_config = shroudforge_loader_config::config::read_loader_defaults().unwrap();
        loader_config["modules"]["dllManager"]["enabled"] = json!(true);
        let mut xhl_dll_files = vec![json!({"path":"xhl-core.dll","mode":"dinput8"})];
        for (plugin, _) in sf_xhl_tree_adapter::pins::PLUGINS {
            xhl_dll_files.push(json!({"path":format!("mods/{plugin}"),"mode":"library"}));
        }
        loader_config["modules"]["dllManager"]["profiles"] = json!([{
            "id":"xhl-core",
            "packageDirectory":"xhl-core",
            "name":"XHL Core",
            "target":"client",
            "enabled":true,
            "approved":true,
            "files":xhl_dll_files
        },{
            "id":"legacy-xhl-gifting-disabled",
            "packageDirectory":"XHL-Unlimited-Gifting-Range",
            "name":"Disabled legacy Gifting DLL selection",
            "target":"client",
            "enabled":false,
            "approved":true,
            "compatibilityProfile":null,
            "files":[]
        }]);
        let loader_config_bytes = serde_json::to_vec(&loader_config).unwrap();
        let mut root_files = Vec::new();
        for (target, body) in [
            ("winmm.dll", b"synthetic selected host".as_slice()),
            ("dinput8.dll", b"synthetic SF DirectInput host".as_slice()),
            (
                "shroudforge/shroudforge-runtime.dll",
                b"synthetic runtime".as_slice(),
            ),
            (
                "shroudforge/kfc-runtime.dll",
                b"synthetic kfc runtime".as_slice(),
            ),
            (
                "shroudforge/config/modloader-config.json",
                loader_config_bytes.as_slice(),
            ),
        ] {
            if target == "dinput8.dll" && !with_dinput {
                continue;
            }
            let source = base.join(format!("source-{}", target.replace('/', "_")));
            fs::write(&source, body).unwrap();
            root_files.push(RootFileInput {
                component_id: "sf-host-runtime".into(),
                source: source.clone(),
                target: target.into(),
                stamp: stamp(&source).unwrap(),
                runtime_binding: true,
            });
        }
        let excluded = [
            (ExclusionKind::Autoloot, "autoloot"),
            (ExclusionKind::Shroudtopia, "shroudtopia"),
            (ExclusionKind::StandaloneFlight, "standalone-flight"),
            (ExclusionKind::DuplicateFlight, "duplicate-flight"),
            (ExclusionKind::OldMinimapHost, "old-minimap-host"),
            (ExclusionKind::ServerOnly, "server-only"),
            (ExclusionKind::LegacyFallback, "legacy-fallback"),
        ]
        .into_iter()
        .map(|(kind, value)| Exclusion {
            kind,
            ids: vec![value.into()],
            paths: vec![format!("mods/{value}")],
            reason: format!("fixture excludes {value}"),
        })
        .collect();
        let manifest = InputManifest {
            schema_version: 1,
            candidate_id: "synthetic-client-candidate".into(),
            target_root,
            output_root: output_root.clone(),
            request: EvidencePin {
                path: request_path.clone(),
                sha256: digest(&request_bytes),
            },
            asset_receipt: EvidencePin {
                path: asset_receipt_path.clone(),
                sha256: digest(&asset_receipt_bytes),
            },
            derived_environment_evidence: None,
            prepared_producer_closure: "a".repeat(64),
            runtime_source_closure: "b".repeat(64),
            selected_manifest: EvidencePin {
                path: selected_source.clone(),
                sha256: digest(&fs::read(&selected_source).unwrap()),
            },
            selected_features,
            components,
            asset_provider_ids: provider_ids,
            exclusions: excluded,
            packages,
            root_files,
            contract_file_paths: Vec::new(),
            greeneye_paths: Vec::new(),
            greeneye_route: None,
            extended_zoom: false,
            selection: RuntimeSelection {
                boot_services: vec!["metadata-init-v1".into()],
                process_role: "client".into(),
                dedicated_guest_qualified: false,
                allow_unverified_cursor_gate: false,
                flight_transport_enabled: false,
                vein_selected: false,
                vein_settings_path: None,
                overlay: false,
                first_person: false,
            },
            xhl_plan: EvidencePin {
                path: plan_path.clone(),
                sha256: digest(&plan_bytes),
            },
            preservation_evidence: vec![patch_evidence],
            preservation: patch_preservation,
            preserved_runtime_files: Vec::new(),
            preserved_patch_configs,
            missing_requirements: Vec::new(),
        };
        let receipt = synthetic_xhl_receipt(&plan, &plan_bytes);
        let receipt_bytes = serde_json::to_vec_pretty(&receipt).unwrap();
        let xhl_receipt_path = base.join("xhl-prepared-receipt.json");
        fs::write(&xhl_receipt_path, &receipt_bytes).unwrap();
        Self {
            base,
            xhl_source,
            manifest,
            request_path,
            asset_receipt_path,
            xhl_plan_path: plan_path,
            xhl_receipt_path,
            output_root,
            receipt_sha256: digest(&receipt_bytes),
        }
    }
    fn prepare(&self) -> Template {
        let bytes = serde_json::to_vec_pretty(&self.manifest).unwrap();
        prepare_template(&bytes, &digest(&bytes)).unwrap()
    }
    fn plan_bytes(&self) -> Vec<u8> {
        fs::read(&self.xhl_plan_path).unwrap()
    }
    fn receipt_bytes(&self) -> Vec<u8> {
        fs::read(&self.xhl_receipt_path).unwrap()
    }
}
fn add_greeneye_host(fixture: &mut Fixture) -> String {
    let minimap = fixture
        .manifest
        .components
        .iter_mut()
        .find(|component| component.id == "minimap-frontend")
        .unwrap();
    minimap.dependencies.push("greeneye-host".into());
    let host_root = fixture.base.join("approved-greeneye-host");
    fs::create_dir_all(&host_root).unwrap();
    fs::write(host_root.join("dbgcore.dll"), b"synthetic Greeneye host").unwrap();
    let host_files = inventory(&host_root).unwrap();
    fixture.manifest.components.push(SelectedComponent {
        feature_rows: Vec::new(),
        dependencies: vec!["sf-host-runtime".into()],
        id: "greeneye-host".into(),
        version: "7b97-profile-fixture".into(),
        kind: ComponentKind::NativeOnly,
        enabled: true,
        package_directory: Some("greeneye-host".into()),
        service_id: None,
        xhl_paths: Vec::new(),
        requires_greeneye: true,
        target: "client".into(),
    });
    fixture.manifest.packages.push(PackageInput {
        component_id: "greeneye-host".into(),
        package_directory: "greeneye-host".into(),
        origin: PackageOrigin::ApprovedE,
        source_root: Some(host_root),
        source_files: host_files,
        overlays: Vec::new(),
    });
    let config = fixture
        .manifest
        .root_files
        .iter_mut()
        .find(|file| file.target == "shroudforge/config/modloader-config.json")
        .unwrap();
    let mut loader: Value = serde_json::from_slice(&fs::read(&config.source).unwrap()).unwrap();
    let profiles = loader
        .pointer_mut("/modules/dllManager/profiles")
        .and_then(Value::as_array_mut)
        .unwrap();
    profiles.push(json!({
        "id":"greeneye-host",
        "packageDirectory":"greeneye-host",
        "name":"Greeneye host",
        "target":"client",
        "enabled":true,
        "approved":true,
        "files":[{"path":"dbgcore.dll","mode":"library"}]
    }));
    fs::write(&config.source, serde_json::to_vec(&loader).unwrap()).unwrap();
    config.stamp = stamp(&config.source).unwrap();
    let target = "mods/greeneye-host/dbgcore.dll".to_string();
    fixture.manifest.greeneye_paths.push(target.clone());
    target
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let root = PathBuf::from(std::env::var_os("SF_FULL_CANDIDATE_FIXTURE_ROOT").unwrap());
        if let (Ok(base), Ok(parent)) = (fs::canonicalize(&self.base), fs::canonicalize(root)) {
            if base.starts_with(&parent) && base.parent() == Some(parent.as_path()) {
                let _ = fs::remove_dir_all(base);
            }
        }
    }
}
fn synthetic_xhl_receipt(plan: &ApprovedPlan, plan_bytes: &[u8]) -> XhlReceipt {
    let directories = expected_scan_directories(plan)
        .unwrap()
        .into_iter()
        .enumerate()
        .map(|(index, relative)| DirectoryPin {
            relative,
            volume: 1,
            id: [(index as u8).wrapping_add(80); 16],
        })
        .collect::<Vec<_>>();
    let container = DirectoryPin {
        relative: String::new(),
        volume: 1,
        id: [60; 16],
    };
    let files = plan
        .files
        .iter()
        .enumerate()
        .map(|(index, (name, input))| {
            (
                format!("mods/xhl-core/{name}"),
                FilePin {
                    bytes: input.bytes,
                    sha256: input.sha256.clone(),
                    identity: Identity {
                        volume: 1,
                        id: [(index as u8).wrapping_add(100); 16],
                    },
                },
            )
        })
        .collect();
    XhlReceipt {
        schema_version: 1,
        transaction: plan.transaction.clone(),
        approved_plan_sha256: digest(plan_bytes),
        target: plan.target_parent.join("xhl-core"),
        container,
        directories,
        files,
    }
}
fn write_json(path: &Path, value: &impl Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn add_preserved_vein(fixture: &mut Fixture) -> PreservedRuntimeFile {
    let target = "mods/shroudweave-vein-mining-test.ini";
    let stamp = Stamp {
        bytes: 82,
        sha256: "c".repeat(64),
    };
    let evidence_path = fixture.base.join("preserved-values.json");
    let evidence = json!({"sourceRoot":fixture.manifest.target_root,"files":[{
        "relativePath":target,"bytes":stamp.bytes,"sha256":stamp.sha256,
        "postSha256":stamp.sha256,"status":"PRESENT_PRESERVE_BYTES"
    }]});
    write_json(&evidence_path, &evidence);
    let pin = EvidencePin {
        path: evidence_path.clone(),
        sha256: digest(&fs::read(&evidence_path).unwrap()),
    };
    let input = PreservedRuntimeFile {
        component_id: "vein-support".into(),
        target: target.into(),
        stamp,
        evidence: pin.clone(),
    };
    fixture.manifest.preservation_evidence.push(pin.clone());
    fixture.manifest.preservation.push(PreservationEntry {
        component_id: input.component_id.clone(),
        target: target.into(),
        evidence_sha256: pin.sha256,
        fields: vec!["all bytes including unknown fields".into()],
        disposition: "PRESERVE_BYTES".into(),
    });
    fixture.manifest.preserved_runtime_files.push(input.clone());
    fixture.manifest.selection.vein_settings_path = Some(target.into());
    fixture.manifest.selection.vein_selected = true;
    fixture.manifest.selection.dedicated_guest_qualified = true;
    fixture.manifest.selection.allow_unverified_cursor_gate = true;
    fixture.manifest.selection.boot_services.extend([
        "vein-flight-provider-v1".into(),
        "vein-activation-v1".into(),
    ]);
    for (id, service, dependency) in [
        (
            "shared-vein-service",
            "vein-flight-provider-v1",
            "metadata-service",
        ),
        (
            "activate-vein-service",
            "vein-activation-v1",
            "shared-vein-service",
        ),
    ] {
        fixture.manifest.components.push(SelectedComponent {
            feature_rows: Vec::new(),
            dependencies: vec![dependency.into()],
            id: id.into(),
            version: "1.0.0".into(),
            kind: ComponentKind::RuntimeService,
            enabled: true,
            package_directory: None,
            service_id: Some(service.into()),
            xhl_paths: Vec::new(),
            requires_greeneye: false,
            target: "client".into(),
        });
    }
    fixture
        .manifest
        .components
        .iter_mut()
        .find(|unit| unit.id == "vein-support")
        .unwrap()
        .dependencies
        .push("activate-vein-service".into());
    input
}

#[test]
fn preserved_runtime_ini_binds_without_copy_and_survives_finalization() {
    let mut fixture = Fixture::new(true);
    let input = add_preserved_vein(&mut fixture);
    let template = fixture.prepare();
    assert_eq!(template.status, "NONINSTALLABLE_TEMPLATE");
    assert!(!template.payload_files.contains_key(&input.target));
    assert!(!template.payload_root.join(&input.target).exists());
    let contract = template.contract.as_ref().unwrap();
    assert_eq!(contract.files[&input.target].sha256, input.stamp.sha256);
    assert_eq!(contract.files[&input.target].bytes, 82);
    let path = fixture.output_root.join("template.json");
    let hash = digest(&fs::read(&path).unwrap());
    let result = finalize_policy(
        &path,
        &hash,
        &fixture.xhl_plan_path,
        &fixture.xhl_receipt_path,
        &fixture.receipt_sha256,
    )
    .unwrap();
    assert!(!result.install_performed);
    let policy: Value = serde_json::from_slice(
        &fs::read(
            template
                .payload_root
                .join("shroudforge/config/current-assets.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        policy["runtime"]["files"][&input.target]["sha256"],
        input.stamp.sha256
    );
}

#[test]
fn preserved_runtime_ini_rejects_wrong_evidence_copy_and_contract_drift() {
    let mut fixture = Fixture::new(true);
    let input = add_preserved_vein(&mut fixture);
    let mut changed = input.clone();
    changed.stamp.bytes += 1;
    assert!(
        validate_preserved_runtime_files(
            &[changed],
            &fixture.manifest.target_root,
            Some(&input.target),
            &fixture.manifest.preservation,
            &BTreeMap::new()
        )
        .is_err()
    );
    let payload = [(
        input.target.to_ascii_uppercase(),
        FileBinding {
            bytes: 82,
            sha256: input.stamp.sha256.clone(),
        },
    )]
    .into_iter()
    .collect();
    assert!(
        validate_preserved_runtime_files(
            &[input.clone()],
            &fixture.manifest.target_root,
            Some(&input.target),
            &fixture.manifest.preservation,
            &payload
        )
        .is_err()
    );
    assert!(
        validate_preserved_runtime_files(
            &[input.clone()],
            &fixture.manifest.target_root,
            None,
            &fixture.manifest.preservation,
            &BTreeMap::new()
        )
        .is_err()
    );
    let mut template = fixture.prepare();
    template
        .contract
        .as_mut()
        .unwrap()
        .files
        .get_mut(&input.target)
        .unwrap()
        .bytes += 1;
    let path = fixture.output_root.join("tampered-template.json");
    write_json(&path, &template);
    let hash = digest(&fs::read(&path).unwrap());
    assert!(
        finalize_policy(
            &path,
            &hash,
            &fixture.xhl_plan_path,
            &fixture.xhl_receipt_path,
            &fixture.receipt_sha256
        )
        .is_err()
    );
}

#[test]
fn patch_config_sources_are_inventory_checked_but_never_staged_or_runtime_bound() {
    let fixture = Fixture::new(true);
    let template = fixture.prepare();
    assert_eq!(template.preserved_patch_configs.len(), 2);
    let contract = template.contract.as_ref().unwrap();
    for entry in &template.preserved_patch_configs {
        assert!(
            !template
                .payload_files
                .keys()
                .any(|path| path.eq_ignore_ascii_case(&entry.target))
        );
        assert!(
            !template
                .payload_root
                .join(entry.target.replace('/', "\\"))
                .exists()
        );
        assert!(
            !contract
                .files
                .keys()
                .any(|path| path.eq_ignore_ascii_case(&entry.target))
        );
        assert!(
            !template
                .contract_file_paths
                .iter()
                .any(|path| path.eq_ignore_ascii_case(&entry.target))
        );
        let request: AssetRequest = serde_json::from_str(&template.request_json).unwrap();
        let source_stamp = request
            .environment_files
            .iter()
            .find(|(path, _)| path.eq_ignore_ascii_case(&entry.target))
            .map(|(_, stamp)| stamp)
            .unwrap();
        assert_eq!(source_stamp, &entry.source_stamp);
        assert!(
            fixture
                .manifest
                .packages
                .iter()
                .find(|package| package.component_id == entry.component_id)
                .unwrap()
                .source_files
                .contains_key("config.txt")
        );
    }

    let template_path = fixture.output_root.join("template.json");
    let hash = digest(&fs::read(&template_path).unwrap());
    let result = finalize_policy(
        &template_path,
        &hash,
        &fixture.xhl_plan_path,
        &fixture.xhl_receipt_path,
        &fixture.receipt_sha256,
    )
    .unwrap();
    assert!(!result.install_performed);
    let policy: Value = serde_json::from_slice(
        &fs::read(
            template
                .payload_root
                .join("shroudforge/config/current-assets.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let runtime_files = policy["runtime"]["files"].as_object().unwrap();
    for entry in &template.preserved_patch_configs {
        assert!(
            !runtime_files
                .keys()
                .any(|path| path.eq_ignore_ascii_case(&entry.target))
        );
    }
}

#[test]
fn patch_config_inputs_reject_missing_rows_unknown_fields_and_value_or_pin_drift() {
    let fixture = Fixture::new(true);
    let mut missing = fixture.manifest.clone();
    missing.preserved_patch_configs.pop();
    let bytes = serde_json::to_vec(&missing).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());

    let mut unknown = serde_json::to_value(&fixture.manifest).unwrap();
    unknown["preservedPatchConfigs"][0]["providerId"] = json!("xhl-rested-enhanced");
    let bytes = serde_json::to_vec(&unknown).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());

    let mut wrong_values = fixture.manifest.clone();
    wrong_values.preserved_patch_configs[1].effective_values =
        PreservedPatchEffectiveValues::Workshop(WorkshopPatchEffectiveValues {
            enabled: true,
            multiplier: 10,
        });
    let bytes = serde_json::to_vec(&wrong_values).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());

    let mut wrong_poststate = fixture.manifest.clone();
    wrong_poststate.preserved_patch_configs[0]
        .required_poststate
        .sha256 = "d".repeat(64);
    let bytes = serde_json::to_vec(&wrong_poststate).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());

    let mut contract_collision = fixture.manifest.clone();
    contract_collision
        .contract_file_paths
        .push(contract_collision.preserved_patch_configs[0].target.clone());
    let bytes = serde_json::to_vec(&contract_collision).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());

    let mut overlay_collision = fixture.manifest.clone();
    let target_component = overlay_collision.preserved_patch_configs[0]
        .component_id
        .clone();
    overlay_collision
        .packages
        .iter_mut()
        .find(|package| package.component_id == target_component)
        .unwrap()
        .overlays
        .push(IntendedOverlay {
            path: "config.txt".into(),
            source: fixture.base.join("unused-overlay"),
            stamp: Stamp {
                bytes: 1,
                sha256: "e".repeat(64),
            },
            replaces: true,
            evidence: Vec::new(),
            fields: vec!["enabled".into()],
            disposition: "fixture collision".into(),
        });
    let bytes = serde_json::to_vec(&overlay_collision).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());
}

#[test]
fn finalizer_rejects_patch_config_contract_and_payload_collisions() {
    let fixture = Fixture::new(true);
    let mut template = fixture.prepare();
    let target = template.preserved_patch_configs[0].target.clone();
    template.payload_files.insert(
        target.clone(),
        FileBinding {
            bytes: 1,
            sha256: "f".repeat(64),
        },
    );
    let path = fixture.output_root.join("tampered-patch-template.json");
    write_json(&path, &template);
    let hash = digest(&fs::read(&path).unwrap());
    assert!(
        finalize_policy(
            &path,
            &hash,
            &fixture.xhl_plan_path,
            &fixture.xhl_receipt_path,
            &fixture.receipt_sha256
        )
        .is_err()
    );
    assert!(
        !template
            .payload_root
            .join(target.replace('/', "\\"))
            .exists()
    );
    assert!(
        !template
            .payload_root
            .join("shroudforge/config/current-assets.json")
            .exists()
    );
}

#[test]
fn prepares_and_finalizes_without_reading_target_and_keeps_digest_forms_distinct() {
    let fixture = Fixture::new(true);
    let manifest_bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    let template = prepare_template(&manifest_bytes, &digest(&manifest_bytes)).unwrap();
    assert_eq!(template.status, "NONINSTALLABLE_TEMPLATE");
    assert!(template.missing_requirements.is_empty());
    assert!(template.contract.is_some());
    let request: AssetRequest = serde_json::from_str(&template.request_json).unwrap();
    assert_ne!(
        template.request_raw_sha256,
        digest(&serde_json::to_vec(&request).unwrap())
    );
    assert_eq!(
        template.request_canonical_sha256,
        digest(&serde_json::to_vec(&request).unwrap())
    );
    assert_eq!(
        template.request_canonical_sha256,
        request
            .inputs
            .get("enshrouded.exe")
            .map(|_| {
                let receipt: AssetReceipt = serde_json::from_str(&template.receipt_json).unwrap();
                receipt.request_sha256
            })
            .unwrap()
    );
    let template_contract = template.contract.as_ref().unwrap();
    let core_target = target_path(&template.target_root, XHL_CORE_PATH).unwrap();
    let core_plan_entries = template_contract
        .dll_plan
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| {
            entry
                .get("path")
                .and_then(Value::as_str)
                .is_some_and(|path| {
                    normalize_absolute(Path::new(path)).ok()
                        == normalize_absolute(&core_target).ok()
                })
        })
        .count();
    assert_eq!(core_plan_entries, 1);
    assert!(!template_contract.files.contains_key(XHL_CORE_PATH));
    assert_eq!(
        template.xhl_files[XHL_CORE_PATH].sha256,
        sf_xhl_tree_adapter::pins::CORE_SHA
    );
    assert!(
        template
            .payload_files
            .keys()
            .all(|path| !path.to_ascii_lowercase().starts_with("mods/xhl-core/"))
    );
    assert!(!template.payload_root.join("mods/xhl-core").exists());
    let template_path = fixture.output_root.join("template.json");
    let template_bytes = fs::read(&template_path).unwrap();
    let template_hash = digest(&template_bytes);
    let template_json: Value = serde_json::from_slice(&template_bytes).unwrap();
    assert!(
        template_json.pointer("/contract/selection/xhl").is_none()
            || template_json.pointer("/contract/selection/xhl") == Some(&Value::Null)
    );
    let result = finalize_policy(
        &template_path,
        &template_hash,
        &fixture.xhl_plan_path,
        &fixture.xhl_receipt_path,
        &fixture.receipt_sha256,
    )
    .unwrap();
    assert_eq!(
        result.status,
        "FINALIZED_POLICY_NONINSTALLABLE_UNTIL_DEPLOYMENT"
    );
    assert!(!result.install_performed && result.deployment_target_recheck_required);
    let policy_path = fixture
        .output_root
        .join("payload/shroudforge/config/current-assets.json");
    let policy: Value = serde_json::from_slice(&fs::read(policy_path).unwrap()).unwrap();
    assert_eq!(policy["schemaVersion"], 6);
    assert_eq!(policy["owner"], "shroudforge-full-prepared");
    assert_eq!(policy["requestSha256"], template.request_raw_sha256);
    assert_eq!(policy["receiptSha256"], template.receipt_raw_sha256);
    assert_eq!(
        policy["runtime"]["preparedRequestSha256"],
        template.request_raw_sha256
    );
    assert_eq!(
        policy["runtime"]["preparedReceiptSha256"],
        template.receipt_raw_sha256
    );
    assert_eq!(
        policy["runtime"]["selection"]["xhl"]["service"],
        "xhl-core-registration-v1"
    );
    assert_eq!(
        policy["runtime"]["selection"]["xhl"]["directories"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        policy["runtime"]["dllPlan"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| {
                entry
                    .get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|path| {
                        normalize_absolute(Path::new(path)).ok()
                            == normalize_absolute(&core_target).ok()
                    })
            })
            .count(),
        1
    );
    assert_eq!(
        policy["runtime"]["files"][XHL_CORE_PATH]["sha256"],
        sf_xhl_tree_adapter::pins::CORE_SHA
    );
}

#[test]
fn missing_native_binding_leaves_only_a_blocked_noninstallable_template() {
    let mut fixture = Fixture::new(false);
    fixture.output_root = fixture.base.join("candidate-missing-native");
    fixture.manifest.output_root = fixture.output_root.clone();
    let bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    let template = prepare_template(&bytes, &digest(&bytes)).unwrap();
    assert_eq!(template.status, "BLOCKED_MISSING_REQUIREMENTS");
    assert!(template.contract.is_none());
    assert!(
        template
            .missing_requirements
            .iter()
            .any(|item| item.id == "runtime-file:dinput8.dll")
    );
    let result = finalize_policy(
        &fixture.output_root.join("template.json"),
        &digest(&fs::read(fixture.output_root.join("template.json")).unwrap()),
        &fixture.xhl_plan_path,
        &fixture.xhl_receipt_path,
        &fixture.receipt_sha256,
    );
    assert!(result.is_err());
}

#[test]
fn refuses_target_escape_and_xhl_plan_or_receipt_mismatch() {
    let mut escaped = Fixture::new(true);
    escaped.output_root = escaped.base.join("candidate-escape");
    escaped.manifest.output_root = escaped.output_root.clone();
    escaped.manifest.root_files[0].target = "../outside.dll".into();
    let bytes = serde_json::to_vec_pretty(&escaped.manifest).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());

    let fixture = Fixture::new(true);
    let template = fixture.prepare();
    let template_path = fixture.output_root.join("template.json");
    let template_hash = digest(&fs::read(&template_path).unwrap());
    let mut changed_plan: Value = serde_json::from_slice(&fixture.plan_bytes()).unwrap();
    changed_plan["transaction"] = json!("another-xhl-transaction");
    write_json(&fixture.xhl_plan_path, &changed_plan);
    assert!(
        finalize_policy(
            &template_path,
            &template_hash,
            &fixture.xhl_plan_path,
            &fixture.xhl_receipt_path,
            &fixture.receipt_sha256
        )
        .is_err()
    );

    let fixture = Fixture::new(true);
    let template = fixture.prepare();
    let template_path = fixture.output_root.join("template.json");
    let template_hash = digest(&fs::read(&template_path).unwrap());
    let mut wrong_receipt: XhlReceipt = serde_json::from_slice(&fixture.receipt_bytes()).unwrap();
    wrong_receipt.approved_plan_sha256 = "0".repeat(64);
    let bad_receipt = fixture.base.join("wrong-xhl-receipt.json");
    write_json(&bad_receipt, &wrong_receipt);
    let bad_hash = digest(&fs::read(&bad_receipt).unwrap());
    assert!(
        finalize_policy(
            &template_path,
            &template_hash,
            &fixture.xhl_plan_path,
            &bad_receipt,
            &bad_hash
        )
        .is_err()
    );
    assert!(template.missing_requirements.is_empty());
}

#[test]
fn source_inventory_mismatch_is_refused_before_candidate_output_creation() {
    let mut fixture = Fixture::new(true);
    fixture.output_root = fixture.base.join("candidate-inventory-mismatch");
    fixture.manifest.output_root = fixture.output_root.clone();
    fixture.manifest.packages[0].source_files.insert(
        "unlisted.txt".into(),
        Stamp {
            bytes: 1,
            sha256: digest(b"missing"),
        },
    );
    let bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());
    assert!(!fixture.output_root.exists());
}

#[test]
fn selected_manifest_binds_exact_twenty_named_rows() {
    let mut fixture = Fixture::new(false);
    fixture.manifest.selected_features[0].name = "different selected feature".into();
    let bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());

    let mut fixture = Fixture::new(false);
    fs::write(
        &fixture.manifest.selected_manifest.path,
        b"{\"selected\": []}",
    )
    .unwrap();
    fixture.manifest.selected_manifest.sha256 =
        digest(&fs::read(&fixture.manifest.selected_manifest.path).unwrap());
    let bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());
}

#[test]
fn enabled_runtime_only_requires_a_package_and_canonical_plan_row() {
    let mut fixture = Fixture::new(false);
    let component = fixture
        .manifest
        .components
        .iter_mut()
        .find(|component| component.kind == ComponentKind::NativeOnly)
        .unwrap();
    component.kind = ComponentKind::RuntimeOnly;
    component.package_directory = Some("missing-runtime-package".into());
    let bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());

    let mut fixture = Fixture::new(false);
    let component = fixture
        .manifest
        .components
        .iter_mut()
        .find(|component| component.kind == ComponentKind::NativeOnly)
        .unwrap();
    component.kind = ComponentKind::RuntimeOnly;
    let id = component.id.clone();
    assert!(validate_enabled_runtime_plan(&fixture.manifest, &[]).is_err());
    assert!(validate_enabled_runtime_plan(&fixture.manifest, &[id]).is_ok());
}

#[test]
fn runtime_service_is_a_typed_nonpackage_selection() {
    let mut fixture = Fixture::new(false);
    let service = fixture
        .manifest
        .components
        .iter_mut()
        .find(|component| component.kind == ComponentKind::RuntimeService)
        .unwrap();
    service.service_id = Some("unselected-service".into());
    assert!(validate_runtime_service_rows(&fixture.manifest).is_err());
}

#[test]
fn selected_greeneye_requires_and_binds_explicit_paths() {
    let mut fixture = Fixture::new(false);
    add_greeneye_host(&mut fixture);
    fixture.manifest.greeneye_paths.clear();
    let bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());
    assert!(!fixture.output_root.exists());

    let mut fixture = Fixture::new(false);
    let host_target = add_greeneye_host(&mut fixture);
    let bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    let template = prepare_template(&bytes, &digest(&bytes)).unwrap();
    assert_eq!(template.status, "BLOCKED_MISSING_REQUIREMENTS");
    assert!(template.contract.is_none());
    assert!(template.greeneye_paths.contains(&host_target));
    assert!(template.payload_files.contains_key(&host_target));
    assert!(
        template
            .missing_requirements
            .iter()
            .any(|item| item.id == "greeneye-route-evidence")
    );
}

#[test]
fn greeneye_profile_and_runtime_artifact_identity_are_typed_gates() {
    let mut fixture = Fixture::new(false);
    let host_target = add_greeneye_host(&mut fixture);
    let runtime = fixture
        .manifest
        .root_files
        .iter()
        .find(|file| file.target == "shroudforge/shroudforge-runtime.dll")
        .unwrap();
    let bootstrap = fixture
        .manifest
        .root_files
        .iter()
        .find(|file| file.target == "winmm.dll")
        .unwrap();
    let profile = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/runtime/GREENEYE-ENTRY-CONTINUATION.md");
    let route = GreeneyeRouteEvidence {
        profile_id: GREENEYE_PROFILE_ID.into(),
        profile_version: GREENEYE_PROFILE_VERSION + 1,
        profile_definition: EvidencePin {
            path: profile,
            sha256: GREENEYE_PROFILE_SHA256.into(),
        },
        runtime_source_closure: fixture.manifest.runtime_source_closure.clone(),
        runtime_artifact: GreeneyeArtifactBinding {
            source: runtime.source.clone(),
            target: runtime.target.clone(),
            stamp: Stamp {
                bytes: 1,
                sha256: "0".repeat(64),
            },
        },
        bootstrap_artifact: GreeneyeArtifactBinding {
            source: bootstrap.source.clone(),
            target: bootstrap.target.clone(),
            stamp: Stamp {
                bytes: 1,
                sha256: "0".repeat(64),
            },
        },
        host: GreeneyeInputBinding {
            target: host_target,
            stamp: Stamp {
                bytes: GREENEYE_HOST_BYTES,
                sha256: GREENEYE_HOST_SHA256.into(),
            },
        },
        scan_inputs: Vec::new(),
    };
    fixture.manifest.greeneye_route = Some(route);
    let bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());

    let route = fixture.manifest.greeneye_route.as_mut().unwrap();
    route.profile_version = GREENEYE_PROFILE_VERSION;
    let bytes = serde_json::to_vec_pretty(&fixture.manifest).unwrap();
    assert!(prepare_template(&bytes, &digest(&bytes)).is_err());
}

#[test]
fn greeneye_host_load_plan_requires_one_ordinary_library_entry() {
    let target_root = PathBuf::from(r"D:\Games\SteamLibrary\steamapps\common\Enshrouded");
    let host_target = "mods/greeneye-host/dbgcore.dll";
    let host_path = target_path(&target_root, host_target).unwrap();
    let accepted = serde_json::json!([{
        "path": host_path,
        "mode": "library"
    }]);
    validate_greeneye_dll_plan(&accepted, &target_root, host_target).unwrap();

    let wrong_mode = serde_json::json!([{
        "path": target_path(&target_root, host_target).unwrap(),
        "mode": "dinput8"
    }]);
    assert!(validate_greeneye_dll_plan(&wrong_mode, &target_root, host_target).is_err());

    let wrong_path = serde_json::json!([{
        "path": target_path(&target_root, "dbgcore.dll").unwrap(),
        "mode": "library"
    }]);
    assert!(validate_greeneye_dll_plan(&wrong_path, &target_root, host_target).is_err());

    let duplicate = serde_json::json!([
        {"path": target_path(&target_root, host_target).unwrap(), "mode": "library"},
        {"path": target_path(&target_root, host_target).unwrap(), "mode": "library"}
    ]);
    assert!(validate_greeneye_dll_plan(&duplicate, &target_root, host_target).is_err());
}

#[test]
fn xhl_reserved_host_log_is_matched_case_insensitively() {
    assert!(is_reserved_xhl_output("XHL-Native-Plugin-Host.log"));
    assert!(is_reserved_xhl_output("xhl-native-plugin-host.log"));
    assert!(is_reserved_xhl_output("xHl/config/native.json"));
    assert!(!is_reserved_xhl_output("XHL-Native-Plugin-Host.log.old"));
}

#[test]
fn request_order_is_preserved_as_audit_while_runtime_order_is_canonical() {
    let mut fixture = Fixture::new(true);
    let mut request: AssetRequest =
        serde_json::from_slice(&fs::read(&fixture.request_path).unwrap()).unwrap();
    request.selected_ids.reverse();
    let request_bytes = serde_json::to_vec_pretty(&request).unwrap();
    fs::write(&fixture.request_path, &request_bytes).unwrap();
    fixture.manifest.request.sha256 = digest(&request_bytes);
    fixture.manifest.asset_provider_ids.reverse();
    let (mut providers, others): (Vec<_>, Vec<_>) = fixture
        .manifest
        .components
        .drain(..)
        .partition(|component| component.kind == ComponentKind::AssetProvider);
    providers.reverse();
    fixture.manifest.components = providers.into_iter().chain(others).collect();

    let mut receipt: AssetReceipt =
        serde_json::from_slice(&fs::read(&fixture.asset_receipt_path).unwrap()).unwrap();
    receipt.request_sha256 = digest(&serde_json::to_vec(&request).unwrap());
    let receipt_bytes = serde_json::to_vec_pretty(&receipt).unwrap();
    fs::write(&fixture.asset_receipt_path, &receipt_bytes).unwrap();
    fixture.manifest.asset_receipt.sha256 = digest(&receipt_bytes);

    let template = fixture.prepare();
    assert_eq!(template.asset_request_selected_ids, request.selected_ids);
    assert_eq!(
        template.asset_provider_ids,
        fixture.manifest.asset_provider_ids
    );
    assert_eq!(
        template.canonical_runtime_mod_order,
        template.contract.as_ref().unwrap().runtime_mods
    );
}

#[test]
fn grouped_ember_and_building_features_map_to_distinct_provider_units() {
    let fixture = Fixture::new(false);
    validate_feature_unit_mapping(&fixture.manifest).unwrap();
    let row_units = |row| {
        fixture
            .manifest
            .components
            .iter()
            .filter(|unit| {
                unit.kind == ComponentKind::AssetProvider && unit.feature_rows.contains(&row)
            })
            .count()
    };
    assert_eq!(row_units(8), 5);
    assert_eq!(row_units(9), 4);
    assert_eq!(row_units(12), 1);
    assert_eq!(
        fixture
            .manifest
            .components
            .iter()
            .filter(|unit| unit.kind == ComponentKind::AssetProvider)
            .count(),
        REQUIRED_ASSET_PROVIDERS
    );
    assert_eq!(fixture.manifest.selected_features.len(), REQUIRED_FEATURES);
}

#[test]
fn enabled_selected_feature_without_an_enabled_unit_is_refused() {
    let mut fixture = Fixture::new(false);
    for unit in &mut fixture.manifest.components {
        unit.feature_rows.retain(|row| *row != 9);
    }
    assert!(validate_feature_unit_mapping(&fixture.manifest).is_err());
}

#[test]
fn xhl_units_must_own_each_approved_plan_file_exactly_once() {
    let mut fixture = Fixture::new(false);
    let plan: ApprovedPlan = serde_json::from_slice(&fixture.plan_bytes()).unwrap();
    validate_xhl_unit_ownership(&fixture.manifest.components, &plan).unwrap();
    let core = fixture
        .manifest
        .components
        .iter_mut()
        .find(|unit| unit.id == "xhl-core")
        .unwrap();
    core.xhl_paths.pop();
    assert!(validate_xhl_unit_ownership(&fixture.manifest.components, &plan).is_err());
}

#[test]
fn xhl_planning_view_refuses_missing_or_wrong_mode_core_and_source_drift() {
    fn bytes_for(fixture: &Fixture) -> Vec<u8> {
        serde_json::to_vec_pretty(&fixture.manifest).unwrap()
    }
    fn rewrite_config(fixture: &mut Fixture, change: impl FnOnce(&mut Value)) {
        let root = fixture
            .manifest
            .root_files
            .iter_mut()
            .find(|file| file.target == "shroudforge/config/modloader-config.json")
            .unwrap();
        let mut value: Value = serde_json::from_slice(&fs::read(&root.source).unwrap()).unwrap();
        change(&mut value);
        fs::write(&root.source, serde_json::to_vec(&value).unwrap()).unwrap();
        root.stamp = stamp(&root.source).unwrap();
    }

    let mut missing_core = Fixture::new(true);
    rewrite_config(&mut missing_core, |config| {
        config["modules"]["dllManager"]["profiles"][0]["files"] = json!([
            {"path":"mods/XHL-Remote-Altar-Close/XHL-Remote-Altar-Close.dll","mode":"library"},
            {"path":"mods/XHL-Grappling-Hook-Pull-2x/XHL-Grappling-Hook-Pull-2x.dll","mode":"library"},
            {"path":"mods/XHL-Unlimited-Gifting-Range/XHL-Unlimited-Gifting-Range.dll","mode":"library"}
        ]);
    });
    let manifest = bytes_for(&missing_core);
    assert!(prepare_template(&manifest, &digest(&manifest)).is_err());
    assert!(!missing_core.output_root.join(".dll-planning-view").exists());

    let mut wrong_mode = Fixture::new(true);
    rewrite_config(&mut wrong_mode, |config| {
        config["modules"]["dllManager"]["profiles"][0]["files"][0]["mode"] = json!("library");
    });
    let manifest = bytes_for(&wrong_mode);
    assert!(prepare_template(&manifest, &digest(&manifest)).is_err());
    assert!(!wrong_mode.output_root.join(".dll-planning-view").exists());

    let mut drifted_source = Fixture::new(true);
    let core = drifted_source.xhl_source.join("xhl-core.dll");
    let mut image = fs::read(&core).unwrap();
    image[0] ^= 1;
    fs::write(&core, image).unwrap();
    let manifest = bytes_for(&drifted_source);
    assert!(prepare_template(&manifest, &digest(&manifest)).is_err());
    assert!(
        !drifted_source
            .output_root
            .join(".dll-planning-view")
            .exists()
    );
}

#[test]
fn disabled_legacy_gifting_selection_suppresses_only_its_patch_dll() {
    use shroudforge_package::dll_manager::{self, DllLoadMode};

    fn write_config(root: &Path, include_suppression: bool, enabled: bool, core_files: Vec<Value>) {
        let mut config = shroudforge_loader_config::config::read_loader_defaults().unwrap();
        config["modules"]["dllManager"]["enabled"] = json!(true);
        let mut profiles = vec![json!({
            "id":"xhl-core",
            "packageDirectory":"xhl-core",
            "name":"XHL Core",
            "target":"client",
            "enabled":true,
            "approved":true,
            "files":core_files
        })];
        if include_suppression {
            profiles.push(json!({
                "id":"legacy-xhl-gifting-disabled",
                "packageDirectory":"XHL-Unlimited-Gifting-Range",
                "name":"Disabled legacy Gifting DLL selection",
                "target":"client",
                "enabled":enabled,
                "approved":true,
                "compatibilityProfile":null,
                "files":[]
            }));
        }
        config["modules"]["dllManager"]["profiles"] = Value::Array(profiles);
        let directory = root.join("shroudforge/config");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("modloader-config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
    }

    let fixture = Fixture::new(false);
    let root = fixture.base.join("legacy-gifting-resolver");
    let core = root.join("mods/xhl-core");
    fs::create_dir_all(&core).unwrap();
    fs::write(core.join("xhl-core.dll"), b"inert Core image fixture").unwrap();
    let plugin_files = [
        "mods/XHL-Remote-Altar-Close/XHL-Remote-Altar-Close.dll",
        "mods/XHL-Grappling-Hook-Pull-2x/XHL-Grappling-Hook-Pull-2x.dll",
        "mods/XHL-Unlimited-Gifting-Range/XHL-Unlimited-Gifting-Range.dll",
    ];
    let mut core_files = vec![json!({"path":"xhl-core.dll","mode":"dinput8"})];
    for relative in plugin_files {
        let path = core.join(relative.replace('/', "\\"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"inert XHL plugin image fixture").unwrap();
        core_files.push(json!({"path":relative,"mode":"library"}));
    }
    let legacy = root.join("mods/XHL-Unlimited-Gifting-Range");
    fs::create_dir_all(&legacy).unwrap();
    fs::write(
        legacy.join("mod.json"),
        br#"{"id":"legacy.xhl-gifting","name":"Legacy Gifting","version":"0.4.0","capabilities":["patch"]}"#,
    )
    .unwrap();
    fs::write(
        legacy.join("extended.mod.json"),
        br#"{"schemaVersion":1,"enabled":true,"targets":["client"]}"#,
    )
    .unwrap();
    fs::write(
        legacy.join("native-plugin.ini"),
        b"[Plugin]\r\nEnabled=true\r\nDll=legacy_gifting.dll\r\n",
    )
    .unwrap();
    fs::write(
        legacy.join("legacy_gifting.dll"),
        b"inert DLL fixture, never loaded",
    )
    .unwrap();

    write_config(&root, true, false, core_files.clone());
    let disabled = dll_manager::resolve_load_plan(&root, false);
    assert!(disabled.issues.is_empty(), "{:?}", disabled.issues);
    let disabled_paths: BTreeSet<_> = disabled
        .entries
        .iter()
        .map(|entry| relative_native_path(&entry.path, &root).unwrap())
        .collect();
    assert!(disabled_paths.contains("mods/xhl-core/xhl-core.dll"));
    for relative in plugin_files {
        assert!(disabled_paths.contains(&format!("mods/xhl-core/{relative}")));
    }
    assert!(!disabled_paths.contains("mods/XHL-Unlimited-Gifting-Range/legacy_gifting.dll"));
    assert_eq!(
        disabled
            .entries
            .iter()
            .filter(|entry| entry.mode == DllLoadMode::Dinput8)
            .count(),
        1
    );

    write_config(&root, true, true, core_files.clone());
    let reenabled = dll_manager::resolve_load_plan(&root, false);
    assert!(reenabled.entries.iter().any(|entry| {
        relative_native_path(&entry.path, &root).unwrap() == "mods/xhl-core/xhl-core.dll"
    }));
    assert!(reenabled.entries.iter().any(|entry| {
        relative_native_path(&entry.path, &root).unwrap()
            == "mods/XHL-Unlimited-Gifting-Range/legacy_gifting.dll"
    }));

    write_config(&root, false, false, core_files);
    let missing_suppression = dll_manager::resolve_load_plan(&root, false);
    assert!(missing_suppression.entries.iter().any(|entry| {
        relative_native_path(&entry.path, &root).unwrap() == "mods/xhl-core/xhl-core.dll"
    }));
    assert!(missing_suppression.entries.iter().any(|entry| {
        relative_native_path(&entry.path, &root).unwrap()
            == "mods/XHL-Unlimited-Gifting-Range/legacy_gifting.dll"
    }));
}

#[test]
fn drive_prefix_comparison_rejects_unc_device_and_traversal_paths() {
    assert_eq!(
        normalize_absolute(Path::new(r"D:\Games\Enshrouded")).unwrap(),
        normalize_absolute(Path::new(r"\\?\D:\Games\Enshrouded")).unwrap()
    );
    for path in [
        r"\\server\share\Enshrouded",
        r"\\?\UNC\server\share\Enshrouded",
        r"\\.\D:\Games\Enshrouded",
        r"D:\Games\..\Enshrouded",
        r"D:\Games\.\Enshrouded",
        r"D:relative\Enshrouded",
    ] {
        assert!(normalize_absolute(Path::new(path)).is_err(), "{path}");
    }
}

#[test]
fn accepted_derived_cache_binding_adds_only_the_two_fixed_outputs() {
    let bridge_path = PathBuf::from(
        std::env::var_os("SF_DERIVED_CACHE_BRIDGE_MANIFEST")
            .expect("explicit E: cache bridge manifest is required"),
    );
    let verdict_path = PathBuf::from(
        std::env::var_os("SF_DERIVED_CACHE_TESTER_VERDICT")
            .expect("explicit E: cache tester verdict is required"),
    );
    let receipt_path = PathBuf::from(
        std::env::var_os("SF_DERIVED_CACHE_ASSET_RECEIPT")
            .expect("explicit E: prepared asset receipt is required"),
    );
    assert!(is_e_path(&bridge_path) && is_e_path(&verdict_path) && is_e_path(&receipt_path));
    let bridge_bytes = read_input_file(&bridge_path, 1024 * 1024).unwrap();
    assert_eq!(digest(&bridge_bytes), CACHE_BINDING_SHA256);
    let bridge: Value = serde_json::from_slice(&bridge_bytes).unwrap();
    let request_path = PathBuf::from(
        bridge
            .pointer("/request/path")
            .and_then(Value::as_str)
            .unwrap(),
    );
    let request_bytes = read_input_file(&request_path, 16 * 1024 * 1024).unwrap();
    let request: AssetRequest = serde_json::from_slice(&request_bytes).unwrap();
    let receipt_bytes = read_input_file(&receipt_path, 32 * 1024 * 1024).unwrap();
    let receipt: AssetReceipt = serde_json::from_slice(&receipt_bytes).unwrap();
    let mut fixture = Fixture::new(false);
    let mut manifest = fixture.manifest.clone();
    manifest.request = EvidencePin {
        path: request_path.clone(),
        sha256: digest(&request_bytes),
    };
    manifest.prepared_producer_closure = receipt.source_closure.clone();
    manifest.derived_environment_evidence = Some(DerivedEnvironmentEvidence {
        bridge_manifest: EvidencePin {
            path: bridge_path.clone(),
            sha256: CACHE_BINDING_SHA256.into(),
        },
        tester_verdict: EvidencePin {
            path: verdict_path,
            sha256: CACHE_VERDICT_SHA256.into(),
        },
    });
    let frozen_environment = request.environment_files.clone();
    let outputs =
        validate_derived_environment_evidence(&manifest, &request_bytes, &request, &receipt)
            .unwrap();
    assert_eq!(outputs.len(), 2);
    assert_eq!(request.environment_files, frozen_environment);
    assert_eq!(outputs[CACHE_IDENTITY_RELATIVE].bytes, CACHE_IDENTITY_BYTES);
    assert_eq!(outputs[CACHE_REGISTRY_RELATIVE].bytes, CACHE_REGISTRY_BYTES);
    for (relative, expected) in &outputs {
        let path = request.environment_root.join(relative.replace('/', "\\"));
        validate_e_path(&path, true, false).unwrap();
        assert_eq!(stamp(&path).unwrap(), *expected);
    }

    manifest.request.path = fixture.request_path.clone();
    assert!(
        validate_derived_environment_evidence(&manifest, &request_bytes, &request, &receipt,)
            .is_err()
    );
}
