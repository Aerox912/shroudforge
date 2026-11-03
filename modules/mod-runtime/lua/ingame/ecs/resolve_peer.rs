use super::*;

pub(super) fn lua_ecs_resolve_peer(
    lua: &mlua::Lua,
    args: FunctionArgs,
    r#mod: &Mod,
) -> mlua::Result<(LuaValue, Option<String>)> {
    let state = lua.app_data_ref::<AppState>().unwrap();
    if let Some(reason) = runtime_denial_reason(&state, r#mod, "runtime.ecs.resolve") {
        return Ok((LuaValue::Nil, Some(reason)));
    }
    if args.len() != 1 {
        return Ok((
            LuaValue::Nil,
            Some("resolve_peer expects one Steam64 string".into()),
        ));
    }
    // Lua numbers cannot exactly represent a Steam64. Refuse numeric coercion.
    let LuaValue::String(peer) = args.get::<LuaValue>(0)? else {
        return Ok((
            LuaValue::Nil,
            Some("resolve_peer requires a Steam64 string".into()),
        ));
    };
    let peer = peer.to_str()?;
    Ok(match runtime_provider::resolve_peer_player(peer.as_ref()) {
        Ok(handle) => (LuaValue::Integer(i64::from(handle)), None),
        Err(reason) => (LuaValue::Nil, Some(reason.into())),
    })
}
