use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn control_snapshot_reads_only_admitted_bytes_or_proved_absence_and_route_is_explicit() {
    let fixture = Fixture::new();
    let mut lease = SourceLease::admit(fixture.plan.clone(), fixture.limits()).unwrap();
    assert_eq!(
        lease.read_control("control.json", 1024).unwrap().unwrap(),
        b"{\"mode\":1}"
    );
    assert!(
        lease
            .read_control("shroudforge/config/modloader-config.json", 1024)
            .unwrap()
            .is_none()
    );
    assert!(lease.read_control("control.json", 1).is_err());
    assert!(lease.read_control("control.json/child", 1024).is_err());
    assert!(lease.read_control("mods", 1024).is_err());
    assert!(lease.read_control("../escape", 1024).is_err());
    let root = fixture.plan.environment_root.clone();
    let env = shroudforge_package::ModEnvironment::from_prepared_packages(
        camino::Utf8PathBuf::from_path_buf(root.clone()).unwrap(),
        camino::Utf8PathBuf::from_path_buf(root.join("cache")).unwrap(),
        camino::Utf8PathBuf::from_path_buf(root.join("mods")).unwrap(),
        lease.package_views().unwrap(),
        lease.manifest_policy(),
    )
    .unwrap();
    assert!(env.has_prepared_sources());
    assert!(
        !shroudforge_package::ModEnvironment::load(root.to_str().unwrap())
            .unwrap()
            .has_prepared_sources()
    );
    assert!(lease.finalize().unwrap().verified);
    assert!(lease.read_control("control.json", 1024).is_err());
}

#[test]
fn lua_size_cap_rejects_raw_or_override_before_any_source_open() {
    let fixture = Fixture::new();
    let mut plan = fixture.plan.clone();
    plan.raw_files
        .get_mut("mods/example/src/profile.lua")
        .unwrap()
        .bytes = 8 * 1024 * 1024 + 1;
    reseal(&mut plan);
    assert!(plan.validate().is_err());
    let mut plan = fixture.plan;
    plan.replacements[0].replacement.bytes = 8 * 1024 * 1024 + 1;
    reseal(&mut plan);
    assert!(plan.validate().is_err());
}

fn stamp(raw: &[u8]) -> Stamp {
    Stamp {
        bytes: raw.len() as u64,
        sha256: format!("{:x}", Sha256::digest(raw)),
    }
}

struct Fixture {
    plan: SourcePlan,
    outer: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let outer = PathBuf::from(r"E:\Build\sf178-source-lease-synthetic-fixtures-20261104-r1")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        assert!(!outer.exists());
        let root = outer.join("environment");
        fs::create_dir_all(root.join("mods/example/src")).unwrap();
        let inputs=[("control.json",b"{\"mode\":1}".as_slice()),
            ("mods/example/mod.json",b"{\"id\":\"example\",\"name\":\"Example\",\"version\":\"1.0.0\",\"capabilities\":[\"runtime\"]}"),
            ("mods/example/extended.mod.json",b"{\"schemaVersion\":1,\"enabled\":true,\"settings\":{\"enabledFeature\":{\"value\":true,\"label\":\"Enabled\",\"control\":\"toggle\"}}}"),
            ("mods/example/src/profile.lua",b"return {}")];
        let mut raw_files = BTreeMap::new();
        for (name, raw) in inputs {
            fs::write(root.join(name), raw).unwrap();
            raw_files.insert(name.into(), stamp(raw));
        }
        let replacement = b"return {on_update=function()end}";
        let path = outer.join("profile-successor.lua");
        fs::write(&path, replacement).unwrap();
        let logical = "mods/example/src/profile.lua";
        let mut plan = SourcePlan {
            schema: 1,
            kind: "prepared_effective_source_plan_v1".into(),
            environment_root: root.clone(),
            raw_files,
            entries: inventory(&root, 20_000).unwrap(),
            package_roots: vec!["mods/example".into()],
            replacements: vec![Replacement {
                logical_path: logical.into(),
                raw: stamp(b"return {}"),
                path,
                replacement: stamp(replacement),
            }],
            profile_policy: ProfilePolicy::RejectAmbientProfiles,
            allow_unknown_manifest_fields: true,
            raw_inventory_sha256: String::new(),
            effective_inventory_sha256: String::new(),
        };
        reseal(&mut plan);
        Self { plan, outer }
    }
    fn limits(&self) -> ReadLimits {
        let bytes = self.plan.required_final_bytes().unwrap();
        let calls = self.plan.required_final_calls().unwrap();
        ReadLimits {
            max_bytes: bytes * 10 + 100_000,
            max_calls: calls * 10 + 100,
            final_reserve_bytes: bytes,
            final_reserve_calls: calls,
        }
    }
}
fn reseal(plan: &mut SourcePlan) {
    plan.raw_inventory_sha256 = plan.inventory_digest(false).unwrap();
    plan.effective_inventory_sha256 = plan.inventory_digest(true).unwrap();
}

