use super::*;

fn denial(state: &AppState, r#mod: &Mod, write: bool) -> Option<String> {
    policy_denial(
        runtime_denial_reason(state, r#mod, "runtime.lifecycle"),
        state.is_server() && r#mod.info().target.supports_process(true),
        write,
        || runtime_denial_reason(state, r#mod, "runtime.ecs.write"),
    )
}
fn policy_denial(
    lifecycle: Option<String>,
    server: bool,
    write: bool,
    write_denial: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if let Some(reason) = lifecycle {
        return Some(reason);
    }
    if !server {
        return Some("Vein peer control requires a dedicated-server mod".into());
    }
    if write {
        return write_denial();
    }
    None
}
fn response(
    lua: &mlua::Lua,
    result: Result<runtime_provider::VeinLease, &'static str>,
) -> mlua::Result<(LuaValue, Option<String>)> {
    match result {
        Err(reason) => Ok((LuaValue::Nil, Some(reason.into()))),
        Ok(lease) => {
            let value = lua.create_table()?;
            value.raw_set("token", lease.token)?;
            value.raw_set("ttl_ms", lease.ttl_ms)?;
            value.raw_set("renew_after_ms", lease.renew_after_ms)?;
            Ok((LuaValue::Table(value), None))
        }
    }
}
pub(super) fn lua_ecs_begin_peer_vein(
    lua: &mlua::Lua,
    args: FunctionArgs,
    r#mod: &Mod,
) -> mlua::Result<(LuaValue, Option<String>)> {
    let state = lua.app_data_ref::<AppState>().unwrap();
    if let Some(reason) = denial(&state, r#mod, true) {
        return Ok((LuaValue::Nil, Some(reason)));
    }
    if args.len() != 1 {
        return Ok((
            LuaValue::Nil,
            Some("begin_peer_vein expects one Steam64 string".into()),
        ));
    }
    let LuaValue::String(peer) = args.get::<LuaValue>(0)? else {
        return Ok((LuaValue::Nil, Some("Steam64 must be a string".into())));
    };
    response(
        lua,
        runtime_provider::begin_peer_vein(peer.to_str()?.as_ref()),
    )
}
pub(super) fn lua_ecs_set_peer_vein(
    lua: &mlua::Lua,
    args: FunctionArgs,
    r#mod: &Mod,
) -> mlua::Result<(LuaValue, Option<String>)> {
    let state = lua.app_data_ref::<AppState>().unwrap();
    if args.len() != 5 {
        return Ok((
            LuaValue::Nil,
            Some("set_peer_vein expects peer, token, sequence, active and tier".into()),
        ));
    }
    let LuaValue::Boolean(active) = args.get::<LuaValue>(3)? else {
        return Ok((LuaValue::Nil, Some("Vein active must be a boolean".into())));
    };
    if let Some(reason) = denial(&state, r#mod, *active) {
        return Ok((LuaValue::Nil, Some(reason)));
    }
    let parsed = match parse_set(&args) {
        Ok(parsed) => parsed,
        Err(reason) => return Ok((LuaValue::Nil, Some(reason.into()))),
    };
    let allow_renew = runtime_denial_reason(&state, r#mod, "runtime.ecs.write").is_none();
    response(
        lua,
        runtime_provider::set_peer_vein(
            &parsed.peer,
            &parsed.token,
            parsed.sequence,
            parsed.active,
            parsed.tier,
            allow_renew,
        ),
    )
}
#[derive(Debug, PartialEq)]
struct SetArgs {
    peer: String,
    token: String,
    sequence: u32,
    active: bool,
    tier: u32,
}
fn parse_set(args: &FunctionArgs) -> Result<SetArgs, &'static str> {
    if args.len() != 5 {
        return Err("set_peer_vein expects peer, token, sequence, active and tier");
    }
    let get = |index| {
        args.get::<LuaValue>(index)
            .map_err(|_| "invalid Vein argument")
    };
    let (
        LuaValue::String(peer),
        LuaValue::String(token),
        LuaValue::Integer(sequence),
        LuaValue::Boolean(active),
        LuaValue::Integer(tier),
    ) = (get(0)?, get(1)?, get(2)?, get(3)?, get(4)?)
    else {
        return Err("Vein peer/token require strings, sequence/tier integers and active boolean");
    };
    let (Ok(sequence), Ok(tier)) = (u32::try_from(*sequence), u32::try_from(*tier)) else {
        return Err("Vein sequence/tier outside uint32 range");
    };
    Ok(SetArgs {
        peer: peer
            .to_str()
            .map_err(|_| "Vein peer is not UTF-8")?
            .to_string(),
        token: token
            .to_str()
            .map_err(|_| "Vein token is not UTF-8")?
            .to_string(),
        sequence,
        active: *active,
        tier,
    })
}

pub(super) fn lua_ecs_revoke_peer_vein(
    lua: &mlua::Lua,
    args: FunctionArgs,
    r#mod: &Mod,
) -> mlua::Result<(bool, Option<String>)> {
    let state = lua.app_data_ref::<AppState>().unwrap();
    if let Some(reason) = denial(&state, r#mod, false) {
        return Ok((false, Some(reason)));
    }
    if args.len() != 2 {
        return Ok((
            false,
            Some("revoke_peer_vein expects peer and token strings".into()),
        ));
    }
    let (LuaValue::String(peer), LuaValue::String(token)) =
        (args.get::<LuaValue>(0)?, args.get::<LuaValue>(1)?)
    else {
        return Ok((false, Some("Vein peer and token must be strings".into())));
    };
    Ok(
        match runtime_provider::revoke_peer_vein(peer.to_str()?.as_ref(), token.to_str()?.as_ref())
        {
            Ok(()) => (true, None),
            Err(reason) => (false, Some(reason.into())),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::FromLuaMulti;
    fn arguments(lua: &mlua::Lua, expression: &str) -> FunctionArgs {
        FunctionArgs::from_lua_multi(lua.load(expression).eval().unwrap(), lua).unwrap()
    }
    #[test]
    fn actual_lua_values_keep_peer_precision_and_ordered_off() {
        let lua = mlua::Lua::new();
        let parsed = parse_set(&arguments(
            &lua,
            "return '76561198000000001', '01000000000000000200000000000000', 4294967295, false, 3",
        ))
        .unwrap();
        assert_eq!(
            parsed,
            SetArgs {
                peer: "76561198000000001".into(),
                token: "01000000000000000200000000000000".into(),
                sequence: u32::MAX,
                active: false,
                tier: 3
            }
        );
    }
    #[test]
    fn strict_lua_parser_refuses_coercion_range_and_arity() {
        let lua = mlua::Lua::new();
        for text in [
            "return 76561198000000001,'t',1,true,2",
            "return 'p',4,1,true,2",
            "return 'p','t',1.5,true,2",
            "return 'p','t',1,1,2",
            "return 'p','t',1,true,2.5",
            "return 'p','t',-1,true,2",
            "return 'p','t',4294967296,true,2",
            "return 'p','t',1,true,-1",
            "return 'p','t',1,true,4294967296",
            "return 'p','t',1,true",
            "return 'p','t',1,true,2,3",
            "return string.char(255),'t',1,true,2",
            "return 'p',string.char(255),1,true,2",
        ] {
            assert!(parse_set(&arguments(&lua, text)).is_err(), "{text}");
        }
    }
    #[test]
    fn lifecycle_and_server_gates_cannot_be_bypassed_by_off() {
        for write in [false, true] {
            assert_eq!(
                policy_denial(Some("inactive mod".into()), true, write, || panic!()),
                Some("inactive mod".into())
            );
            assert!(policy_denial(None, false, write, || panic!()).is_some());
        }
        assert_eq!(
            policy_denial(None, true, true, || Some("write denied".into())),
            Some("write denied".into())
        );
        assert_eq!(policy_denial(None, true, false, || panic!()), None);
        assert_eq!(policy_denial(None, true, true, || None), None);
    }
    #[test]
    fn off_ack_and_refusal_are_not_usable_leases() {
        let lua = mlua::Lua::new();
        let (value, reason) = response(
            &lua,
            Ok(runtime_provider::VeinLease {
                token: None,
                ttl_ms: 0,
                renew_after_ms: 0,
            }),
        )
        .unwrap();
        assert!(reason.is_none());
        let LuaValue::Table(table) = value else {
            panic!("Off must return a structured acknowledgement")
        };
        assert!(matches!(
            table.get::<LuaValue>("token").unwrap(),
            LuaValue::Nil
        ));
        assert_eq!(table.get::<u32>("ttl_ms").unwrap(), 0);
        assert_eq!(table.get::<u32>("renew_after_ms").unwrap(), 0);
        let (value, reason) = response(&lua, Err("provider unavailable")).unwrap();
        assert!(matches!(value, LuaValue::Nil));
        assert_eq!(reason.as_deref(), Some("provider unavailable"));
    }
}
