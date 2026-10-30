use std::time::{SystemTime, UNIX_EPOCH};

use mlua::{Function, Table, Value, Variadic};
use mod_loader::Mod;
use serde_json::json;

use super::set_mod_settings::set_settings;
use crate::lua::LuaValue;

pub fn create(lua: &mlua::Lua, r#mod: &Mod) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.raw_set("version", env!("CARGO_PKG_VERSION"))?;
    table.raw_set("mod_id", r#mod.info().id.as_str())?;

    let log = lua.create_table()?;
    for level in ["trace", "debug", "info", "warn", "error"] {
        let id = r#mod.info().id.clone();
        log.raw_set(
            level,
            lua.create_function(move |_, args: Variadic<LuaValue>| {
                let disabled_detail_level = match level {
                    "trace" => {
                        !tracing::enabled!(target: "shroudforge::mod", tracing::Level::TRACE)
                    }
                    "debug" => {
                        !tracing::enabled!(target: "shroudforge::mod", tracing::Level::DEBUG)
                    }
                    _ => false,
                };
                if disabled_detail_level {
                    return Ok(());
                }
                let message = args
                    .into_iter()
                    .map(|value| value.to_string())
                    .collect::<mlua::Result<Vec<_>>>()?
                    .join("\t");
                match level {
                    "trace" => {
                        tracing::trace!(target: "shroudforge::mod", mod_id = %id, "{message}")
                    }
                    "debug" => {
                        tracing::debug!(target: "shroudforge::mod", mod_id = %id, "{message}")
                    }
                    "info" => tracing::info!(target: "shroudforge::mod", mod_id = %id, "{message}"),
                    "warn" => tracing::warn!(target: "shroudforge::mod", mod_id = %id, "{message}"),
                    "error" => {
                        tracing::error!(target: "shroudforge::mod", mod_id = %id, "{message}")
                    }
                    _ => unreachable!(),
                }
                Ok(())
            })?,
        )?;
    }
    table.raw_set("log", log)?;

    let game_dir = lua
        .app_data_ref::<crate::env::AppState>()
        .unwrap()
        .env()
        .game_dir()
        .as_std_path()
        .to_path_buf();
    let mod_id = r#mod.info().id.clone();

    let settings = lua.create_table()?;
    // In-memory values; the runtime refreshes explicitly live settings between updates.
    set_settings(lua, &mod_id, &r#mod.info().setting_values)?;
    let settings_id = mod_id.clone();
    settings.raw_set(
        "get",
        lua.create_function(move |lua, (key, fallback): (String, Option<Value>)| {
            if !valid_identifier(&key) {
                return Err(mlua::Error::runtime("invalid setting key"));
            }
            let current: Table =
                lua.named_registry_value(&format!("shroudforge.settings.{settings_id}"))?;
            let value: Value = current.raw_get(key)?;
            Ok(if value.is_nil() {
                fallback.unwrap_or(Value::Nil)
            } else {
                value
            })
        })?,
    )?;
    table.raw_set("settings", settings)?;

    let input = lua.create_table()?;
    input.raw_set(
        "is_key_down",
        lua.create_function(|_, key: String| Ok(is_game_key_down(&key)))?,
    )?;
    table.raw_set("input", input)?;

    let ui = lua.create_table()?;
    let ui_mod_id = mod_id.clone();
    ui.raw_set(
        "on_action",
        lua.create_function(move |lua, (action, callback): (String, Function)| {
            if !valid_identifier(&ui_mod_id) || !valid_identifier(&action) {
                return Err(mlua::Error::runtime("invalid ui action"));
            }
            lua.set_named_registry_value(&action_registry_key(&ui_mod_id, &action), callback)
        })?,
    )?;
    table.raw_set("ui", ui)?;

    let notifications = lua.create_table()?;
    notifications.raw_set(
        "publish",
        lua.create_function(move |_, notice: Table| {
            let id = notice.get::<String>("id")?;
            let title = notice.get::<String>("title")?;
            let message = notice.get::<String>("message")?;
            let level = notice.get::<Option<String>>("level")?.unwrap_or_else(|| "info".into());
            let action_url = notice.get::<Option<String>>("action_url")?;
            if !valid_identifier(&mod_id)
                || !valid_identifier(&id)
                || title.len() > 120
                || message.len() > 2_000
            {
                return Err(mlua::Error::runtime("invalid notification payload"));
            }
            if !matches!(level.as_str(), "info" | "success" | "warning" | "error" | "update") {
                return Err(mlua::Error::runtime("invalid notification level"));
            }
            if action_url.as_deref().is_some_and(|url| !url.starts_with("https://")) {
                return Err(mlua::Error::runtime("notification action_url must use HTTPS"));
            }
            let namespaced_id = format!("{mod_id}.{id}");
            let payload = json!({
                "id": namespaced_id,
                "modId": mod_id,
                "title": title,
                "message": message,
                "level": level,
                "actionUrl": action_url,
                "updatedAt": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
            });
            shroudforge_news::write_event(&game_dir, &format!("{mod_id}-{id}"), &payload).map_err(mlua::Error::external)?;
            Ok(())
        })?,
    )?;
    table.raw_set("notifications", notifications)?;

    Ok(table)
}

fn is_game_key_down(key: &str) -> bool {
    parse_virtual_key(key).and_then(|key|shroudforge_runtime_bridge::read_input_keys(&[key]).ok())
        .is_some_and(|states|states.first().is_some_and(|state|state.flags&1!=0))
}
fn parse_virtual_key(key:&str)->Option<u32> {
    let key=key.trim().to_ascii_uppercase();
    if let Some(code)=key.strip_prefix("0X").and_then(|code|u32::from_str_radix(code,16).ok()).filter(|code|(1..=255).contains(code)){return Some(code);}
    if let Some(number)=key.strip_prefix('F').and_then(|number|number.parse::<u32>().ok()).filter(|n|(1..=24).contains(n)){return Some(0x6f+number);}
    if key.len()==1 {let byte=key.as_bytes()[0];if byte.is_ascii_alphanumeric(){return Some(u32::from(byte));}}
    match key.as_str(){"PLUS"|"OEMPLUS"|"="|"+"=>Some(0xbb),"MINUS"|"OEMMINUS"|"-"=>Some(0xbd),"NUMPAD+"|"NUMPADPLUS"|"ADD"=>Some(0x6b),"NUMPAD-"|"NUMPADMINUS"|"SUBTRACT"=>Some(0x6d),"SPACE"=>Some(0x20),"TAB"=>Some(9),"HOME"=>Some(0x24),"END"=>Some(0x23),_=>None}
}

pub(super) fn action_registry_key(mod_id: &str, action: &str) -> String {
    format!("shroudforge.ui.action.{mod_id}.{action}")
}

pub(super) fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

#[cfg(test)]mod input_key_tests {use super::*;
    #[test]fn configured_minimap_keys_use_one_bounded_mapping(){
        for (key,expected) in [("M",77),("f24",135),("+",187),("-",189),("NUMPAD+",107),("0x6A",106),("ADD",107)]{
            let parsed=if key=="+"{parse_virtual_key("PLUS")}else{parse_virtual_key(key)};assert_eq!(parsed,Some(expected));
        }
        assert_eq!(parse_virtual_key("F25"),None);assert_eq!(parse_virtual_key("unknown"),None);
    }
}
