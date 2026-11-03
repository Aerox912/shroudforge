use mlua::{Function, Lua, Table};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[test]
fn first_person_home_toggle_updates_both_modes_and_unload_disables() -> mlua::Result<()> {
    let lua = Lua::new();
    let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../mods/shroudweave-first-person/src/mod.lua");
    let source = std::fs::read_to_string(source_path).unwrap();

    let calls = Rc::new(RefCell::new(Vec::<(bool, f64)>::new()));
    let reject_next = Rc::new(Cell::new(false));
    let key_pressed = Rc::new(Cell::new(false));
    let distance = Rc::new(Cell::new(-0.15_f64));
    let reported = Rc::new(RefCell::new(Vec::<(String, String)>::new()));

    let runtime = lua.create_table()?;
    let first_person = lua.create_table()?;
    let calls_from_native = Rc::clone(&calls);
    let reject_from_native = Rc::clone(&reject_next);
    first_person.set(
        "set_mode",
        lua.create_function(move |_, (enabled, camera_distance): (bool, f64)| {
            calls_from_native
                .borrow_mut()
                .push((enabled, camera_distance));
            if reject_from_native.replace(false) {
                Ok((false, Some("unsupported build".to_string())))
            } else {
                Ok((true, None::<String>))
            }
        })?,
    )?;
    runtime.set("first_person", first_person)?;
    let reports = Rc::clone(&reported);
    runtime.set(
        "report_effect",
        lua.create_function(move |_, (state, detail): (String, Option<String>)| {
            reports
                .borrow_mut()
                .push((state, detail.unwrap_or_default()));
            Ok((true, None::<String>))
        })?,
    )?;
    lua.globals().set("runtime", runtime)?;

    let settings = lua.create_table()?;
    let distance_for_settings = Rc::clone(&distance);
    settings.set(
        "get",
        lua.create_function(move |_, (key, fallback): (String, f64)| {
            assert_eq!(key, "cameraDistance");
            let value = distance_for_settings.get();
            if value.is_nan() {
                Ok(fallback)
            } else {
                Ok(value)
            }
        })?,
    )?;
    let input = lua.create_table()?;
    let key_for_input = Rc::clone(&key_pressed);
    input.set(
        "was_keybind_pressed",
        lua.create_function(move |_, key: String| {
            assert_eq!(key, "toggleFirstPerson");
            Ok(key_for_input.replace(false))
        })?,
    )?;
    let shroudforge = lua.create_table()?;
    shroudforge.set("settings", settings)?;
    shroudforge.set("input", input)?;
    lua.globals().set("shroudforge", shroudforge)?;

    let callbacks: Table = lua
        .load(&source)
        .set_name("shroudweave-first-person")
        .eval()?;
    let on_load: Function = callbacks.get("on_load")?;
    let on_update: Function = callbacks.get("on_update")?;
    let on_unload: Function = callbacks.get("on_unload")?;

    on_load.call::<()>(())?;
    assert_eq!(calls.borrow().as_slice(), &[(false, -0.15)]);

    key_pressed.set(true);
    on_update.call::<()>(())?;
    assert_eq!(calls.borrow().last(), Some(&(true, -0.15)));

    key_pressed.set(true);
    on_update.call::<()>(())?;
    assert_eq!(calls.borrow().last(), Some(&(false, -0.15)));

    distance.set(0.25);
    on_update.call::<()>(())?;
    assert_eq!(calls.borrow().last(), Some(&(false, 0.25)));

    reject_next.set(true);
    key_pressed.set(true);
    on_update.call::<()>(())?;
    assert_eq!(calls.borrow().last(), Some(&(true, 0.25)));
    assert!(
        reported
            .borrow()
            .iter()
            .any(|(state, detail)| state == "waiting" && detail == "unsupported build")
    );

    // A failed native enable leaves the local selection in third person. The
    // next accepted Home press retries the same desired first-person state.
    key_pressed.set(true);
    on_update.call::<()>(())?;
    assert_eq!(calls.borrow().last(), Some(&(true, 0.25)));

    on_unload.call::<()>(())?;
    assert_eq!(calls.borrow().last(), Some(&(false, 0.25)));
    Ok(())
}
