pub mod acquire;
pub mod containment;
pub mod extractor;
pub mod guard;
pub mod metadata;
pub mod schema;
pub const CONTRACT_BYTES: &[u8] = include_bytes!("../CONTRACT.json");
pub mod diagnostics;

mod read_budget;
