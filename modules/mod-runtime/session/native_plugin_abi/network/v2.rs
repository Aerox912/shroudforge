//! The queried V2 service. Transport calls always occur outside state borrows.
use super::*;
use shroudforge_steam_networking::{
    mod_messages::mod_channel,
    mod_network_frame::{self as wire, Frame, Session},
};
use std::sync::OnceLock;
use std::time::Instant;

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct SendResult {
    pub size: u32,
    pub version: u32,
    pub attempted_frames: u32,
    pub accepted_frames: u32,
    pub delivery_uncertain: u32,
    pub reserved: u32,
}
impl SendResult {
    fn empty() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            version: 2,
            ..Default::default()
        }
    }
}
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Status {
    pub size: u32,
    pub version: u32,
    pub process_target: u32,
    pub available: u32,
    pub service_ready: u32,
    pub logical_ready: u32,
    pub pending_sessions: u32,
    pub logical_peers: u32,
    pub local_steam_id: u64,
    pub connected_dedicated_server_steam_id: u64,
    pub dropped_queue_frames: u64,
}
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Message {
    pub size: u32,
    pub version: u32,
    pub peer_steam_id: u64,
    pub payload_bytes: u32,
    pub flags: u32,
    pub reply_token: u64,
    pub logical_session_id: u64,
}
#[repr(C)]
pub(super) struct Service {
    pub size: u32,
    pub version: u32,
    pub context: *mut c_void,
    pub max_payload_bytes: u32,
    pub max_fragment_bytes: u32,
    pub read_status: Option<unsafe extern "C" fn(*mut c_void, u32, *mut Status) -> i32>,
    pub begin_session: Option<unsafe extern "C" fn(*mut c_void, u32, *mut SendResult) -> i32>,
    pub reset: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    pub send_to_server:
        Option<unsafe extern "C" fn(*mut c_void, *const u8, u32, u32, *mut SendResult) -> i32>,
    pub receive: Option<unsafe extern "C" fn(*mut c_void, *mut u8, u32, u32, *mut Message) -> i32>,
    pub reply:
        Option<unsafe extern "C" fn(*mut c_void, u64, *const u8, u32, u32, *mut SendResult) -> i32>,
}
fn now() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    u64::try_from(START.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
}
fn nonce() -> [u8; 16] {
    *uuid::Uuid::new_v4().as_bytes()
}
fn with_state<T>(
    context: &NativePluginHostContext,
    f: impl FnOnce(&mut state::State) -> T,
) -> Result<T, i32> {
    let mut state = context
        .network
        .v2
        .try_borrow_mut()
        .map_err(|_| NATIVE_STATUS_BUSY)?;
    Ok(f(&mut state))
}
pub(super) fn fresh(
    lease: &OwnerServiceLease<'_>,
    transport: &impl Transport,
    time: u64,
) -> Result<shroudforge_steam_networking::Status, i32> {
    require_network_enabled(lease)?;
    let status = transport.status();
    lease.ensure_active()?;
    require_network_enabled(lease)?;
    lease
        .context
        .network
        .observe_identity(peer_identity(lease.context.process_target, &status))?;
    with_state(lease.context, |s| s.sweep(time))?;
    if !status.available || !status.service_ready {
        with_state(lease.context, |s| s.clear())?;
        return Err(NATIVE_STATUS_UNAVAILABLE);
    }
    if lease.context.process_target == NATIVE_TARGET_SERVER {
        let peers = with_state(lease.context, |s| {
            s.peers.keys().copied().collect::<Vec<_>>()
        })?;
        for peer in peers {
            let allowed = transport.is_authorized_server_peer(peer);
            lease.ensure_active()?;
            if !allowed {
                with_state(lease.context, |s| s.retire_peer(peer))?;
            }
        }
    }
    Ok(status)
}
fn register(context: &NativePluginHostContext) -> Result<u64, i32> {
    with_state(context, |s| s.register(context.package_id()))?
}
pub(in super::super) unsafe fn query(
    context: *mut c_void,
    capacity: u32,
    output: *mut c_void,
) -> i32 {
    if capacity < size_of::<Service>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let Some(owner) = (unsafe { host_context(context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    let _lease = match OwnerServiceLease::acquire(owner) {
        Ok(l) => l,
        Err(e) => return e,
    };
    // Query opts this exact host generation into reserved framing, without
    // asserting policy/transport/session readiness.
    if owner.network_policy.get() == NativeNetworkPolicy::Enabled {
        if let Err(e) = register(owner) {
            return e;
        }
    }
    unsafe {
        output.cast::<Service>().write(Service {
            size: size_of::<Service>() as u32,
            version: 2,
            context,
            max_payload_bytes: wire::MAX_PAYLOAD as u32,
            max_fragment_bytes: wire::CHUNK as u32,
            read_status: Some(read_status),
            begin_session: Some(begin_session),
            reset: Some(reset),
            send_to_server: Some(send_to_server),
            receive: Some(receive),
            reply: Some(reply),
        })
    };
    NATIVE_STATUS_OK
}
unsafe extern "C" fn read_status(context: *mut c_void, size: u32, output: *mut Status) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = (unsafe { host_context(context) }) else {
            return NATIVE_STATUS_INVALID_ARGUMENT;
        };
        status_inner(context, size, output, &SteamTransport, now())
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}
fn status_inner(
    context: &NativePluginHostContext,
    size: u32,
    output: *mut Status,
    transport: &impl Transport,
    time: u64,
) -> i32 {
    let result = (|| {
        let lease = OwnerServiceLease::acquire(context)?;
        if size < size_of::<Status>() as u32 {
            return Err(NATIVE_STATUS_BUFFER_TOO_SMALL);
        }
        if output.is_null() {
            return Err(NATIVE_STATUS_INVALID_ARGUMENT);
        }
        // Disabled policy is visible as zero readiness. Unavailable policy
        // preserves output and returns UNAVAILABLE, matching V1.
        let native = match fresh(&lease, transport, time) {
            Ok(s) => s,
            Err(NATIVE_STATUS_UNAVAILABLE)
                if context.network_policy.get() != NativeNetworkPolicy::Unavailable =>
            {
                unavailable_status()
            }
            Err(e) => return Err(e),
        };
        let identity = peer_identity(context.process_target, &native);
        let (ready, pending, peers) = with_state(context, |s| {
            (
                s.peers.values().filter(|p| p.ready).count(),
                s.peers.values().filter(|p| !p.ready).count(),
                s.peers.len(),
            )
        })?;
        lease.ensure_active()?;
        unsafe {
            output.write(Status {
                size: size_of::<Status>() as u32,
                version: 2,
                process_target: context.process_target,
                available: native.available as u32,
                service_ready: native.service_ready as u32,
                logical_ready: ready as u32,
                pending_sessions: pending as u32,
                logical_peers: peers as u32,
                local_steam_id: identity.local_steam_id.unwrap_or(0),
                connected_dedicated_server_steam_id: identity
                    .connected_server_steam_id
                    .unwrap_or(0),
                dropped_queue_frames: shroudforge_steam_networking::receive_router::drops(),
            })
        };
        Ok(())
    })();
    result.map_or_else(|e| e, |_| NATIVE_STATUS_OK)
}
fn submit(
    lease: &OwnerServiceLease<'_>,
    transport: &impl Transport,
    peer: u64,
    bytes: &[u8],
    report: &mut SendResult,
) -> Result<(), i32> {
    require_network_enabled(lease)?;
    // Once invoked, even an error or reentrant revocation cannot prove that
    // the native transport submitted nothing.
    report.attempted_frames += 1;
    report.delivery_uncertain = 1;
    let sent = transport.send(peer, bytes, mod_channel(lease.context.package_id()));
    if sent.is_ok() {
        report.accepted_frames += 1;
    }
    lease.ensure_active()?;
    require_network_enabled(lease)?;
    sent.map_err(|_| NATIVE_STATUS_FAILED)
}
fn control(owner: &str, kind: u8, session: Session) -> Vec<u8> {
    wire::encode(&Frame {
        kind,
        owner,
        session,
        transfer: 0,
        total: 0,
        index: 0,
        count: 0,
        data: &[],
    })
    .expect("host-generated valid control")
}
fn begin_inner(
    context: &NativePluginHostContext,
    transport: &impl Transport,
    time: u64,
    report: &mut SendResult,
) -> Result<(), i32> {
    let lease = OwnerServiceLease::acquire(context)?;
    if context.process_target != NATIVE_TARGET_CLIENT {
        return Err(NATIVE_STATUS_UNAVAILABLE);
    }
    let status = fresh(&lease, transport, time)?;
    let peer = peer_identity(context.process_target, &status)
        .connected_server_steam_id
        .ok_or(NATIVE_STATUS_UNAVAILABLE)?;
    // Every explicit begin is a reset, including while an earlier HELLO is pending.
    with_state(context, |s| s.clear())?;
    register(context)?;
    let session = Session {
        client: nonce(),
        server: [0; 16],
    };
    with_state(context, |s| s.insert_session(peer, session, false, time))??;
    submit(
        &lease,
        transport,
        peer,
        &control(context.package_id(), wire::HELLO, session),
        report,
    )
}
unsafe extern "C" fn reset(context: *mut c_void) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = (unsafe { host_context(context) }) else {
            return NATIVE_STATUS_INVALID_ARGUMENT;
        };
        if !context.is_owner_thread() {
            return NATIVE_STATUS_WRONG_THREAD;
        }
        if context.services_revoked.get() || context.network.revoked.get() {
            return NATIVE_STATUS_UNAVAILABLE;
        }
        // Reset is intentionally reentrant: invalidate an in-flight lease now.
        context
            .network
            .transition_policy()
            .map_or_else(|e| e, |_| NATIVE_STATUS_OK)
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}
/// Valid result output is initialized even on operation errors. Invalid result
/// pointer/size is the sole send-output exception and submits nothing.
fn report_call(
    size: u32,
    output: *mut SendResult,
    f: impl FnOnce(&mut SendResult) -> Result<(), i32>,
) -> i32 {
    if size < size_of::<SendResult>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let mut report = SendResult::empty();
    let status = catch_unwind(AssertUnwindSafe(|| f(&mut report)))
        .map_or(NATIVE_STATUS_FAILED, |r| {
            r.map_or_else(|e| e, |_| NATIVE_STATUS_OK)
        });
    unsafe { output.write(report) };
    status
}
unsafe extern "C" fn begin_session(
    context: *mut c_void,
    size: u32,
    output: *mut SendResult,
) -> i32 {
    report_call(size, output, |r| {
        let context = unsafe { host_context(context) }.ok_or(NATIVE_STATUS_INVALID_ARGUMENT)?;
        begin_inner(context, &SteamTransport, now(), r)
    })
}
fn copy_payload(ptr: *const u8, len: u32) -> Result<Vec<u8>, i32> {
    if len as usize > wire::MAX_PAYLOAD {
        return Err(NATIVE_STATUS_TOO_LARGE);
    }
    if len > 0 && ptr.is_null() {
        return Err(NATIVE_STATUS_INVALID_ARGUMENT);
    }
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize) }
    };
    std::str::from_utf8(bytes).map_err(|_| NATIVE_STATUS_INVALID_ARGUMENT)?;
    Ok(bytes.to_vec())
}
fn send_inner(
    context: &NativePluginHostContext,
    token: Option<u64>,
    ptr: *const u8,
    len: u32,
    transport: &impl Transport,
    time: u64,
    report: &mut SendResult,
) -> Result<(), i32> {
    let lease = OwnerServiceLease::acquire(context)?;
    let payload = copy_payload(ptr, len)?;
    let status = fresh(&lease, transport, time)?;
    register(context)?;
    let peer = if let Some(token) = token {
        if token == 0 {
            return Err(NATIVE_STATUS_INVALID_ARGUMENT);
        }
        let route = with_state(context, |s| {
            s.reply.filter(|r| {
                r.token == token
                    && r.generation == context.package_generation
                    && s.matching(r.peer, r.session, r.serial)
            })
        })?
        .ok_or(NATIVE_STATUS_UNAVAILABLE)?;
        route.peer
    } else {
        if context.process_target != NATIVE_TARGET_CLIENT {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        peer_identity(context.process_target, &status)
            .connected_server_steam_id
            .ok_or(NATIVE_STATUS_UNAVAILABLE)?
    };
    let (session, serial) =
        with_state(context, |s| s.route(peer))?.ok_or(NATIVE_STATUS_UNAVAILABLE)?;
    let id = with_state(context, |s| s.next_transfer())??;
    let count = wire::fragment_count(payload.len()).ok_or(NATIVE_STATUS_TOO_LARGE)?;
    for index in 0..count {
        // Recheck both transport-selected identity and authorization for every
        // physical submission; an earlier accepted fragment is never replayed.
        fresh(&lease, transport, time)?;
        if !with_state(context, |s| s.matching(peer, session, serial))? {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        let start = index * wire::CHUNK;
        let end = (start + wire::CHUNK).min(payload.len());
        let bytes = wire::encode(&Frame {
            kind: wire::DATA,
            owner: context.package_id(),
            session,
            transfer: id,
            total: payload.len(),
            index,
            count,
            data: &payload[start..end],
        })
        .ok_or(NATIVE_STATUS_FAILED)?;
        submit(&lease, transport, peer, &bytes, report)?;
    }
    with_state(context, |s| {
        if let Some(p) = s.peers.get_mut(&peer) {
            p.touched = time;
        }
    })?;
    Ok(())
}
unsafe extern "C" fn send_to_server(
    context: *mut c_void,
    ptr: *const u8,
    len: u32,
    size: u32,
    output: *mut SendResult,
) -> i32 {
    report_call(size, output, |r| {
        let context = unsafe { host_context(context) }.ok_or(NATIVE_STATUS_INVALID_ARGUMENT)?;
        send_inner(context, None, ptr, len, &SteamTransport, now(), r)
    })
}
unsafe extern "C" fn reply(
    context: *mut c_void,
    token: u64,
    ptr: *const u8,
    len: u32,
    size: u32,
    output: *mut SendResult,
) -> i32 {
    report_call(size, output, |r| {
        let context = unsafe { host_context(context) }.ok_or(NATIVE_STATUS_INVALID_ARGUMENT)?;
        send_inner(context, Some(token), ptr, len, &SteamTransport, now(), r)
    })
}
pub(super) fn receive_inner(
    context: &NativePluginHostContext,
    ptr: *mut u8,
    capacity: u32,
    size: u32,
    output: *mut Message,
    transport: &impl Transport,
    time: u64,
) -> Result<(), i32> {
    let lease = OwnerServiceLease::acquire(context)?;
    // Every admitted receive retires the last logical route, even on bad args.
    with_state(context, |s| s.reply = None)?;
    if size < size_of::<Message>() as u32 {
        return Err(NATIVE_STATUS_BUFFER_TOO_SMALL);
    }
    if output.is_null() || (capacity > 0 && ptr.is_null()) {
        return Err(NATIVE_STATUS_INVALID_ARGUMENT);
    }
    let status = fresh(&lease, transport, time)?;
    let registration = register(context)?;
    // A completion survives a short buffer without consuming another frame.
    if !with_state(context, |s| s.completed().is_some())? {
        let received = transport.receive_v2(mod_channel(context.package_id()), registration);
        lease.ensure_active()?;
        require_network_enabled(&lease)?;
        let message = received
            .map_err(|_| NATIVE_STATUS_FAILED)?
            .ok_or(NATIVE_STATUS_NOT_FOUND)?;
        let peer = parse_steam_id(&message.peer_steam_id).ok_or(NATIVE_STATUS_NOT_FOUND)?;
        if !message.reliable {
            return Err(NATIVE_STATUS_NOT_FOUND);
        }
        if context.process_target == NATIVE_TARGET_CLIENT {
            if peer_identity(context.process_target, &status).connected_server_steam_id
                != Some(peer)
            {
                return Err(NATIVE_STATUS_NOT_FOUND);
            }
        } else {
            let authorized = transport.is_authorized_server_peer(peer);
            lease.ensure_active()?;
            if !authorized {
                with_state(context, |s| s.retire_peer(peer))?;
                return Err(NATIVE_STATUS_NOT_FOUND);
            }
        }
        let Some(frame) =
            wire::decode(&message.payload).filter(|f| f.owner == context.package_id())
        else {
            with_state(context, |s| s.malformed(peer, &message.payload))?;
            return Err(NATIVE_STATUS_NOT_FOUND);
        };
        match frame.kind {
            wire::HELLO if context.process_target == NATIVE_TARGET_SERVER => {
                let existing = with_state(context, |s| {
                    s.peers
                        .get(&peer)
                        .filter(|p| p.session.client == frame.session.client)
                        .map(|p| p.session)
                })?;
                let session = existing.unwrap_or(Session {
                    client: frame.session.client,
                    server: nonce(),
                });
                if existing.is_none() {
                    with_state(context, |s| s.insert_session(peer, session, true, time))??;
                }
                let mut report = SendResult::empty();
                submit(
                    &lease,
                    transport,
                    peer,
                    &control(context.package_id(), wire::ACK, session),
                    &mut report,
                )?;
            }
            wire::ACK if context.process_target == NATIVE_TARGET_CLIENT => {
                with_state(context, |s| s.accept_ack(peer, frame.session, time))?;
            }
            wire::DATA => {
                with_state(context, |s| s.admit(peer, &frame, time))?;
            }
            _ => {}
        }
    }
    lease.ensure_active()?;
    // No FFI/callback occurs within this borrow. Copy succeeds before a reply
    // token is committed, and BUFFER_TOO_SMALL cannot touch caller payload.
    with_state(context, |s| {
        let (peer, _, serial, bytes) = s.completed().ok_or(NATIVE_STATUS_NOT_FOUND)?;
        let mut metadata = Message {
            size: size_of::<Message>() as u32,
            version: 2,
            peer_steam_id: peer,
            payload_bytes: bytes.len() as u32,
            flags: 1,
            reply_token: 0,
            logical_session_id: serial,
        };
        if (capacity as usize) < bytes.len() {
            unsafe { output.write(metadata) };
            return Err(NATIVE_STATUS_BUFFER_TOO_SMALL);
        }
        let route = s.prepare_reply(peer, context.package_generation)?;
        if !bytes.is_empty() {
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
        }
        metadata.reply_token = route.token;
        s.publish_reply(route);
        unsafe { output.write(metadata) };
        Ok(())
    })?
}
unsafe extern "C" fn receive(
    context: *mut c_void,
    ptr: *mut u8,
    capacity: u32,
    size: u32,
    output: *mut Message,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = (unsafe { host_context(context) }) else {
            return NATIVE_STATUS_INVALID_ARGUMENT;
        };
        receive_inner(context, ptr, capacity, size, output, &SteamTransport, now())
            .map_or_else(|e| e, |_| NATIVE_STATUS_OK)
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
