use crate::{
    API_VERSION, RunArgs, ShroudForgeApi,
    cache::{CacheDiff, FileStateCache},
    env::{AppFeatures, AppState},
    logging::debug,
    runner::LuaModRunner,
};
use mod_loader::ModEnvironment;
use std::path::{Path, PathBuf};

pub fn run(env: &ModEnvironment, api: ShroudForgeApi, args: RunArgs) -> anyhow::Result<()> {
    run_with_api(env, Some(api), args)
}

pub fn run_with_local_schema(env: &ModEnvironment, args: RunArgs) -> anyhow::Result<()> {
    run_with_api(env, None, args)
}

/// Run a selected pregame patch plan using separate mod/config, asset-input, and
/// generated-output roots.
/// The returned path names the private stage, which is deliberately not
/// publishable through the normal asset transaction entrypoints.
///
/// This entrypoint is for an isolated preparation worker. The output directory
/// must already exist, and all three roots must be pairwise disjoint. Logging
/// is process-global, so the worker must use no file logger or direct logs to a
/// separate E: logging directory outside these roots.
pub fn run_prepared(
    env: &ModEnvironment,
    api: ShroudForgeApi,
    args: RunArgs,
    asset_input_root: &Path,
    output_root: &Path,
) -> anyhow::Result<PathBuf> {
    run_prepared_with_api(env, Some(api), args, asset_input_root, output_root)
}

/// Local-schema form of [`run_prepared`].
pub fn run_prepared_with_local_schema(
    env: &ModEnvironment,
    args: RunArgs,
    asset_input_root: &Path,
    output_root: &Path,
) -> anyhow::Result<PathBuf> {
    run_prepared_with_api(env, None, args, asset_input_root, output_root)
}

fn run_prepared_with_api(
    env: &ModEnvironment,
    api: Option<ShroudForgeApi>,
    mut args: RunArgs,
    asset_input_root: &Path,
    output_root: &Path,
) -> anyhow::Result<PathBuf> {
    validate_prepared_options(&args)?;
    let stem = args.file_name.clone();
    // Prepared runs do not read or publish the ordinary installed-state cache.
    args.options.skip_cache = true;
    let (asset_input_root, output_root) =
        shroudforge_game_assets::transaction::validate_prepared_roots(
            env.game_dir().as_std_path(),
            asset_input_root,
            output_root,
            &args.file_name,
        )?;
    run_with_api_and_output(env, api, args, Some((&asset_input_root, &output_root)))?;
    Ok(shroudforge_game_assets::transaction::directory(
        &output_root,
        &stem,
    ))
}

fn validate_prepared_options(args: &RunArgs) -> anyhow::Result<()> {
    let selected = args.options.current_asset_mods.as_ref();
    anyhow::ensure!(
        selected.is_some_and(|ids| !ids.is_empty()),
        "prepared generation requires a selected pregame asset mod set"
    );
    anyhow::ensure!(
        args.options.phase == crate::RuntimePhase::Pregame && args.options.patch,
        "prepared generation requires writing pregame patching"
    );
    anyhow::ensure!(
        args.options.defer_current_commit,
        "prepared generation requires deferred asset publication"
    );
    anyhow::ensure!(
        !args.options.export && args.options.export_dir.is_none(),
        "prepared generation requires exports disabled and no export directory"
    );
    Ok(())
}

fn run_with_api(
    env: &ModEnvironment,
    api: Option<ShroudForgeApi>,
    args: RunArgs,
) -> anyhow::Result<()> {
    run_with_api_and_output(env, api, args, None)
}

