//! Optional native per-peer Flight intent. Acceptance is not gameplay evidence.
fn set_with(
    peer: &str,
    session: u64,
    enabled: bool,
    mut call: impl FnMut(u64, u64, u32) -> Result<i32, &'static str>,
) -> Result<(), &'static str> {
    let uid = crate::player_ownership::parse_peer(peer)?;
    if enabled && session == 0 {
        return Err("active authoritative world required");
    }
    match call(uid, session, u32::from(enabled))? {
        0 => Ok(()),
        4 => Err("native Flight control busy"),
        7 => Err("native Flight control fault"),
        _ => Err("native per-player Flight unavailable"),
    }
}
#[cfg(windows)]
fn call_native(uid: u64, session: u64, enabled: u32) -> Result<i32, &'static str> {
    use std::{
        ffi::{c_char, c_void},
        sync::OnceLock,
    };
    type Set = unsafe extern "C" fn(u64, u64, u32) -> i32;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *const c_void;
    }
    static SETTER: OnceLock<Set> = OnceLock::new();
    let call = if let Some(call) = SETTER.get() {
        *call
    } else {
        let module_name: Vec<u16> = "kfc-runtime.dll\0".encode_utf16().collect();
        let module = unsafe { GetModuleHandleW(module_name.as_ptr()) };
        if module.is_null() {
            return Err("native Flight provider unavailable");
        }
        let pointer =
            unsafe { GetProcAddress(module, c"KfcRuntimeFlightSetPeerIntentV1".as_ptr()) };
        if pointer.is_null() {
            return Err("native per-player Flight unsupported by this provider");
        }
        let call = unsafe { std::mem::transmute::<*const c_void, Set>(pointer) };
        *SETTER.get_or_init(|| call)
    };
    Ok(unsafe { call(uid, session, enabled) })
}
#[cfg(not(windows))]
fn call_native(_: u64, _: u64, _: u32) -> Result<i32, &'static str> {
    Err("native per-player Flight requires Windows")
}

pub fn set_peer_flight(peer: &str, enabled: bool) -> Result<(), &'static str> {
    let session = crate::world_session_id();
    set_with(peer, session, enabled, call_native)?;
    if enabled && crate::world_session_id() != session {
        return Err("world session changed during Flight request");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const PEER: &str = "76561198000000001";
    #[test]
    fn preserves_uid_and_strict_types() {
        let mut observed = None;
        assert_eq!(
            set_with(PEER, 7, true, |uid, epoch, on| {
                observed = Some((uid, epoch, on));
                Ok(0)
            }),
            Ok(())
        );
        assert_eq!(observed, Some((76561198000000001, 7, 1)));
        for invalid in [
            "0",
            "076561198000000001",
            "76561198000000001.0",
            "18446744073709551616",
        ] {
            assert!(
                set_with(invalid, 7, true, |_, _, _| panic!(
                    "bad UID reached native control"
                ))
                .is_err()
            );
        }
    }
    #[test]
    fn off_clears_without_active_epoch_and_on_requires_one() {
        assert_eq!(
            set_with(PEER, 0, false, |_, epoch, on| {
                assert_eq!((epoch, on), (0, 0));
                Ok(0)
            }),
            Ok(())
        );
        assert!(
            set_with(PEER, 0, true, |_, _, _| panic!(
                "On without world reached native control"
            ))
            .is_err()
        );
    }
    #[test]
    fn no_refusal_is_reported_as_accepted_intent() {
        for code in [1, 3, 4, 6, 7, 99] {
            assert!(set_with(PEER, 7, true, |_, _, _| Ok(code)).is_err());
        }
        assert!(set_with(PEER, 7, true, |_, _, _| Err("optional provider absent")).is_err());
    }
}
