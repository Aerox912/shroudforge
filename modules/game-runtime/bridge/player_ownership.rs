//! Current native peer lookup. Returned values are ordinary guarded ECS handles,
//! never a cached ownership grant for a later flight callback.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NativePlayerBinding {
    size: u32,
    version: u32,
    session: u64,
    world_uuid: [u64; 2],
    peer_uid: u64,
    gameplay_key: u32,
    machine_key: u32,
    entity_id: u32,
    entity_generation: u32,
    flags: u32,
    reserved: u32,
}
impl Default for NativePlayerBinding {
    fn default() -> Self {
        Self {
            size: 64,
            version: 1,
            session: 0,
            world_uuid: [0, 0],
            peer_uid: 0,
            gameplay_key: 0,
            machine_key: 0,
            entity_id: 0,
            entity_generation: 0,
            flags: 0,
            reserved: 0,
        }
    }
}
pub(super) fn parse_peer(peer: &str) -> Result<u64, &'static str> {
    if !(16..=20).contains(&peer.len())
        || peer.starts_with('0')
        || !peer.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("peer must be a canonical Steam64 string");
    }
    peer.parse::<u64>()
        .map_err(|_| "peer exceeds Steam64 range")
}
fn valid(binding: &NativePlayerBinding, uid: u64, session: u64) -> bool {
    binding.size == 64
        && binding.version == 1
        && binding.flags == 0
        && binding.reserved == 0
        && binding.peer_uid == uid
        && binding.session == session
        && binding.world_uuid != [0, 0]
        && binding.gameplay_key != 0
        && binding.machine_key != 0
        && binding.entity_id != 0
}
fn resolve_with(
    uid: u64,
    session: u64,
    mut read: impl FnMut(u64, u64) -> Result<NativePlayerBinding, &'static str>,
    mut handle: impl FnMut(u32) -> Option<u32>,
    mut identity: impl FnMut(u32) -> Option<u32>,
) -> Result<u32, &'static str> {
    if uid == 0 || session == 0 {
        return Err("active authoritative world required");
    }
    let before = read(uid, session)?;
    if !valid(&before, uid, session) {
        return Err("invalid native player binding");
    }
    let entity = handle(before.entity_id)
        .filter(|h| *h != 0)
        .ok_or("current player entity handle unavailable")?;
    let after = read(uid, session)?;
    if !valid(&after, uid, session) || after != before || identity(entity) != Some(before.entity_id)
    {
        return Err("player binding changed during entity resolution");
    }
    Ok(entity)
}
#[cfg(windows)]
fn read_native(uid: u64, session: u64) -> Result<NativePlayerBinding, &'static str> {
    use std::{
        ffi::{c_char, c_void},
        sync::OnceLock,
    };
    type Read = unsafe extern "C" fn(u64, u64, *mut NativePlayerBinding) -> i32;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *const c_void;
    }
    static READER: OnceLock<Read> = OnceLock::new();
    let call = if let Some(call) = READER.get() {
        *call
    } else {
        // world_session_id already passed the canonical provider ABI check.
        // Do not cache an absent optional export before provider startup.
        let module_name: Vec<u16> = "kfc-runtime.dll\0".encode_utf16().collect();
        let module = unsafe { GetModuleHandleW(module_name.as_ptr()) };
        if module.is_null() {
            return Err("native player provider unavailable");
        }
        let pointer = unsafe { GetProcAddress(module, c"KfcRuntimeReadPlayerBindingV1".as_ptr()) };
        if pointer.is_null() {
            return Err("native player binding is unsupported by this provider");
        }
        let call = unsafe { std::mem::transmute::<*const c_void, Read>(pointer) };
        *READER.get_or_init(|| call)
    };
    let mut result = NativePlayerBinding::default();
    let status = unsafe { call(uid, session, &mut result) };
    if status == 0 {
        Ok(result)
    } else {
        Err(match status {
            4 => "native player binding busy",
            6 => "native player binding ambiguous",
            7 => "native player binding fault",
            _ => "native player binding unavailable",
        })
    }
}
#[cfg(not(windows))]
fn read_native(_: u64, _: u64) -> Result<NativePlayerBinding, &'static str> {
    Err("native player binding requires Windows")
}

