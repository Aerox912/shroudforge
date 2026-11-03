//! Single-use handoff from verified current-assets preparation to the parked
//! bootstrap. Executable changes remain outside the Rust preparation path.
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

const SUPPORTED_EXE: &str = "d64966e23b1a458ee6f147bcbceac0b577fa7e39a212f371287d8cc119817882";

thread_local! {
    static PREPARED: RefCell<Option<(PathBuf, u32)>> = const { RefCell::new(None) };
}

pub fn clear() {
    PREPARED.with(|slot| *slot.borrow_mut() = None);
}

pub(super) fn publish(canonical_root: &Path, enabled: bool, executable_sha256: &str) {
    let demand = if !enabled {
        0
    } else if executable_sha256 == SUPPORTED_EXE {
        1
    } else {
        2
    };
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

    fn root() -> PathBuf {
        std::fs::canonicalize(std::env::temp_dir()).unwrap()
    }

    #[test]
    fn demand_is_root_bound_single_use_and_thread_local() {
        let root = root();
        clear();
        publish(&root, true, SUPPORTED_EXE);
        assert_eq!(take(&root), 1);
        assert_eq!(take(&root), 2);

        publish(&root, true, SUPPORTED_EXE);
        let other = root.clone();
        assert_eq!(std::thread::spawn(move || take(&other)).join().unwrap(), 2);
        assert_eq!(take(&root), 1);

        publish(&root, true, SUPPORTED_EXE);
        assert_eq!(take(&root.join("absent-zoom-demand-7fc55a")), 2);
        assert_eq!(take(&root), 2);
    }

    #[test]
    fn disabled_and_unsupported_selections_are_distinct() {
        let root = root();
        publish(&root, false, SUPPORTED_EXE);
        assert_eq!(take(&root), 0);
        publish(&root, true, "unsupported");
        assert_eq!(take(&root), 2);
    }

    #[test]
    fn failed_or_repeated_preparation_cannot_reuse_old_demand() {
        let root = root();
        publish(&root, true, SUPPORTED_EXE);
        clear();
        assert_eq!(take(&root), 2);
        publish(&root, true, SUPPORTED_EXE);
        clear();
        publish(&root, false, SUPPORTED_EXE);
        assert_eq!(take(&root), 0);
    }
}
