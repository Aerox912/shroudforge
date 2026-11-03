//! Rust binding of the canonical native runtime ABI.
const KFC_RUNTIME_ABI_VERSION: u32 = 12;

mod runtime_records;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
#[cfg(not(windows))]
mod unsupported_platform;
#[cfg(not(windows))]
pub use unsupported_platform::*;

#[path = "building_input.rs"]
pub mod player_input;

mod flight_control;
mod map_views;
mod notices;
mod player_ownership;
mod vein_control;
pub use flight_control::set_peer_flight;
pub use map_views::{map_fog, map_markers, map_overlay_status, map_summary};
pub use notices::show_notice;
pub use player_ownership::resolve_peer_player;
pub use vein_control::{VeinLease, begin_peer_vein, revoke_peer_vein, set_peer_vein};

pub const fn first_person_operation_name() -> &'static str {
    "runtime.first_person.set_mode"
}
