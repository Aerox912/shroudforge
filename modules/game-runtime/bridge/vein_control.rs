//! Binding-qualified Vein desired intent. A lease never proves mining readiness.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct NativeLease {
    size: u32,
    version: u32,
    token: [u64; 2],
    ttl_ms: u32,
    renew_after_ms: u32,
    flags: u32,
    reserved: u32,
}
impl Default for NativeLease {
    fn default() -> Self {
        Self {
            size: 40,
            version: 1,
            token: [0; 2],
            ttl_ms: 0,
            renew_after_ms: 0,
            flags: 0,
            reserved: 0,
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NativeUpdate {
    size: u32,
    version: u32,
    token: [u64; 2],
    sequence: u32,
    active: u32,
    tier: u32,
    reserved: u32,
}
#[derive(Clone, Copy)]
enum Request {
    Begin,
    Set(NativeUpdate),
    Revoke([u64; 2]),
}
#[derive(Debug, PartialEq, Eq)]
pub struct VeinLease {
    /// None with zero durations is an Off acknowledgement, not a usable lease.
    pub token: Option<String>,
    pub ttl_ms: u32,
    pub renew_after_ms: u32,
}
fn parse_token(text: &str) -> Result<[u64; 2], &'static str> {
    if text.len() != 32 {
        return Err("Vein token must contain exactly 32 lowercase hex characters");
    }
    let mut bytes = [0u8; 16];
    for (i, pair) in text.as_bytes().chunks_exact(2).enumerate() {
        let digit = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            _ => None,
        };
        bytes[i] = (digit(pair[0]).ok_or("invalid Vein token")? << 4)
            | digit(pair[1]).ok_or("invalid Vein token")?;
    }
    let token = [
        u64::from_le_bytes(bytes[..8].try_into().unwrap()),
        u64::from_le_bytes(bytes[8..].try_into().unwrap()),
    ];
    if token == [0; 2] {
        return Err("zero Vein token is not a lease");
    }
    Ok(token)
}
fn format_token(token: [u64; 2]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(32);
    for byte in token.into_iter().flat_map(u64::to_le_bytes) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}
fn status(code: i32) -> Result<(), &'static str> {
    match code {
        0 => Ok(()),
        1 => Err("invalid Vein control request"),
        4 => Err("native Vein control busy or full"),
        6 => Err("native Vein player binding ambiguous"),
        7 => Err("native Vein control fault"),
        _ => Err("native Vein lease unavailable, stale or expired"),
    }
}
fn execute_with(
    uid: u64,
    session: u64,
    request: Request,
    mut call: impl FnMut(u64, u64, Request) -> Result<(i32, NativeLease), &'static str>,
    mut current_session: impl FnMut() -> u64,
    mut revoke: impl FnMut(u64, [u64; 2]) -> Result<(), &'static str>,
) -> Result<VeinLease, &'static str> {
    let (code, value) = call(uid, session, request)?;
    status(code)?;
    let clearing = matches!(request, Request::Set(NativeUpdate { active: 0, .. }));
    let empty = value.token == [0; 2] && value.ttl_ms == 0 && value.renew_after_ms == 0;
    let valid = value.size == 40
        && value.version == 1
        && value.flags == 0
        && value.reserved == 0
        && ((clearing && empty)
            || (value.token != [0; 2] && value.ttl_ms == 2000 && value.renew_after_ms == 500));
    if !valid {
        if value.token != [0; 2] {
            let _ = revoke(uid, value.token);
        }
        return Err("invalid native Vein lease reply");
    }
    if empty {
        return Ok(VeinLease {
            token: None,
            ttl_ms: 0,
            renew_after_ms: 0,
        });
    }
    if session == 0 || current_session() != session {
        let _ = revoke(uid, value.token);
        return Err("world session changed during Vein request");
    }
    Ok(VeinLease {
        token: Some(format_token(value.token)),
        ttl_ms: value.ttl_ms,
        renew_after_ms: value.renew_after_ms,
    })
}
#[cfg(windows)]
fn call_native(
    uid: u64,
    session: u64,
    request: Request,
) -> Result<(i32, NativeLease), &'static str> {
    use std::{
        ffi::{c_char, c_void},
        sync::OnceLock,
    };
    type Begin = unsafe extern "C" fn(u64, u64, *mut NativeLease) -> i32;
    type Set = unsafe extern "C" fn(u64, u64, *const NativeUpdate, *mut NativeLease) -> i32;
    type Revoke = unsafe extern "C" fn(u64, *const u64) -> i32;
    #[derive(Clone, Copy)]
    struct Api {
        begin: Begin,
        set: Set,
        revoke: Revoke,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *const c_void;
    }
    static API: OnceLock<Api> = OnceLock::new();
    let api = if let Some(api) = API.get() {
        *api
    } else {
        let name: Vec<u16> = "kfc-runtime.dll\0".encode_utf16().collect();
        let module = unsafe { GetModuleHandleW(name.as_ptr()) };
        if module.is_null() {
            return Err("native Vein provider unavailable");
        }
        let begin = unsafe { GetProcAddress(module, c"KfcRuntimeBeginPeerVeinV1".as_ptr()) };
        let set = unsafe { GetProcAddress(module, c"KfcRuntimeSetPeerVeinV1".as_ptr()) };
        let revoke = unsafe { GetProcAddress(module, c"KfcRuntimeRevokePeerVeinV1".as_ptr()) };
        if begin.is_null() || set.is_null() || revoke.is_null() {
            return Err("native Vein control unsupported by this provider");
        }
        let api = unsafe {
            Api {
                begin: std::mem::transmute::<*const c_void, Begin>(begin),
                set: std::mem::transmute::<*const c_void, Set>(set),
                revoke: std::mem::transmute::<*const c_void, Revoke>(revoke),
            }
        };
        *API.get_or_init(|| api)
    };
    let mut value = NativeLease::default();
    let code = unsafe {
        match request {
            Request::Begin => (api.begin)(uid, session, &mut value),
            Request::Set(update) => (api.set)(uid, session, &update, &mut value),
            Request::Revoke(token) => (api.revoke)(uid, token.as_ptr()),
        }
    };
    Ok((code, value))
}
#[cfg(not(windows))]
fn call_native(_: u64, _: u64, _: Request) -> Result<(i32, NativeLease), &'static str> {
    Err("native Vein control requires Windows")
}
fn revoke_native(uid: u64, token: [u64; 2]) -> Result<(), &'static str> {
    status(call_native(uid, 0, Request::Revoke(token))?.0)
}
pub fn begin_peer_vein(peer: &str) -> Result<VeinLease, &'static str> {
    let uid = crate::player_ownership::parse_peer(peer)?;
    execute_with(
        uid,
        crate::world_session_id(),
        Request::Begin,
        call_native,
        crate::world_session_id,
        revoke_native,
    )
}
pub fn set_peer_vein(
    peer: &str,
    token: &str,
    sequence: u32,
    active: bool,
    tier: u32,
    allow_renew: bool,
) -> Result<VeinLease, &'static str> {
    let uid = crate::player_ownership::parse_peer(peer)?;
    if sequence == 0 || !(1..=3).contains(&tier) {
        return Err("Vein sequence must be nonzero and tier must be 1, 2 or 3");
    }
    let request = Request::Set(NativeUpdate {
        size: 40,
        version: 1,
        token: parse_token(token)?,
        sequence,
        active: u32::from(active),
        tier,
        reserved: 0,
    });
    // An Off caller without ECS write permission can clear only: native session
    // zero forces retirement after the matching token/sequence checks.
    let session = if allow_renew {
        crate::world_session_id()
    } else {
        0
    };
    execute_with(
        uid,
        session,
        request,
        call_native,
        crate::world_session_id,
        revoke_native,
    )
}
pub fn revoke_peer_vein(peer: &str, token: &str) -> Result<(), &'static str> {
    revoke_native(
        crate::player_ownership::parse_peer(peer)?,
        parse_token(token)?,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn reply() -> NativeLease {
        NativeLease {
            token: [31, 41],
            ttl_ms: 2000,
            renew_after_ms: 500,
            ..Default::default()
        }
    }
    #[test]
    fn abi_and_strict_token_roundtrip() {
        assert_eq!(std::mem::size_of::<NativeLease>(), 40);
        assert_eq!(std::mem::size_of::<NativeUpdate>(), 40);
        assert_eq!(std::mem::offset_of!(NativeLease, ttl_ms), 24);
        assert_eq!(std::mem::offset_of!(NativeUpdate, sequence), 24);
        for token in [[1, 2], [u64::MAX, u64::MAX], [0, u64::MAX]] {
            assert_eq!(parse_token(&format_token(token)), Ok(token));
        }
        for value in [
            "",
            "00000000000000000000000000000000",
            "AB000000000000000000000000000000",
            "gg000000000000000000000000000000",
            "0100000000000000000000000000000 ",
            "010000000000000000000000000000000",
        ] {
            assert!(parse_token(value).is_err());
        }
    }
    #[test]
    fn accepted_intent_has_bounded_native_duration() {
        let value = execute_with(
            55,
            7,
            Request::Begin,
            |uid, epoch, _| {
                assert_eq!((uid, epoch), (55, 7));
                Ok((0, reply()))
            },
            || 7,
            |_, _| panic!(),
        )
        .unwrap();
        assert_eq!(value.token, Some(format_token([31, 41])));
        assert_eq!((value.ttl_ms, value.renew_after_ms), (2000, 500));
    }
    #[test]
    fn every_refusal_and_missing_export_stays_failure() {
        for code in [1, 3, 4, 6, 7, 99] {
            assert!(
                execute_with(
                    1,
                    7,
                    Request::Begin,
                    |_, _, _| Ok((code, reply())),
                    || 7,
                    |_, _| panic!()
                )
                .is_err()
            );
        }
        assert!(
            execute_with(
                1,
                7,
                Request::Begin,
                |_, _, _| Err("missing optional export"),
                || 7,
                |_, _| panic!()
            )
            .is_err()
        );
    }
    #[test]
    fn session_change_revokes_exact_returned_token() {
        let mut revoked = None;
        assert!(
            execute_with(
                55,
                7,
                Request::Begin,
                |_, _, _| Ok((0, reply())),
                || 8,
                |uid, token| {
                    revoked = Some((uid, token));
                    Ok(())
                }
            )
            .is_err()
        );
        assert_eq!(revoked, Some((55, [31, 41])));
    }
    #[test]
    fn empty_off_is_cleared_ack_never_begin_or_on() {
        let off = Request::Set(NativeUpdate {
            size: 40,
            version: 1,
            token: [31, 41],
            sequence: 2,
            active: 0,
            tier: 2,
            reserved: 0,
        });
        assert_eq!(
            execute_with(
                55,
                0,
                off,
                |_, _, _| Ok((0, NativeLease::default())),
                || panic!(),
                |_, _| panic!()
            )
            .unwrap(),
            VeinLease {
                token: None,
                ttl_ms: 0,
                renew_after_ms: 0
            }
        );
        assert!(
            execute_with(
                55,
                7,
                Request::Begin,
                |_, _, _| Ok((0, NativeLease::default())),
                || 7,
                |_, _| panic!()
            )
            .is_err()
        );
    }
    #[test]
    fn malformed_success_is_not_a_lease() {
        for index in 0..5 {
            let mut value = reply();
            match index {
                0 => value.version = 2,
                1 => value.size = 39,
                2 => value.flags = 1,
                3 => value.ttl_ms = 2001,
                _ => value.renew_after_ms = 0,
            }
            let mut revoked = false;
            assert!(
                execute_with(
                    1,
                    7,
                    Request::Begin,
                    |_, _, _| Ok((0, value)),
                    || 7,
                    |_, _| {
                        revoked = true;
                        Ok(())
                    }
                )
                .is_err()
            );
            assert!(revoked);
        }
    }
}