fn run_with_api_and_output(
    env: &ModEnvironment,
    api: Option<ShroudForgeApi>,
    mut args: RunArgs,
    prepared_roots: Option<(&Path, &Path)>,
) -> anyhow::Result<()> {
    debug!("Running lua with options: {:?}", args);
    if prepared_roots.is_some() {
        validate_prepared_options(&args)?;
        args.options.skip_cache = true;
    }
    let selected = args.options.current_asset_mods.clone();
    let selective_generation = selected.is_some();
    if let Some(ids) = &selected {
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        anyhow::ensure!(
            !ids.is_empty() && unique.len() == ids.len(),
            "empty or duplicate current asset mod selection"
        );
        anyhow::ensure!(
            args.options.phase == crate::RuntimePhase::Pregame && args.options.patch,
            "current asset selection requires a writing pregame session"
        );
        // This mode is invoked only after the caller's exact prepared-state check.
        args.options.force_patch = true;
    }

    // check cache if files have changed

    let cache_diff = if !args.options.skip_cache {
        let current_cache = FileStateCache::read(env.cache_dir());
        let mut new_cache = FileStateCache::default();

        new_cache.track_game_files(env.game_dir());
        new_cache.track_mod_files(env.mods_dir());
        new_cache.track_mod_config(env.game_dir());

        let cache_diff = new_cache.diff(&current_cache);

        if cache_diff.is_none() && args.options.patch && !args.options.force_patch {
            args.options.patch = false;

            debug!("No changes detected, skipping asset writes");
        }

        cache_diff
    } else {
        debug!("Skipping cache check");
        CacheDiff::new_dirty()
    };

    // create a new app state

    let created_app_state = match prepared_roots {
        Some((asset_input_root, output_root)) => AppState::new_prepared(
            env.clone(),
            api,
            args,
            &cache_diff,
            asset_input_root,
            output_root,
        ),
        None => AppState::new(env.clone(), api, args, &cache_diff),
    };
    let app_state = match created_app_state {
        Ok(context) => context,
        Err(_) => anyhow::bail!("Failed to create AppState"),
    };

    // setup and run all mods

    let runner = LuaModRunner::new(app_state)?;

    let is_server = runner.lua.app_data_ref::<AppState>().unwrap().is_server();
    let mut plan = env.plan(is_server, API_VERSION);
    if let Some(ids) = selected {
        anyhow::ensure!(
            ids.iter().all(|id| plan
                .iter()
                .any(|m| &m.info().id == id && m.info().requires_pregame())),
            "current asset selection contains an unavailable pregame mod"
        );
        plan.retain(|m| ids.contains(&m.info().id));
    }
    match runner.setup(plan) {
        Ok(_) => {}
        Err(err) => {
            anyhow::bail!("Failed to setup LuaModRunner: {}", err);
        }
    }

    debug!("Running mods...");

    // TODO: move this somewhere else, maybe to the GameContext?

    runner.run()?;

    let app_state = runner.lua.app_data_ref::<AppState>().unwrap();

    super::commit_asset_changes::commit_asset_changes(&runner.lua, &app_state)?;

    // A partial plan cannot acknowledge inputs belonging to mods it did not run.
    // Its caller owns a separately scoped prepared-state record; leave the
    // ordinary all-mod applied cache byte-for-byte unchanged.
    if selective_generation {
        return Ok(());
    }

    // create a new cache file

    if app_state.has_feature(AppFeatures::PATCH) {
        let mut new_cache = FileStateCache::default();

        new_cache.track_game_files(env.game_dir());
        new_cache.track_mod_files(env.mods_dir());
        new_cache.track_mod_config(env.game_dir());

        new_cache.write(env.cache_dir());
    } else {
        let mut new_cache = FileStateCache::read(env.cache_dir());

        new_cache.track_game_files(env.game_dir());
        new_cache.track_mod_config(env.game_dir());
        new_cache.write(env.cache_dir());
    }

    Ok(())
}