pub fn resolve_peer_player(peer: &str) -> Result<u32, &'static str> {
    let uid = parse_peer(peer)?;
    let session = crate::world_session_id();
    let handle = resolve_with(
        uid,
        session,
        read_native,
        crate::resolve_entity,
        crate::world_entity_identity,
    )?;
    if crate::world_session_id() != session {
        return Err("world session changed during player resolution");
    }
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    const UID: u64 = 76561198000000001;
    fn binding() -> NativePlayerBinding {
        NativePlayerBinding {
            size: 64,
            version: 1,
            session: 7,
            world_uuid: [11, 22],
            peer_uid: UID,
            gameplay_key: 0x141,
            machine_key: 0x101,
            entity_id: 2,
            entity_generation: 40,
            flags: 0,
            reserved: 0,
        }
    }
    #[test]
    fn preserves_steam_id_precision_and_rejects_noncanonical_ids() {
        assert_eq!(parse_peer("76561198000000001"), Ok(UID));
        assert_eq!(parse_peer("76561198000000002"), Ok(UID + 1));
        for bad in [
            "",
            "0",
            "+76561198000000001",
            "076561198000000001",
            "76561198000000001.0",
            "18446744073709551616",
        ] {
            assert!(parse_peer(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn native_record_layout_and_current_handle() {
        assert_eq!(std::mem::size_of::<NativePlayerBinding>(), 64);
        assert_eq!(
            resolve_with(
                UID,
                7,
                |_, _| Ok(binding()),
                |id| {
                    assert_eq!(id, 2);
                    Some(77)
                },
                |handle| {
                    assert_eq!(handle, 77);
                    Some(2)
                }
            ),
            Ok(77)
        );
    }
    #[test]
    fn refuses_reconnect_or_entity_change_during_handle_resolution() {
        for field in 0..5 {
            let mut calls = 0;
            assert!(
                resolve_with(
                    UID,
                    7,
                    |_, _| {
                        calls += 1;
                        let mut r = binding();
                        if calls == 2 {
                            match field {
                                0 => r.gameplay_key += 0x40,
                                1 => r.machine_key += 0x80,
                                2 => r.entity_generation += 1,
                                3 => r.world_uuid[0] += 1,
                                _ => r.session += 1,
                            }
                        }
                        Ok(r)
                    },
                    |_| Some(77),
                    |_| Some(2)
                )
                .is_err()
            );
        }
    }
    #[test]
    fn refuses_missing_stale_or_mismatched_entity_handle() {
        assert!(resolve_with(UID, 7, |_, _| Ok(binding()), |_| None, |_| Some(2)).is_err());
        assert!(resolve_with(UID, 7, |_, _| Ok(binding()), |_| Some(77), |_| None).is_err());
        assert!(resolve_with(UID, 7, |_, _| Ok(binding()), |_| Some(77), |_| Some(3)).is_err());
        assert!(
            resolve_with(
                UID,
                0,
                |_, _| panic!("zero session reached native code"),
                |_| None,
                |_| None
            )
            .is_err()
        );
    }
    #[test]
    fn refuses_invalid_native_contract_without_resolving_a_handle() {
        for field in 0..7 {
            let mut r = binding();
            match field {
                0 => r.size = 63,
                1 => r.version = 2,
                2 => r.peer_uid += 1,
                3 => r.flags = 1,
                4 => r.world_uuid = [0, 0],
                5 => r.entity_id = 0,
                _ => r.reserved = 1,
            }
            assert!(
                resolve_with(
                    UID,
                    7,
                    |_, _| Ok(r),
                    |_| panic!("invalid binding reached handle resolver"),
                    |_| None
                )
                .is_err()
            );
        }
    }
    #[test]
    fn inactive_host_does_not_fabricate_a_player() {
        assert!(resolve_peer_player("76561198000000001").is_err());
    }
}
