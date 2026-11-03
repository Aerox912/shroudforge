//! Single-use handoff from a verified asset preparation to the parked bootstrap.
//! Thread-local ownership binds demand to the synchronous preparing caller; an
//! offline preparation helper never applies executable changes.
use std::{cell::RefCell, path::{Path, PathBuf}};

const SUPPORTED_EXE: &str = "0403981fd2e8ad18bd8c5698dff03c8d505b5a46677832c496db9c645513f2a6";
thread_local! { static PREPARED: RefCell<Option<(PathBuf, u32)>> = const { RefCell::new(None) }; }

pub fn clear() { PREPARED.with(|slot| *slot.borrow_mut() = None); }

pub(super) fn publish(canonical_root: &Path, enabled: bool, executable_sha256: &str) {
    let demand = if !enabled { 0 } else if executable_sha256 == SUPPORTED_EXE { 1 } else { 2 };
    PREPARED.with(|slot| *slot.borrow_mut() = Some((canonical_root.to_owned(), demand)));
}

/// 0 disabled, 1 verified supported fifth-level demand, 2 unavailable.
/// Even a wrong-root or invalid-path attempt consumes the handoff.
pub fn take(root: &Path) -> u32 {
    let prepared = PREPARED.with(|slot| slot.borrow_mut().take());
    match (prepared, std::fs::canonicalize(root)) {
        (Some((prepared_root, demand)), Ok(root)) if prepared_root == root => demand,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const NEW_EXE: &str = "0403981fd2e8ad18bd8c5698dff03c8d505b5a46677832c496db9c645513f2a6";
    const PREVIOUS_EXE: &str = "d64966e23b1a458ee6f147bcbceac0b577fa7e39a212f371287d8cc119817882";
    fn root() -> PathBuf { std::fs::canonicalize(std::env::temp_dir()).unwrap() }
    #[test] fn enabled_once() { let r=root(); clear(); publish(&r,true,SUPPORTED_EXE); assert_eq!(take(&r),1); assert_eq!(take(&r),2); }
    #[test] fn new_client_executable_is_supported_once() { let r=root(); clear(); publish(&r,true,NEW_EXE); assert_eq!(take(&r),1); assert_eq!(take(&r),2); }
    #[test] fn previous_client_executable_is_unsupported() { let r=root(); clear(); publish(&r,true,PREVIOUS_EXE); assert_eq!(take(&r),2); }
    #[test] fn disabled_and_unsupported() { let r=root(); publish(&r,false,SUPPORTED_EXE); assert_eq!(take(&r),0); publish(&r,true,"other"); assert_eq!(take(&r),2); }
    #[test] fn failed_prepare_clears_old_demand() { let r=root(); publish(&r,true,SUPPORTED_EXE); clear(); assert_eq!(take(&r),2); }
    #[test] fn wrong_root_consumes() { let r=root(); publish(&r,true,SUPPORTED_EXE); assert_eq!(take(&r.join("absent-zoom-demand-7fc55a")),2); assert_eq!(take(&r),2); }
    #[test] fn other_thread_cannot_consume() { let r=root(); publish(&r,true,SUPPORTED_EXE); let other=r.clone(); assert_eq!(std::thread::spawn(move || take(&other)).join().unwrap(),2); assert_eq!(take(&r),1); }
    #[test] fn new_preparation_replaces_old_selection() { let r=root(); publish(&r,true,SUPPORTED_EXE); clear(); publish(&r,false,SUPPORTED_EXE); assert_eq!(take(&r),0); }
}
