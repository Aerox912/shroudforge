use crate::DiagnosticExecution;
use mlua::{DeserializeOptions, Lua, LuaSerdeExt, SerializeOptions};

fn install_report_codec(lua: &Lua) -> mlua::Result<()> {
    lua.globals().set(
        "encode_diagnostic_report",
        lua.create_function(|lua, value: mlua::Value| {
            let report: serde_json::Value = lua.from_value_with(
                value,
                DeserializeOptions::new().encode_empty_tables_as_array(false),
            )?;
            serde_json::to_string(&report).map_err(mlua::Error::external)
        })?,
    )?;
    lua.globals().set(
        "parse_typed_input_report",
        lua.create_function(|lua, text: String| {
            let report: serde_json::Value =
                serde_json::from_str(&text).map_err(mlua::Error::external)?;
            lua.to_value_with(
                &report,
                SerializeOptions::new()
                    .set_array_metatable(false)
                    .serialize_none_to_null(false)
                    .serialize_unit_to_null(false),
            )
        })?,
    )
}

fn job_source(id: &str) -> String {
    let job = crate::job_catalog::builtins()
        .expect("bundled diagnostic job definitions must load")
        .into_iter()
        .find(|job| job.id == id)
        .expect("World Editor diagnostic job must be discoverable");
    match job.execution {
        Some(DiagnosticExecution::Lua { source, .. }) => source,
        _ => panic!("World Editor diagnostic job must load its Lua source"),
    }
}

#[test]
fn diagnostic_job_sources_compile() -> mlua::Result<()> {
    let lua = Lua::new();
    let mut compiled = 0;
    for job in crate::job_catalog::builtins().expect("bundled diagnostic job definitions must load")
    {
        if let Some(DiagnosticExecution::Lua { source, .. }) = job.execution {
            lua.load(source.as_str()).set_name(job.id).into_function()?;
            compiled += 1;
        }
    }
    assert!(compiled >= 4, "the migrated Lua jobs must be discovered");
    Ok(())
}

#[test]
fn registered_impact_checks_both_branches_and_rejects_unadmitted_operations() -> mlua::Result<()> {
    let lua = Lua::new();
    install_report_codec(&lua)?;
    lua.globals().set(
        "impact_probe_source",
        job_source("world-editor.registered-impact-ab"),
    )?;
    lua.load(
        r#"
        local report, calls, finished = nil, 0, 0
        diagnostic = {
            publish = function(value)
                report = parse_typed_input_report(encode_diagnostic_report(value))
            end,
            finish = function()
                assert(report and (report.state == 'passed' or report.state == 'failed'),
                    'a terminal report must be published before finish')
                finished = finished + 1
            end,
        }
        shroudforge = {log = {info = function() end}}
        runtime = {
            world = {session_id = function() return 1 end},
            types = {resolve = function(name) return {qualified_name = name} end},
            impact = {},
        }
        runtime.impact.call = function(ty, args)
            if type(ty) == 'string' then
                return nil, 'no validated registered Impact adapter'
            end
            calls = calls + 1
            return {branch = args.entityAId.id == args.entityBId.id}
        end
        local probe = assert(load(impact_probe_source))()
        probe.on_load()
        probe.on_update()
        assert(calls == 4 and report.state == 'passed' and finished == 1)
        assert(#report.results == 4 and report.unadmitted_operation == 'rejected')
        assert(report.results[2].name == 'different' and report.results[2].branch == false)
        assert(report.results[4].name == 'full-uint32' and report.results[4].branch == false)
        assert(report.world_mutations_requested == 0)
        probe.on_update()
        assert(calls == 4 and finished == 1)
        runtime.impact.call = function() return {branch = true} end
        probe = assert(load(impact_probe_source))()
        probe.on_update()
        assert(report.state == 'failed' and report.detail:find('different returned unexpected branch', 1, true))
        assert(finished == 2)
    "#,
    )
    .set_name("registered-impact-private-api")
    .exec()
}

#[test]
fn typed_input_observation_separates_readability_from_consumption_and_sessions() -> mlua::Result<()>
{
    let lua = Lua::new();
    install_report_codec(&lua)?;
    struct NamedMask(Vec<String>);
    impl mlua::UserData for NamedMask {
        fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
            methods.add_meta_method(mlua::MetaMethod::Len, |_, this, ()| Ok(this.0.len()));
            methods.add_meta_method(mlua::MetaMethod::Index, |_, this, index: usize| {
                Ok(index
                    .checked_sub(1)
                    .and_then(|index| this.0.get(index))
                    .cloned())
            });
        }
    }
    lua.globals().set(
        "fixture_named_mask",
        lua.create_function(|lua, names: Vec<String>| lua.create_userdata(NamedMask(names)))?,
    )?;
    lua.globals().set(
        "typed_inputs_probe_source",
        job_source("world-editor.typed-inputs"),
    )?;
    lua.load(include_str!("fixtures/typed_inputs.lua"))
        .set_name("typed-input-private-api")
        .exec()
}

#[test]
fn voxel_write_preserves_baseline_and_stops_after_uncertain_results() -> mlua::Result<()> {
    let lua = Lua::new();
    install_report_codec(&lua)?;
    lua.globals().set(
        "voxel_write_probe_source",
        job_source("world-editor.voxel-write-ab"),
    )?;
    lua.load(include_str!("fixtures/voxel_write.lua"))
        .set_name("voxel-write-private-world")
        .exec()
}
