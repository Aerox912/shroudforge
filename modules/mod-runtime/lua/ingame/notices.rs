use super::*;

pub(crate) fn create(lua: &mlua::Lua, owner: &Mod) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;
    add_function_with_mod(lua, &table, "show", owner, show)?;
    Ok(table)
}
fn show(lua: &mlua::Lua, args: FunctionArgs, owner: &Mod) -> mlua::Result<(bool, Option<String>)> {
    if args.len() != 1 {
        return Err(LuaError::generic(
            "notice.show(message) takes exactly one string",
        ));
    }
    let message = args.get::<String>(0)?;
    let state = lua.app_data_ref::<AppState>().unwrap();
    let denial = runtime_denial_reason(&state, owner, "runtime.notice.show");
    let opted_in = owner
        .info()
        .capabilities
        .contains(&Capability::RuntimeNotices);
    let client = state.is_client() && owner.info().target.supports_process(false);
    deliver(&message, denial, opted_in, client, |message| {
        let generation = state.lua_map_owner(&owner.info().id);
        shroudforge_runtime_bridge::show_notice(&owner.info().id, generation, message)
    })
}

fn deliver(
    message: &str,
    denial: Option<String>,
    opted_in: bool,
    client: bool,
    submit: impl FnOnce(&str) -> Result<(), i32>,
) -> mlua::Result<(bool, Option<String>)> {
    if let Some(reason) = denial {
        return Ok((false, Some(reason)));
    }
    if !opted_in {
        return Ok((false, Some("notices require runtime-notices capability at startup and a game restart after enabling".into())));
    }
    if !client {
        return Ok((
            false,
            Some("notices require a client package and process".into()),
        ));
    }
    match submit(message) {
        Ok(()) => Ok((true, None)),
        Err(status) => Ok((
            false,
            Some(format!(
                "notice unavailable (status {status}); requires valid UTF-8 up to 256 bytes and an admitted graphics backend"
            )),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::deliver;
    use std::{cell::Cell, rc::Rc};
    #[test]
    fn public_lua_notice_contract_preserves_bool_reason_and_blocks_unopted_calls()
    -> mlua::Result<()> {
        // Use the canonical authority/dispatch operation with an inert submit
        // dependency. No KFC file, AppState, provider DLL or game is loaded.
        for (denial, opted, client, status, expected, reason, called) in [
            (None, false, true, 0, false, "runtime-notices", false),
            (None, true, false, 0, false, "client package", false),
            (
                Some("inactive lifecycle"),
                true,
                true,
                0,
                false,
                "inactive lifecycle",
                false,
            ),
            (None, true, true, 3, false, "status 3", true),
            (None, true, true, 0, true, "", true),
        ] {
            let lua = mlua::Lua::new();
            let count = Rc::new(Cell::new(0));
            let observed = count.clone();
            let notice = lua.create_table()?;
            notice.set(
                "show",
                lua.create_function(move |_, message: String| {
                    deliver(&message, denial.map(str::to_owned), opted, client, |text| {
                        assert_eq!(text, "Flight enabled");
                        observed.set(observed.get() + 1);
                        if status == 0 { Ok(()) } else { Err(status) }
                    })
                })?,
            )?;
            let runtime = lua.create_table()?;
            runtime.set("notice", notice)?;
            let sf = lua.create_table()?;
            sf.set("runtime", runtime)?;
            lua.globals().set("shroudforge", sf)?;
            let (ok, explanation): (bool, Option<String>) = lua
                .load("return shroudforge.runtime.notice.show('Flight enabled')")
                .eval()?;
            assert_eq!(ok, expected);
            assert_eq!(count.get(), u32::from(called));
            if expected {
                assert!(explanation.is_none());
            } else {
                assert!(explanation.unwrap().contains(reason));
            }
        }
        Ok(())
    }
}
