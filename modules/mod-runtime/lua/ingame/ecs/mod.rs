use super::*;
use crate::env::{loader, registration, registry, runtime_ownership, runtime_values};
mod access_component_fields;
pub(crate) mod access_template_component;
pub(crate) mod attributes;
mod query_entities;
mod read_component;
mod read_component_catalog;
mod read_component_registry;
mod resolve_entity;
mod resolve_peer;
mod set_peer_flight;
mod vein_control;
mod write_component;
use access_component_fields::*;
use query_entities::*;
use read_component::*;
use read_component_catalog::*;
use read_component_registry::*;
use resolve_entity::*;
use resolve_peer::*;
use set_peer_flight::*;
use vein_control::*;
use write_component::*;
pub(super) fn create_ecs(lua: &mlua::Lua, r#mod: &Mod) -> mlua::Result<mlua::Table> {
    let table = lua.create_table()?;

    add_function_with_mod(lua, &table, "get_components", r#mod, lua_ecs_get_components)?;
    add_function_with_mod(lua, &table, "get_catalog", r#mod, lua_ecs_get_catalog)?;
    add_function_with_mod(lua, &table, "get_registry", r#mod, lua_ecs_get_registry)?;
    add_function_with_mod(lua, &table, "get_component", r#mod, lua_ecs_get_component)?;
    add_function_with_mod(lua, &table, "query", r#mod, lua_ecs_query)?;
    add_function_with_mod(lua, &table, "query_bounds", r#mod, lua_ecs_query_bounds)?;
    add_function_with_mod(lua, &table, "resolve", r#mod, lua_ecs_resolve)?;
    add_function_with_mod(lua, &table, "resolve_peer", r#mod, lua_ecs_resolve_peer)?;
    add_function_with_mod(
        lua,
        &table,
        "set_peer_flight",
        r#mod,
        lua_ecs_set_peer_flight,
    )?;
    add_function_with_mod(
        lua,
        &table,
        "begin_peer_vein",
        r#mod,
        lua_ecs_begin_peer_vein,
    )?;
    add_function_with_mod(lua, &table, "set_peer_vein", r#mod, lua_ecs_set_peer_vein)?;
    add_function_with_mod(
        lua,
        &table,
        "revoke_peer_vein",
        r#mod,
        lua_ecs_revoke_peer_vein,
    )?;
    add_function_with_mod(lua, &table, "read", r#mod, lua_ecs_read)?;
    add_function_with_mod(lua, &table, "read_bytes", r#mod, lua_ecs_read_bytes)?;
    add_function_with_mod(lua, &table, "read_fields", r#mod, lua_ecs_read_fields)?;
    add_function_with_mod(lua, &table, "get_template", r#mod, lua_ecs_get_template)?;
    crate::env::runtime_templates::attach(lua, &table, r#mod)?;
    add_function_with_mod(lua, &table, "write_fields", r#mod, lua_ecs_write_fields)?;
    add_function_with_mod(lua, &table, "write", r#mod, lua_ecs_write)?;
    crate::env::runtime_attributes::attach(lua, &table, r#mod)?;

    Ok(table)
}
