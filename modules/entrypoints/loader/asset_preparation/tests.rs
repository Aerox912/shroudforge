use super::*;

#[test]
fn current_policy_failure_never_captures_or_restores_a_baseline() {
    let root=std::path::PathBuf::from("E:/Build/hdmap-r9-main-startup-compatibility-20261028-r1")
        .join(format!("outer-failure-{}",std::process::id()));
    std::fs::create_dir_all(root.join("shroudforge/config")).unwrap();
    for (name,bytes) in [("enshrouded.exe",b"synthetic executable".as_slice()),
        ("enshrouded.kfc",b"current composed index"),("enshrouded.kfc_resources",b"current composed resources"),
        ("enshrouded.kfc.bak",b"old baseline must never be restored")] {
        std::fs::write(root.join(name),bytes).unwrap();
    }
    std::fs::write(root.join("shroudforge/config/current-assets.json"),b"{}").unwrap();
    assert!(run_startup(&root).is_err());
    assert!(!recover_failed_startup(&root,"synthetic current-policy refusal"));
    assert_eq!(std::fs::read(root.join("enshrouded.kfc")).unwrap(),b"current composed index");
    assert_eq!(std::fs::read(root.join("enshrouded.kfc_resources")).unwrap(),b"current composed resources");
    assert!(!root.join("enshrouded.kfc_resources.bak").exists());
    assert!(!root.join("shroudforge/backups").exists());
    assert!(!shroudforge_game_assets::transaction::directory(&root,"enshrouded").exists());
}

#[test]
fn runtime_exports_do_not_restore_prepared_assets_during_startup() {
    let root = std::env::temp_dir().join(format!(
        "sf-startup-export-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    for (id, capabilities) in [
        ("production", serde_json::json!(["patch"])),
        ("editor", serde_json::json!(["runtime", "export"])),
    ] {
        let package = root.join("mods").join(id);
        std::fs::create_dir_all(package.join("src")).unwrap();
        std::fs::write(
            package.join("mod.json"),
            serde_json::to_vec(&serde_json::json!({
                "id": id, "name": id, "version": "1.0.0", "capabilities": capabilities
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            package.join("extended.mod.json"),
            r#"{"schemaVersion":1,"enabled":true,"targets":["client","server"]}"#,
        )
        .unwrap();
        std::fs::write(
            package.join("src/mod.lua"),
            "error('prepared assets must not be reapplied')",
        )
        .unwrap();
    }
    std::fs::write(root.join("enshrouded.exe"), b"test executable identity").unwrap();
    std::fs::write(root.join("enshrouded.kfc"), b"already prepared recipe data").unwrap();
    std::fs::write(root.join("enshrouded.kfc.bak"), b"original recipe data").unwrap();
    shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
    let environment = shroudforge_package::ModEnvironment::load(root.to_str().unwrap()).unwrap();
    assert_eq!(
        environment.plan(false, shroudforge_api::API_VERSION).len(),
        2
    );
    let fingerprint = shroudforge_package::prepared::fingerprint(
        &environment,
        false,
        shroudforge_api::API_VERSION,
    )
    .unwrap();
    shroudforge_loader_config::config::write_document(&root, shroudforge_game_assets::documents::APPLIED, &serde_json::json!({
            "schemaVersion": 1, "status": "applied", "target": "enshrouded", "fingerprint": fingerprint,
            "phase": "prepare", "completedAt": 1, "mods": [{"id":"production","version":"1.0.0"}]
        })).unwrap();
    assert!(!export_pass_needed(&root, "enshrouded").unwrap());
    run_startup(&root).unwrap();
    assert_eq!(
        std::fs::read(root.join("enshrouded.kfc")).unwrap(),
        b"already prepared recipe data"
    );
    // A real pregame exporter must still request its export pass.
    std::fs::write(
        root.join("mods/editor/mod.json"),
        r#"{"id":"editor","name":"editor","version":"1.0.0","capabilities":["export"]}"#,
    )
    .unwrap();
    assert!(export_pass_needed(&root, "enshrouded").unwrap());
}