#[test]
fn admitted_view_uses_replacement_everywhere_and_finalizes_raw_and_effective_once() {
    let fixture = Fixture::new();
    let raw_before = fs::read(
        fixture
            .plan
            .environment_root
            .join("mods/example/src/profile.lua"),
    )
    .unwrap();
    let mut lease = SourceLease::admit(fixture.plan.clone(), fixture.limits()).unwrap();
    let root = fixture.plan.environment_root.clone();
    let env = shroudforge_package::ModEnvironment::from_prepared_packages(
        camino::Utf8PathBuf::from_path_buf(root.clone()).unwrap(),
        camino::Utf8PathBuf::from_path_buf(root.join("cache")).unwrap(),
        camino::Utf8PathBuf::from_path_buf(root.join("mods")).unwrap(),
        lease.package_views().unwrap(),
        lease.manifest_policy(),
    )
    .unwrap();
    let example = env.mod_registry().get("example").unwrap();
    assert_eq!(example.info().settings[0].apply_at, "live");
    let mut actual = String::new();
    example
        .fs()
        .read_file("src/profile.lua")
        .unwrap()
        .read_to_string(&mut actual)
        .unwrap();
    assert_eq!(actual, "return {on_update=function()end}");
    assert!(
        example
            .fs()
            .absolute_path("src/profile.lua")
            .unwrap()
            .is_none()
    );
    let result = lease.finalize().unwrap();
    assert!(result.verified);
    assert_eq!(result.files.len(), 5);
    assert!(
        result
            .files
            .iter()
            .all(|row| row.identity_matched == Some(true) && row.hash_matched == Some(true))
    );
    assert!(lease.finalize().is_err());
    assert!(lease.package_views().is_err());
    assert_eq!(
        fs::read(root.join("mods/example/src/profile.lua")).unwrap(),
        raw_before
    );
    assert!(!root.join("cache").exists());
    assert!(!fixture.outer.join("output").exists());
}

#[test]
fn entire_raw_control_graph_is_write_locked_and_new_control_breaks_final_membership() {
    let fixture = Fixture::new();
    let mut lease = SourceLease::admit(fixture.plan.clone(), fixture.limits()).unwrap();
    assert!(
        OpenOptions::new()
            .write(true)
            .open(fixture.plan.environment_root.join("control.json"))
            .is_err()
    );
    fs::write(
        fixture.plan.environment_root.join("new-control.json"),
        b"{}",
    )
    .unwrap();
    let result = lease.finalize().unwrap();
    assert!(!result.verified);
    assert_eq!(result.membership_matched, Some(false));
    assert!(
        result
            .files
            .iter()
            .all(|row| row.hash_matched == Some(true))
    );
}

#[test]
fn malformed_namespace_origin_and_digest_fail_before_admission() {
    let fixture = Fixture::new();
    let mut plan = fixture.plan.clone();
    plan.entries.push("../escape".into());
    assert!(plan.validate().is_err());
    let mut plan = fixture.plan.clone();
    plan.entries.push("CONTROL.JSON".into());
    reseal(&mut plan);
    assert!(plan.validate().is_err());
    let mut plan = fixture.plan.clone();
    plan.replacements[0].raw = stamp(b"wrong");
    reseal(&mut plan);
    assert!(plan.validate().is_err());
    let mut plan = fixture.plan.clone();
    plan.replacements.push(plan.replacements[0].clone());
    reseal(&mut plan);
    assert!(plan.validate().is_err());
    let mut plan = fixture.plan.clone();
    plan.effective_inventory_sha256 = "0".repeat(64);
    assert!(plan.validate().is_err());
    let mut plan = fixture.plan.clone();
    plan.package_roots.push("mods/missing".into());
    reseal(&mut plan);
    assert!(plan.validate().is_err());
    assert!(
        SourceLease::admit(
            fixture.plan,
            ReadLimits {
                max_bytes: 0,
                max_calls: 0,
                final_reserve_bytes: 0,
                final_reserve_calls: 0
            }
        )
        .is_err()
    );
}

#[test]
fn initial_hash_failure_keeps_primary_and_partial_finalization_without_reset_or_fallback() {
    let mut fixture = Fixture::new();
    fixture
        .plan
        .raw_files
        .get_mut("control.json")
        .unwrap()
        .sha256 = "0".repeat(64);
    reseal(&mut fixture.plan);
    let error = SourceLease::admit(fixture.plan.clone(), fixture.limits())
        .err()
        .unwrap();
    let failure = error.downcast_ref::<SourceAdmissionFailure>().unwrap();
    assert!(failure.primary.contains("held source hash"));
    assert!(!failure.finalization.verified);
    let first = failure
        .finalization
        .files
        .iter()
        .find(|row| row.name == "control.json")
        .unwrap();
    assert_eq!(first.identity_matched, Some(true));
    assert_eq!(first.hash_matched, Some(false));
    assert!(
        failure
            .finalization
            .files
            .iter()
            .any(|row| row.identity_matched.is_none() && row.hash_matched.is_none())
    );
    assert_eq!(
        failure.finalization.charged_bytes,
        fixture.plan.raw_files["control.json"].bytes * 2
    );
}

#[test]
fn physical_alias_and_unadmitted_member_are_not_treated_as_another_valid_source() {
    let mut fixture = Fixture::new();
    let alias = fixture.plan.environment_root.join("alias.json");
    fs::hard_link(fixture.plan.environment_root.join("control.json"), &alias).unwrap();
    assert!(SourceLease::admit(fixture.plan.clone(), fixture.limits()).is_err());
    fixture.plan.raw_files.insert(
        "alias.json".into(),
        fixture.plan.raw_files["control.json"].clone(),
    );
    fixture.plan.entries = inventory(&fixture.plan.environment_root, 20_000).unwrap();
    reseal(&mut fixture.plan);
    let error = SourceLease::admit(fixture.plan.clone(), fixture.limits())
        .err()
        .unwrap();
    let failure = error.downcast_ref::<SourceAdmissionFailure>().unwrap();
    assert!(failure.primary.contains("physical source alias"));
    assert!(!failure.finalization.verified);
}
