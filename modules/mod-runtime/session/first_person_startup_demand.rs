use mod_loader::{Capability, ModEnvironment, package_lookup};

const FIRST_PERSON_MOD_ID: &str = "shroudweave-first-person";

/// Report whether the enabled first-person package is part of the actual
/// client runtime plan that must be prepared before releasing the entrypoint.
/// Enabling it after startup requires a game restart.
pub(crate) fn first_person_startup_demand(
    environment: &ModEnvironment,
) -> Result<bool, String> {
    let (runtime_plan, issues) =
        environment.runtime_plan_report_detailed(false, crate::API_VERSION);
    let first_person = environment.mod_registry().get(FIRST_PERSON_MOD_ID);

    let Some(first_person) = first_person else {
        if package_lookup::find_mod_package(
            environment.game_dir().as_std_path(),
            FIRST_PERSON_MOD_ID,
        )
        .is_some()
        {
            return Err(format!(
                "installed {FIRST_PERSON_MOD_ID} package was rejected by the canonical package loader"
            ));
        }

        // The package ID is also the directory name in the supported package
        // layout. This catches malformed mod.json files that the canonical
        // ID-based lookup cannot decode without adding a second manifest parser.
        match std::fs::metadata(
            environment
                .mods_dir()
                .as_std_path()
                .join(FIRST_PERSON_MOD_ID),
        ) {
            Ok(_) => {
                return Err(format!(
                    "installed {FIRST_PERSON_MOD_ID} package could not be admitted by the canonical package loader"
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not inspect installed {FIRST_PERSON_MOD_ID} package path: {error}"
                ));
            }
        }

        if let Some(issue) = relevant_plan_issue(&issues) {
            return Err(format!(
                "could not resolve absent {FIRST_PERSON_MOD_ID} startup demand because the runtime plan is incomplete: {}",
                issue.detail
            ));
        }
        return Ok(false);
    };

    let info = first_person.info();
    if !info.enabled
        || !info.target.supports_process(false)
        || !info.capabilities.contains(&Capability::Runtime)
    {
        return Ok(false);
    }

    if let Some(issue) = relevant_plan_issue(&issues) {
        return Err(format!(
            "enabled {FIRST_PERSON_MOD_ID} package is unavailable in the client runtime plan: {}",
            issue.detail
        ));
    }

    if runtime_plan
        .iter()
        .any(|package| package.info().id == FIRST_PERSON_MOD_ID)
    {
        return Ok(true);
    }

    let detail = if issues.is_empty() {
        "the canonical plan produced no matching package entry".to_owned()
    } else {
        format!(
            "the canonical plan produced no matching package entry (other issues: {})",
            issues
                .iter()
                .map(|issue| format!("{}: {}", issue.mod_id, issue.detail))
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    Err(format!(
        "enabled {FIRST_PERSON_MOD_ID} package did not qualify for the client runtime plan: {detail}"
    ))
}

fn relevant_plan_issue(issues: &[mod_loader::ModPlanIssue]) -> Option<&mod_loader::ModPlanIssue> {
    issues
        .iter()
        .find(|issue| issue.mod_id == FIRST_PERSON_MOD_ID || issue.mod_id == "__loader__")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, path::PathBuf, time::SystemTime};

    struct FixtureRoot(PathBuf);

    impl FixtureRoot {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system clock after Unix epoch")
                .as_nanos();
            let base = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let root = base.join(format!("sf-first-person-demand-{suffix}"));
            fs::create_dir_all(&root).unwrap();
            shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
            Self(root)
        }

        fn write_package(
            &self,
            directory: &str,
            id: &str,
            enabled: bool,
            target: &str,
            capabilities: &[&str],
            dependencies: serde_json::Value,
        ) -> PathBuf {
            let package = self.0.join("mods").join(directory);
            fs::create_dir_all(package.join("src")).unwrap();
            fs::write(package.join("src/mod.lua"), "return {}\n").unwrap();
            fs::write(
                package.join("mod.json"),
                serde_json::to_vec(&json!({
                    "id": id,
                    "name": "Fixture package",
                    "version": "1.0.0",
                    "capabilities": capabilities,
                    "dependencies": dependencies,
                }))
                .unwrap(),
            )
            .unwrap();
            fs::write(
                package.join("extended.mod.json"),
                serde_json::to_vec(&json!({
                    "schemaVersion": 1,
                    "enabled": enabled,
                    "targets": [target],
                }))
                .unwrap(),
            )
            .unwrap();
            package
        }

        fn environment(&self) -> ModEnvironment {
            ModEnvironment::load(camino::Utf8Path::from_path(&self.0).unwrap()).unwrap()
        }
    }

    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn first_person_startup_demand_is_false_for_absent_disabled_server_and_nonruntime_packages() {
        let absent = FixtureRoot::new();
        assert!(!first_person_startup_demand(&absent.environment()).unwrap());

        for (enabled, target, capabilities) in [
            (false, "client", vec!["runtime"]),
            (true, "server", vec!["runtime"]),
            (true, "client", vec![]),
            (true, "client", vec!["runtime-register-dll"]),
        ] {
            let fixture = FixtureRoot::new();
            fixture.write_package(
                "first-person",
                FIRST_PERSON_MOD_ID,
                enabled,
                target,
                &capabilities,
                json!([]),
            );
            assert!(
                !first_person_startup_demand(&fixture.environment()).unwrap(),
                "enabled={enabled} target={target} capabilities={capabilities:?}"
            );
        }
    }

    #[test]
    fn first_person_startup_demand_is_true_only_when_present_in_the_client_runtime_plan() {
        let fixture = FixtureRoot::new();
        fixture.write_package(
            "first-person",
            FIRST_PERSON_MOD_ID,
            true,
            "client",
            &["runtime"],
            json!([]),
        );
        let environment = fixture.environment();
        assert!(first_person_startup_demand(&environment).unwrap());
    }

    #[test]
    fn first_person_startup_demand_reports_a_rejected_installed_package() {
        for invalid_file in ["extended.mod.json", "mod.json"] {
            let fixture = FixtureRoot::new();
            let package = fixture.write_package(
                FIRST_PERSON_MOD_ID,
                FIRST_PERSON_MOD_ID,
                true,
                "client",
                &["runtime"],
                json!([]),
            );
            fs::write(package.join(invalid_file), "{ invalid json").unwrap();
            let error = first_person_startup_demand(&fixture.environment()).unwrap_err();
            assert!(error.contains(FIRST_PERSON_MOD_ID), "{error}");
            assert!(error.contains("rejected") || error.contains("admitted"), "{error}");
        }
    }

    #[test]
    fn first_person_startup_demand_reports_a_failed_first_person_runtime_plan() {
        let fixture = FixtureRoot::new();
        fixture.write_package(
            "first-person",
            FIRST_PERSON_MOD_ID,
            true,
            "client",
            &["runtime"],
            json!([{ "id": "missing-required", "version": "^1.0.0" }]),
        );
        let error = first_person_startup_demand(&fixture.environment()).unwrap_err();
        assert!(error.contains(FIRST_PERSON_MOD_ID), "{error}");
        assert!(error.contains("plan"), "{error}");
    }

    #[test]
    fn unrelated_plan_issues_do_not_change_present_first_person_demand() {
        let fixture = FixtureRoot::new();
        fixture.write_package(
            "first-person",
            FIRST_PERSON_MOD_ID,
            true,
            "client",
            &["runtime"],
            json!([]),
        );
        fixture.write_package(
            "unrelated-broken",
            "unrelated-broken",
            true,
            "client",
            &["runtime"],
            json!([{ "id": "missing-required", "version": "^1.0.0" }]),
        );
        let environment = fixture.environment();
        assert!(first_person_startup_demand(&environment).unwrap());
    }
}
