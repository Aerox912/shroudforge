use mlua::Lua;

#[test]
fn unlimited_flight_uses_public_runtime_api_and_restores_owned_values() -> mlua::Result<()> {
    let lua = Lua::new();
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/sf-unlimited-flight/src");
    lua.globals().set(
        "flight_source",
        std::fs::read_to_string(root.join("mod.lua")).unwrap(),
    )?;
    let source: String = lua.globals().get("flight_source")?;
    assert!(source.contains("runtime.overrides.new()"));
    assert!(source.contains("was_keybind_pressed, \"toggleUpwardGliding\""));
    assert!(!source.contains("bind_modifier"));
    assert!(!source.contains("legacy"));
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("../mod.json")).unwrap()).unwrap();
    assert!(
        manifest["dependencies"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
    let extended: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("../extended.mod.json")).unwrap()).unwrap();
    assert_eq!(
        extended["settings"]["toggleUpwardGliding"]["value"],
        "SDL:63"
    );
    assert_eq!(
        extended["settings"]["toggleUpwardGliding"]["control"],
        "keybind"
    );
    lua.globals().set(
        "overrides_source",
        include_str!("../lua/ingame/overrides/track_runtime_overrides.lua"),
    )?;
    let targets: mlua::Function = lua
        .load(include_str!(
            "../lua/ingame/overrides/create_override_targets.lua"
        ))
        .set_name("flight-override-targets")
        .eval()?;
    lua.globals().set("override_targets_factory", targets)?;
    lua.load(include_str!("unlimited_flight_lifecycle.lua"))
        .set_name("flight-lifecycle")
        .exec()
}