#[cfg(test)]
mod prepared_run_tests {
    use super::*;
    use kfc::{
        container::{KFCFile, KFCReader, KFCWriter},
        guid::{Guid, ResourceId},
    };
    use sha2::Digest;
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path as StdPath, PathBuf},
    };

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("shroudforge-prepared-run-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Fixture {
        _root: TempRoot,
        source: PathBuf,
        environment_root: PathBuf,
        output: PathBuf,
        environment: ModEnvironment,
        resource: ResourceId,
        api: ShroudForgeApi,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempRoot::new();
            let source = temp.0.join("source");
            let environment_root = temp.0.join("staged-environment");
            let output = temp.0.join("prepared-output");
            fs::create_dir_all(&source).unwrap();
            fs::create_dir_all(&environment_root).unwrap();
            fs::create_dir_all(&output).unwrap();
            write_mod(
                &environment_root,
                "selected-writer",
                r#"
                    local resource = game.assets.get_resource(
                        "00000000-0000-0000-0000-000000000001", "test::U32")
                    assert(resource ~= nil, "selected resource is available")
                    assert(resource.data == 7, "selected resource preimage is visible")
                    resource.data = 29
                    return {}
                "#,
            );
            write_mod(
                &environment_root,
                "unselected-failure",
                "error('unselected pregame mod executed')",
            );
            write_mod(
                &environment_root,
                "selected-failure",
                "error('selected pregame mod failed')",
            );

            let source = fs::canonicalize(source).unwrap();
            let environment_root = fs::canonicalize(environment_root).unwrap();
            let environment =
                mod_loader::ModEnvironment::load(environment_root.to_str().unwrap()).unwrap();
            let mut registry = crate::env::registry::tests::fixture();
            registry.version = "prepared-session-test".into();
            let executable = b"prepared synthetic executable identity";
            fs::write(source.join("enshrouded.exe"), executable).unwrap();
            fs::write(source.join("enshrouded.kfc"), []).unwrap();
            fs::write(source.join("enshrouded.kfc_resources"), []).unwrap();

            let metadata = registry
                .iter()
                .find(|metadata| metadata.qualified_name == "test::U32")
                .unwrap();
            let resource = ResourceId::new(
                Guid::parse("00000000-0000-0000-0000-000000000001").unwrap(),
                metadata.qualified_hash,
                0,
            );
            let mut writer =
                KFCWriter::<KFCFile, _>::new(&source, "enshrouded", &registry, &registry.version)
                    .unwrap();
            writer
                .write_resource(&resource, &7_u32.to_le_bytes())
                .unwrap();
            writer.finalize().unwrap();

            fs::create_dir_all(environment.cache_dir().as_std_path()).unwrap();
            fs::write(
                environment.cache_dir().join("types-enshrouded.json"),
                serde_json::to_vec(&registry).unwrap(),
            )
            .unwrap();
            let identity = serde_json::json!({
                "schemaVersion": 2,
                "executableSha256": format!("{:x}", sha2::Sha256::digest(executable)),
                "reflectionExtractor": "kfc-a404b15cde40fc7bd43d812bb460ce40d2a9d3b1"
            });
            fs::write(
                environment
                    .cache_dir()
                    .join("types-enshrouded.identity.json"),
                serde_json::to_vec(&identity).unwrap(),
            )
            .unwrap();

            let source_config = source.join("shroudforge");
            fs::create_dir_all(source_config.join("cache")).unwrap();
            fs::create_dir_all(source_config.join("exports")).unwrap();
            fs::create_dir_all(source_config.join("reports")).unwrap();
            fs::create_dir_all(source_config.join("logs")).unwrap();
            fs::write(source_config.join("cache/keep.json"), b"source cache").unwrap();
            fs::write(source_config.join("exports/keep.txt"), b"source export").unwrap();
            fs::write(source_config.join("reports/keep.json"), b"source report").unwrap();
            fs::write(source_config.join("logs/keep.log"), b"source log").unwrap();

            let config_root = environment_root.join("shroudforge");
            fs::create_dir_all(config_root.join("exports")).unwrap();
            fs::create_dir_all(config_root.join("reports")).unwrap();
            fs::create_dir_all(config_root.join("logs")).unwrap();
            fs::write(config_root.join("exports/keep.txt"), b"existing export").unwrap();
            fs::write(config_root.join("reports/keep.json"), b"existing report").unwrap();
            fs::write(config_root.join("logs/keep.log"), b"existing log").unwrap();
            fs::write(output.join("keep.txt"), b"output sentinel").unwrap();

            let game_file = KFCFile::from_path(source.join("enshrouded.kfc"), false).unwrap();
            let schema = kfc::schema_from_registry(&registry, &game_file);
            let api = ShroudForgeApi::new(
                shroudforge_compatibility::Compatibility::default().resolve(schema),
            );

            Self {
                _root: temp,
                source,
                environment_root,
                output,
                environment,
                resource,
                api,
            }
        }

        fn args(&self) -> RunArgs {
            RunArgs {
                file_name: "enshrouded".into(),
                options: crate::RunOptions {
                    current_asset_mods: Some(vec!["selected-writer".into()]),
                    defer_current_commit: true,
                    // The prepared entry must bypass the installed-state cache.
                    skip_cache: false,
                    is_server: Some(false),
                    patch: true,
                    export: false,
                    phase: crate::RuntimePhase::Pregame,
                    ..Default::default()
                },
            }
        }
    }

    fn write_mod(source: &StdPath, id: &str, lua: &str) {
        let package = source.join("mods").join(id);
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(
            package.join("mod.json"),
            serde_json::to_vec(&serde_json::json!({
                "id": id,
                "name": id,
                "version": "1.0.0",
                "dependencies": [],
                "capabilities": ["patch"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            package.join("extended.mod.json"),
            br#"{"schemaVersion":1,"enabled":true,"targets":["client"]}"#,
        )
        .unwrap();
        fs::write(package.join("src/mod.lua"), lua).unwrap();
    }

    fn snapshot(root: &StdPath) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        fn visit(
            root: &StdPath,
            directory: &StdPath,
            entries: &mut BTreeMap<PathBuf, Option<Vec<u8>>>,
        ) {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                let metadata = entry.metadata().unwrap();
                if metadata.is_dir() {
                    entries.insert(relative, None);
                    visit(root, &path, entries);
                } else {
                    entries.insert(relative, Some(fs::read(path).unwrap()));
                }
            }
        }

        let mut entries = BTreeMap::new();
        visit(root, root, &mut entries);
        entries
    }

    fn args_with_export_directory(fixture: &Fixture) -> RunArgs {
        let mut args = fixture.args();
        args.options.export_dir = Some(PathBuf::from("new-export-directory"));
        args
    }

    #[test]
    fn prepared_entry_runs_only_selected_lua_and_writes_outside_source() {
        let fixture = Fixture::new();
        let source_before = snapshot(&fixture.source);
        let environment_before = snapshot(&fixture.environment_root);
        let output_before = snapshot(&fixture.output);

        assert!(
            run_prepared(
                &fixture.environment,
                fixture.api.clone(),
                args_with_export_directory(&fixture),
                &fixture.source,
                &fixture.output,
            )
            .is_err()
        );
        assert_eq!(snapshot(&fixture.source), source_before);
        assert_eq!(snapshot(&fixture.environment_root), environment_before);
        assert!(!fixture.source.join("new-export-directory").exists());
        assert!(
            !fixture
                .environment_root
                .join("new-export-directory")
                .exists()
        );

        let failed_output = fixture._root.0.join("failed-output");
        fs::create_dir_all(&failed_output).unwrap();
        fs::write(failed_output.join("keep.txt"), b"failed output sentinel").unwrap();
        let failed_output_before = snapshot(&failed_output);
        let mut failing_args = fixture.args();
        failing_args.options.current_asset_mods = Some(vec!["selected-failure".into()]);
        assert!(
            run_prepared(
                &fixture.environment,
                fixture.api.clone(),
                failing_args,
                &fixture.source,
                &failed_output,
            )
            .is_err()
        );
        assert_eq!(snapshot(&fixture.source), source_before);
        assert_eq!(snapshot(&fixture.environment_root), environment_before);
        assert!(
            shroudforge_game_assets::transaction::directory(&failed_output, "enshrouded")
                .join("prepared-only.json")
                .is_file()
        );
        assert!(
            !shroudforge_game_assets::transaction::directory(&fixture.source, "enshrouded")
                .exists()
        );
        let mut failed_output_after = snapshot(&failed_output);
        let failed_stage_name = PathBuf::from(
            shroudforge_game_assets::transaction::directory(&failed_output, "enshrouded")
                .file_name()
                .unwrap()
                .to_os_string(),
        );
        failed_output_after.retain(|path, _| !path.starts_with(&failed_stage_name));
        assert_eq!(failed_output_after, failed_output_before);

        let stage = run_prepared(
            &fixture.environment,
            fixture.api.clone(),
            fixture.args(),
            &fixture.source,
            &fixture.output,
        )
        .unwrap();
        assert!(!fixture.source.join("mods/selected-writer").exists());
        assert!(
            fixture
                .environment_root
                .join("mods/selected-writer/src/mod.lua")
                .is_file()
        );
        assert!(stage.starts_with(fs::canonicalize(&fixture.output).unwrap()));
        assert!(stage.join("enshrouded.kfc").is_file());
        assert!(stage.join("enshrouded.kfc_resources").is_file());
        assert!(stage.join("prepared-only.json").is_file());
        let stage_names = fs::read_dir(&stage)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(!stage_names.iter().any(|name| name.ends_with(".rollback")));

        let mut reader = KFCReader::new(&stage, "enshrouded")
            .unwrap()
            .into_cursor()
            .unwrap();
        assert_eq!(
            reader.read_resource(&fixture.resource).unwrap().unwrap(),
            29_u32.to_le_bytes()
        );
        assert_eq!(snapshot(&fixture.source), source_before);
        assert_eq!(snapshot(&fixture.environment_root), environment_before);
        assert!(
            !shroudforge_game_assets::transaction::directory(&fixture.source, "enshrouded")
                .exists()
        );

        let mut output_after = snapshot(&fixture.output);
        let stage_name = PathBuf::from(
            shroudforge_game_assets::transaction::directory(&fixture.output, "enshrouded")
                .file_name()
                .unwrap()
                .to_os_string(),
        );
        output_after.retain(|path, _| !path.starts_with(&stage_name));
        assert_eq!(output_after, output_before);

        let source_before_replay = snapshot(&fixture.source);
        let environment_before_replay = snapshot(&fixture.environment_root);
        let output_before_replay = snapshot(&fixture.output);
        assert!(
            run_prepared(
                &fixture.environment,
                fixture.api.clone(),
                fixture.args(),
                &fixture.source,
                &fixture.output,
            )
            .is_err()
        );
        assert_eq!(snapshot(&fixture.source), source_before_replay);
        assert_eq!(
            snapshot(&fixture.environment_root),
            environment_before_replay
        );
        assert_eq!(snapshot(&fixture.output), output_before_replay);

        let source_transaction =
            shroudforge_game_assets::transaction::begin_generated(&fixture.source, "enshrouded")
                .unwrap();
        fs::write(
            source_transaction.join("foreign.settings"),
            b"source transaction must not be recovered",
        )
        .unwrap();
        let blocked_source_before = snapshot(&fixture.source);
        let blocked_environment_before = snapshot(&fixture.environment_root);
        let blocked_output = fixture._root.0.join("blocked-output");
        fs::create_dir_all(&blocked_output).unwrap();
        let blocked_output_before = snapshot(&blocked_output);
        assert!(
            run_prepared(
                &fixture.environment,
                fixture.api.clone(),
                fixture.args(),
                &fixture.source,
                &blocked_output,
            )
            .is_err()
        );
        assert_eq!(snapshot(&fixture.source), blocked_source_before);
        assert_eq!(
            snapshot(&fixture.environment_root),
            blocked_environment_before
        );
        assert_eq!(snapshot(&blocked_output), blocked_output_before);
    }
}
