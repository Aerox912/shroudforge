mod budget;
mod error;
mod parser;
mod pe_file;
mod util;
pub use parser::extract_reflection_data;
pub fn unresolved_optional_count() -> usize {
    budget::unresolved_count()
}
#[cfg(test)]
pub(crate) mod tests;

pub fn attribute_collisions() -> Vec<serde_json::Value> {
    budget::collisions()
}

pub fn reference_issues() -> Vec<serde_json::Value> {
    budget::reference_issues()
}
