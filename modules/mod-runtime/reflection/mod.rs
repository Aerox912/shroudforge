//! Automatic resolution from the active game data. No Lua, live pointers, patches, or diagnostic I/O.
#[path = "build_attribute_catalog.rs"]
pub(crate) mod attributes;
#[path = "evaluate_attribute_program.rs"]
pub(crate) mod calculation;
#[path = "resolve_system_dependencies.rs"]
pub mod functions;
#[path = "traverse_reflected_types.rs"]
pub mod types;

pub(crate) mod load_type_registry;

pub(crate) mod external_registry;
