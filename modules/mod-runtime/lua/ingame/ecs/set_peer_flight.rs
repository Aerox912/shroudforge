use super::*;

pub(super) fn lua_ecs_set_peer_flight(
    lua: &mlua::Lua,
    args: FunctionArgs,
    r#mod: &Mod,
) -> mlua::Result<(bool, Option<String>)> {
    let state = lua.app_data_ref::<AppState>().unwrap();
    if let Some(reason) = runtime_denial_reason(&state, r#mod, "runtime.lifecycle") {
        return Ok((false, Some(reason)));
    }
    if args.len() != 2 {
        return Ok((
            false,
            Some("set_peer_flight expects a Steam64 string and boolean".into()),
        ));
    }
    let LuaValue::String(peer) = args.get::<LuaValue>(0)? else {
        return Ok((
            false,
            Some("set_peer_flight requires a Steam64 string".into()),
        ));
    };
    let LuaValue::Boolean(enabled) = args.get::<LuaValue>(1)? else {
        return Ok((false, Some("set_peer_flight requires a boolean".into())));
    };
    let enabled = *enabled;
    // Off clears host intent only, so it must remain available when no world
    // or writable ECS binding exists. On requires the ordinary write policy.
    if enabled {
        if let Some(reason) = runtime_denial_reason(&state, r#mod, "runtime.ecs.write") {
            return Ok((false, Some(reason)));
        }
    }
    let peer = peer.to_str()?;
    Ok(
        match runtime_provider::set_peer_flight(peer.as_ref(), enabled) {
            Ok(()) => (true, None),
            Err(reason) => (false, Some(reason.into())),
        },
    )
}
