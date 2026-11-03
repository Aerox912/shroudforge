use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};
mod state;
pub(super) mod v2;

pub(super) const SERVICE: &str = "shroudforge.mod-network";
pub(super) const MAX_PAYLOAD_BYTES: usize = 65_536;

#[repr(C)]
pub(super) struct NetworkStatusV1 {
    pub(super) size: u32,
    pub(super) version: u32,
    pub(super) process_target: u32,
    pub(super) available: u32,
    pub(super) service_ready: u32,
    pub(super) reserved: u32,
    pub(super) local_steam_id: u64,
    pub(super) connected_dedicated_server_steam_id: u64,
}

#[repr(C)]
pub(super) struct NetworkMessageV1 {
    pub(super) size: u32,
    pub(super) version: u32,
    pub(super) peer_steam_id: u64,
    pub(super) payload_bytes: u32,
    pub(super) flags: u32,
    pub(super) reply_token: u64,
}

#[repr(C)]
pub(super) struct NetworkServiceV1 {
    size: u32,
    version: u32,
    context: *mut c_void,
    max_payload_bytes: u32,
    reserved: u32,
    read_status: Option<unsafe extern "C" fn(*mut c_void, u32, *mut NetworkStatusV1) -> i32>,
    send_to_server: Option<unsafe extern "C" fn(*mut c_void, *const u8, u32) -> i32>,
    receive:
        Option<unsafe extern "C" fn(*mut c_void, *mut u8, u32, u32, *mut NetworkMessageV1) -> i32>,
    reply: Option<unsafe extern "C" fn(*mut c_void, u64, *const u8, u32) -> i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PeerIdentity {
    local_steam_id: Option<u64>,
    connected_server_steam_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReplyRoute {
    token: u64,
    package_generation: u64,
    peer_steam_id: u64,
    identity: PeerIdentity,
}

#[derive(Default)]
pub(super) struct OwnerState {
    v2: RefCell<state::State>,
    next_token: Cell<u64>,
    last_reply: RefCell<Option<ReplyRoute>>,
    observed_identity: RefCell<Option<PeerIdentity>>,
    active_calls: Cell<u32>,
    call_epoch: Cell<u64>,
    call_epoch_exhausted: Cell<bool>,
    revoked: Cell<bool>,
}

impl OwnerState {
    pub(super) fn invalidate_reply(&self) -> Result<(), i32> {
        let mut last_reply = self
            .last_reply
            .try_borrow_mut()
            .map_err(|_| NATIVE_STATUS_BUSY)?;
        *last_reply = None;
        Ok(())
    }

    fn observe_identity(&self, identity: PeerIdentity) -> Result<(), i32> {
        let mut observed = self
            .observed_identity
            .try_borrow_mut()
            .map_err(|_| NATIVE_STATUS_BUSY)?;
        if observed.is_some_and(|previous| previous != identity) {
            self.invalidate_reply()?;
            self.v2
                .try_borrow_mut()
                .map_err(|_| NATIVE_STATUS_BUSY)?
                .clear();
        }
        *observed = Some(identity);
        Ok(())
    }

    fn issue_reply(
        &self,
        package_generation: u64,
        peer_steam_id: u64,
        identity: PeerIdentity,
    ) -> Result<u64, i32> {
        if self.revoked.get() || self.call_epoch_exhausted.get() {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        let token = self
            .next_token
            .get()
            .checked_add(1)
            .ok_or(NATIVE_STATUS_UNAVAILABLE)?;
        let mut last_reply = self
            .last_reply
            .try_borrow_mut()
            .map_err(|_| NATIVE_STATUS_BUSY)?;
        self.next_token.set(token);
        *last_reply = Some(ReplyRoute {
            token,
            package_generation,
            peer_steam_id,
            identity,
        });
        Ok(token)
    }

    fn reply_route(&self, token: u64) -> Result<Option<ReplyRoute>, i32> {
        let last_reply = self
            .last_reply
            .try_borrow()
            .map_err(|_| NATIVE_STATUS_BUSY)?;
        Ok(last_reply
            .as_ref()
            .filter(|route| route.token == token)
            .copied())
    }

    fn reply_route_for_any(&self) -> Result<Option<ReplyRoute>, i32> {
        let last_reply = self
            .last_reply
            .try_borrow()
            .map_err(|_| NATIVE_STATUS_BUSY)?;
        Ok(*last_reply)
    }

    pub(super) fn revoke(&self) {
        self.revoked.set(true);
        self.advance_call_epoch();
        if let Ok(mut state) = self.v2.try_borrow_mut() {
            state.clear();
        }
        if let Ok(mut reply) = self.last_reply.try_borrow_mut() {
            *reply = None;
        }
        if let Ok(mut identity) = self.observed_identity.try_borrow_mut() {
            *identity = None;
        }
    }

    fn begin_call(&self) -> Result<u64, i32> {
        if self.revoked.get() || self.call_epoch_exhausted.get() {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        if self.active_calls.get() != 0 {
            return Err(NATIVE_STATUS_BUSY);
        }
        self.active_calls.set(1);
        Ok(self.call_epoch.get())
    }

    fn end_call(&self) {
        self.active_calls
            .set(self.active_calls.get().saturating_sub(1));
    }

    pub(super) fn has_active_call(&self) -> bool {
        self.active_calls.get() != 0
    }

    fn validate_call(&self, call_epoch: u64) -> Result<(), i32> {
        if self.revoked.get()
            || self.call_epoch_exhausted.get()
            || self.call_epoch.get() != call_epoch
        {
            Err(NATIVE_STATUS_UNAVAILABLE)
        } else {
            Ok(())
        }
    }

    pub(super) fn transition_policy(&self) -> Result<(), i32> {
        let advanced = self.advance_call_epoch();
        self.invalidate_reply()?;
        self.v2
            .try_borrow_mut()
            .map_err(|_| NATIVE_STATUS_BUSY)?
            .clear();
        if advanced {
            Ok(())
        } else {
            Err(NATIVE_STATUS_UNAVAILABLE)
        }
    }

    fn advance_call_epoch(&self) -> bool {
        match self.call_epoch.get().checked_add(1) {
            Some(next) => {
                self.call_epoch.set(next);
                true
            }
            None => {
                self.call_epoch_exhausted.set(true);
                false
            }
        }
    }
}

trait Transport {
    fn status(&self) -> shroudforge_steam_networking::Status;
    fn send(&self, peer_steam_id: u64, payload: &[u8], channel: i32) -> Result<(), String>;
    fn receive(
        &self,
        channel: i32,
    ) -> Result<Option<shroudforge_steam_networking::Message>, String>;
    fn is_authorized_server_peer(&self, peer_steam_id: u64) -> bool;
    fn receive_v2(
        &self,
        channel: i32,
        registration: u64,
    ) -> Result<Option<shroudforge_steam_networking::Message>, String> {
        shroudforge_steam_networking::receive_router::poll_with(channel, Some(registration), |c| {
            self.receive(c)
        })
    }
}

impl NativePluginHostContext {
    fn package_id(&self) -> &str {
        std::str::from_utf8(&self.package_id_utf8).unwrap_or("")
    }
}

struct SteamTransport;

impl Transport for SteamTransport {
    fn receive_v2(
        &self,
        channel: i32,
        registration: u64,
    ) -> Result<Option<shroudforge_steam_networking::Message>, String> {
        shroudforge_steam_networking::receive_v2(channel, registration)
    }
    fn status(&self) -> shroudforge_steam_networking::Status {
        shroudforge_steam_networking::status()
    }

    fn send(&self, peer_steam_id: u64, payload: &[u8], channel: i32) -> Result<(), String> {
        shroudforge_steam_networking::send(peer_steam_id, payload, channel, true)
    }

    fn receive(
        &self,
        channel: i32,
    ) -> Result<Option<shroudforge_steam_networking::Message>, String> {
        shroudforge_steam_networking::receive(channel)
    }

    fn is_authorized_server_peer(&self, peer_steam_id: u64) -> bool {
        shroudforge_steam_networking::is_authorized_server_peer(peer_steam_id)
    }
}

pub(super) struct OwnerServiceLease<'a> {
    context: &'a NativePluginHostContext,
    call_epoch: u64,
}

impl<'a> OwnerServiceLease<'a> {
    // One owner-scoped admission slot prevents callback reentry while an
    // external transport call is on the stack. Drop releases it on every exit.
    pub(super) fn acquire(context: &'a NativePluginHostContext) -> Result<Self, i32> {
        if !context.is_owner_thread() {
            return Err(NATIVE_STATUS_WRONG_THREAD);
        }
        if context.services_revoked.get() {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        let is_server = context.process_target == NATIVE_TARGET_SERVER;
        if !matches!(
            context.process_target,
            NATIVE_TARGET_CLIENT | NATIVE_TARGET_SERVER
        ) || !context.package.info().target.supports_process(is_server)
            || !context
                .package
                .info()
                .capabilities
                .contains(&mod_loader::Capability::Runtime)
        {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        let call_epoch = context.network.begin_call()?;
        Ok(Self {
            context,
            call_epoch,
        })
    }

    fn ensure_active(&self) -> Result<(), i32> {
        if self.context.services_revoked.get() {
            return Err(NATIVE_STATUS_UNAVAILABLE);
        }
        self.context.network.validate_call(self.call_epoch)
    }
}

impl Drop for OwnerServiceLease<'_> {
    fn drop(&mut self) {
        self.context.network.end_call();
    }
}

pub(super) unsafe fn query(context: *mut c_void, capacity: u32, output: *mut c_void) -> i32 {
    if capacity < size_of::<NetworkServiceV1>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let Some(owner) = (unsafe { host_context(context) }) else {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    };
    let _lease = match OwnerServiceLease::acquire(owner) {
        Ok(lease) => lease,
        Err(status) => return status,
    };
    unsafe {
        output.cast::<NetworkServiceV1>().write(NetworkServiceV1 {
            size: size_of::<NetworkServiceV1>() as u32,
            version: 1,
            context,
            max_payload_bytes: MAX_PAYLOAD_BYTES as u32,
            reserved: 0,
            read_status: Some(read_status),
            send_to_server: Some(send_to_server),
            receive: Some(receive),
            reply: Some(reply),
        });
    }
    NATIVE_STATUS_OK
}

unsafe extern "C" fn read_status(
    context: *mut c_void,
    out_size: u32,
    output: *mut NetworkStatusV1,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = (unsafe { host_context(context) }) else {
            return NATIVE_STATUS_INVALID_ARGUMENT;
        };
        read_status_inner(context, out_size, output, &SteamTransport)
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

fn read_status_inner(
    context: &NativePluginHostContext,
    out_size: u32,
    output: *mut NetworkStatusV1,
    transport: &impl Transport,
) -> i32 {
    let lease = match OwnerServiceLease::acquire(context) {
        Ok(lease) => lease,
        Err(status) => return status,
    };
    if out_size < size_of::<NetworkStatusV1>() as u32 {
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if output.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }

    let policy = lease.context.network_policy.get();
    if policy == NativeNetworkPolicy::Unavailable {
        let _ = lease.context.network.invalidate_reply();
        return NATIVE_STATUS_UNAVAILABLE;
    }
    let enabled = policy == NativeNetworkPolicy::Enabled;
    if !enabled {
        if let Err(status) = lease.context.network.invalidate_reply() {
            return status;
        }
    }
    let native_status = enabled.then(|| transport.status());
    if let Err(status) = lease.ensure_active() {
        return status;
    }
    let identity = native_status
        .as_ref()
        .map(|status| peer_identity(lease.context.process_target, status))
        .unwrap_or(PeerIdentity {
            local_steam_id: None,
            connected_server_steam_id: None,
        });
    if let Err(status) = lease.context.network.observe_identity(identity) {
        return status;
    }
    if lease.context.process_target == NATIVE_TARGET_SERVER {
        if let Ok(Some(route)) = lease.context.network.reply_route_for_any() {
            let authorized = transport.is_authorized_server_peer(route.peer_steam_id);
            if let Err(status) = lease.ensure_active() {
                return status;
            }
            if !authorized {
                if let Err(status) = lease.context.network.invalidate_reply() {
                    return status;
                }
            }
        }
    }

    let native_status = native_status.unwrap_or_else(unavailable_status);
    let available = enabled && native_status.available;
    let service_ready = available && native_status.service_ready;
    let result = NetworkStatusV1 {
        size: size_of::<NetworkStatusV1>() as u32,
        version: 1,
        process_target: lease.context.process_target,
        available: u32::from(available),
        service_ready: u32::from(service_ready),
        reserved: 0,
        local_steam_id: if enabled && native_status.available {
            native_status
                .local_steam_id
                .as_deref()
                .and_then(parse_steam_id)
                .unwrap_or(0)
        } else {
            0
        },
        connected_dedicated_server_steam_id: if enabled
            && native_status.available
            && native_status.service_ready
            && lease.context.process_target == NATIVE_TARGET_CLIENT
        {
            native_status
                .remote_dedicated_server_steam_id
                .as_deref()
                .and_then(parse_steam_id)
                .unwrap_or(0)
        } else {
            0
        },
    };
    if let Err(status) = lease.ensure_active() {
        return status;
    }
    unsafe { output.write(result) };
    NATIVE_STATUS_OK
}

unsafe extern "C" fn send_to_server(
    context: *mut c_void,
    payload: *const u8,
    payload_length: u32,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = (unsafe { host_context(context) }) else {
            return NATIVE_STATUS_INVALID_ARGUMENT;
        };
        send_to_server_inner(context, payload, payload_length, &SteamTransport)
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

fn send_to_server_inner(
    context: &NativePluginHostContext,
    payload: *const u8,
    payload_length: u32,
    transport: &impl Transport,
) -> i32 {
    let lease = match OwnerServiceLease::acquire(context) {
        Ok(lease) => lease,
        Err(status) => return status,
    };
    if lease.context.process_target != NATIVE_TARGET_CLIENT {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    let payload = match copy_utf8_payload(payload, payload_length) {
        Ok(payload) => payload,
        Err(status) => return status,
    };
    if let Err(status) = require_network_enabled(&lease) {
        return status;
    }
    let status = transport.status();
    if let Err(status) = lease.ensure_active() {
        return status;
    }
    let identity = peer_identity(lease.context.process_target, &status);
    if let Err(status) = lease.context.network.observe_identity(identity) {
        return status;
    }
    if !status.available || !status.service_ready {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    let Some(peer) = identity.connected_server_steam_id else {
        return NATIVE_STATUS_UNAVAILABLE;
    };
    send_owner_envelope(&lease, peer, &payload, transport)
}

unsafe extern "C" fn receive(
    context: *mut c_void,
    output_payload: *mut u8,
    capacity: u32,
    metadata_size: u32,
    output_metadata: *mut NetworkMessageV1,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = (unsafe { host_context(context) }) else {
            return NATIVE_STATUS_INVALID_ARGUMENT;
        };
        receive_inner(
            context,
            output_payload,
            capacity,
            metadata_size,
            output_metadata,
            &SteamTransport,
        )
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

fn receive_inner(
    context: &NativePluginHostContext,
    output_payload: *mut u8,
    capacity: u32,
    metadata_size: u32,
    output_metadata: *mut NetworkMessageV1,
    transport: &impl Transport,
) -> i32 {
    let lease = match OwnerServiceLease::acquire(context) {
        Ok(lease) => lease,
        Err(status) => return status,
    };
    if let Err(status) = lease.context.network.invalidate_reply() {
        return status;
    }
    if metadata_size < size_of::<NetworkMessageV1>() as u32 {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    if output_metadata.is_null() || (capacity > 0 && output_payload.is_null()) {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    if let Err(status) = require_network_enabled(&lease) {
        return status;
    }

    let status = transport.status();
    if let Err(status) = lease.ensure_active() {
        return status;
    }
    let identity = peer_identity(lease.context.process_target, &status);
    if let Err(status) = lease.context.network.observe_identity(identity) {
        return status;
    }
    if !status.available || !status.service_ready {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    if lease.context.process_target == NATIVE_TARGET_CLIENT
        && identity.connected_server_steam_id.is_none()
    {
        return NATIVE_STATUS_UNAVAILABLE;
    }

    let received = transport.receive(shroudforge_steam_networking::mod_messages::mod_channel(
        lease.context.package_id(),
    ));
    if let Err(status) = lease.ensure_active() {
        return status;
    }
    let message = match received {
        Ok(Some(message)) => message,
        Ok(None) => return NATIVE_STATUS_NOT_FOUND,
        Err(_) => return NATIVE_STATUS_FAILED,
    };
    if message.payload.len() > shroudforge_steam_networking::MAX_MESSAGE_BYTES {
        return NATIVE_STATUS_TOO_LARGE;
    }
    let Some(peer) = parse_steam_id(&message.peer_steam_id) else {
        return NATIVE_STATUS_NOT_FOUND;
    };
    if lease.context.process_target == NATIVE_TARGET_CLIENT {
        if identity.connected_server_steam_id != Some(peer) {
            return NATIVE_STATUS_NOT_FOUND;
        }
    } else {
        let authorized = transport.is_authorized_server_peer(peer);
        if let Err(status) = lease.ensure_active() {
            return status;
        }
        if !authorized {
            return NATIVE_STATUS_NOT_FOUND;
        }
    }
    let Some(envelope) = shroudforge_steam_networking::mod_messages::decode_mod_message(
        &message.payload,
        lease.context.package_id(),
    ) else {
        return NATIVE_STATUS_NOT_FOUND;
    };
    if envelope.from_mod != lease.context.package_id() {
        return NATIVE_STATUS_NOT_FOUND;
    }
    if envelope.payload.len() > MAX_PAYLOAD_BYTES {
        return NATIVE_STATUS_TOO_LARGE;
    }
    let payload = envelope.payload.as_bytes();
    let metadata = |reply_token| NetworkMessageV1 {
        size: size_of::<NetworkMessageV1>() as u32,
        version: 1,
        peer_steam_id: peer,
        payload_bytes: payload.len() as u32,
        flags: if message.reliable { 1 } else { 0 },
        reply_token,
    };
    if (capacity as usize) < payload.len() {
        unsafe { output_metadata.write(metadata(0)) };
        return NATIVE_STATUS_BUFFER_TOO_SMALL;
    }
    if !payload.is_empty() && output_payload.is_null() {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    if let Err(status) = lease.ensure_active() {
        return status;
    }
    let reply_token =
        match lease
            .context
            .network
            .issue_reply(lease.context.package_generation, peer, identity)
        {
            Ok(token) => token,
            Err(status) => return status,
        };
    if !payload.is_empty() {
        unsafe { std::ptr::copy_nonoverlapping(payload.as_ptr(), output_payload, payload.len()) };
    }
    unsafe { output_metadata.write(metadata(reply_token)) };
    NATIVE_STATUS_OK
}

unsafe extern "C" fn reply(
    context: *mut c_void,
    reply_token: u64,
    payload: *const u8,
    payload_length: u32,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        let Some(context) = (unsafe { host_context(context) }) else {
            return NATIVE_STATUS_INVALID_ARGUMENT;
        };
        reply_inner(
            context,
            reply_token,
            payload,
            payload_length,
            &SteamTransport,
        )
    }))
    .unwrap_or(NATIVE_STATUS_FAILED)
}

fn reply_inner(
    context: &NativePluginHostContext,
    reply_token: u64,
    payload: *const u8,
    payload_length: u32,
    transport: &impl Transport,
) -> i32 {
    let lease = match OwnerServiceLease::acquire(context) {
        Ok(lease) => lease,
        Err(status) => return status,
    };
    if reply_token == 0 {
        return NATIVE_STATUS_INVALID_ARGUMENT;
    }
    let payload = match copy_utf8_payload(payload, payload_length) {
        Ok(payload) => payload,
        Err(status) => return status,
    };
    if let Err(status) = require_network_enabled(&lease) {
        return status;
    }
    let status = transport.status();
    if let Err(status) = lease.ensure_active() {
        return status;
    }
    let identity = peer_identity(lease.context.process_target, &status);
    if let Err(status) = lease.context.network.observe_identity(identity) {
        return status;
    }
    if !status.available || !status.service_ready {
        return NATIVE_STATUS_UNAVAILABLE;
    }
    let route = match lease.context.network.reply_route(reply_token) {
        Ok(Some(route)) if route.package_generation == lease.context.package_generation => route,
        Ok(_) => return NATIVE_STATUS_UNAVAILABLE,
        Err(status) => return status,
    };
    if route.identity != identity {
        let _ = lease.context.network.invalidate_reply();
        return NATIVE_STATUS_UNAVAILABLE;
    }
    let authorized = match lease.context.process_target {
        NATIVE_TARGET_CLIENT if identity.connected_server_steam_id == Some(route.peer_steam_id) => {
            true
        }
        NATIVE_TARGET_SERVER => {
            let authorized = transport.is_authorized_server_peer(route.peer_steam_id);
            if let Err(status) = lease.ensure_active() {
                return status;
            }
            authorized
        }
        _ => false,
    };
    if !authorized {
        let _ = lease.context.network.invalidate_reply();
        return NATIVE_STATUS_UNAVAILABLE;
    }
    send_owner_envelope(&lease, route.peer_steam_id, &payload, transport)
}

fn send_owner_envelope(
    lease: &OwnerServiceLease<'_>,
    peer: u64,
    payload: &str,
    transport: &impl Transport,
) -> i32 {
    let context = lease.context;
    let bytes = match shroudforge_steam_networking::mod_messages::encode_mod_message(
        context.package_id(),
        context.package_id(),
        payload,
    ) {
        Ok(bytes) => bytes,
        Err(_) => return NATIVE_STATUS_FAILED,
    };
    if bytes.len() > shroudforge_steam_networking::MAX_MESSAGE_BYTES {
        return NATIVE_STATUS_TOO_LARGE;
    }
    if let Err(status) = lease.ensure_active() {
        return status;
    }
    let sent = transport.send(
        peer,
        &bytes,
        shroudforge_steam_networking::mod_messages::mod_channel(context.package_id()),
    );
    if let Err(status) = lease.ensure_active() {
        return status;
    }
    sent.map_or(NATIVE_STATUS_FAILED, |_| NATIVE_STATUS_OK)
}

fn copy_utf8_payload(payload: *const u8, length: u32) -> Result<String, i32> {
    let length = length as usize;
    if length > MAX_PAYLOAD_BYTES {
        return Err(NATIVE_STATUS_TOO_LARGE);
    }
    if length > 0 && payload.is_null() {
        return Err(NATIVE_STATUS_INVALID_ARGUMENT);
    }
    let bytes = if length == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(payload, length) }
    };
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| NATIVE_STATUS_INVALID_ARGUMENT)
}

fn require_network_enabled(lease: &OwnerServiceLease<'_>) -> Result<(), i32> {
    lease.ensure_active()?;
    match lease.context.network_policy.get() {
        NativeNetworkPolicy::Enabled => Ok(()),
        NativeNetworkPolicy::Disabled | NativeNetworkPolicy::Unavailable => {
            lease.context.network.invalidate_reply()?;
            Err(NATIVE_STATUS_UNAVAILABLE)
        }
    }
}

fn peer_identity(
    process_target: u32,
    status: &shroudforge_steam_networking::Status,
) -> PeerIdentity {
    PeerIdentity {
        local_steam_id: status
            .available
            .then(|| status.local_steam_id.as_deref().and_then(parse_steam_id))
            .flatten(),
        connected_server_steam_id: (process_target == NATIVE_TARGET_CLIENT
            && status.available
            && status.service_ready)
            .then(|| {
                status
                    .remote_dedicated_server_steam_id
                    .as_deref()
                    .and_then(parse_steam_id)
            })
            .flatten(),
    }
}

fn parse_steam_id(value: &str) -> Option<u64> {
    let id = value.parse::<u64>().ok()?;
    (id != 0).then_some(id)
}

fn unavailable_status() -> shroudforge_steam_networking::Status {
    shroudforge_steam_networking::Status {
        available: false,
        local_steam_id: None,
        local_dedicated_server_steam_id: None,
        remote_dedicated_server_steam_id: None,
        service_ready: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v2_is_reachable_through_actual_host_query() {
        let fixture = Fixture::new("v2-query-contract", false, true, true, 300);
        let context = fixture.context();
        let mut output = [0u64; 32];
        let name = SERVICE.as_bytes();
        let result = unsafe {
            host_query_service(
                (context as *const NativePluginHostContext)
                    .cast_mut()
                    .cast(),
                name.as_ptr(),
                name.len() as u32,
                2,
                256,
                output.as_mut_ptr().cast(),
            )
        };
        assert_eq!(result, NATIVE_STATUS_OK);
        assert_eq!((output[0] >> 32) as u32, 2);
    }
    use std::{
        collections::{HashSet, VecDeque},
        fs,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    pub(super) const CLIENT_ID: u64 = 7_656_119_801_234_567;
    pub(super) const SERVER_ID: u64 = 7_656_119_809_876_543;
    const OTHER_SERVER_ID: u64 = 7_656_119_807_654_321;
    const AUTHORIZED_PEER: u64 = 7_656_119_800_000_111;
    const SPOOFED_PEER: u64 = 7_656_119_800_000_222;

    #[derive(Default)]
    struct DrainObserver {
        drains: AtomicUsize,
    }

    impl NativePluginServiceLifetime for DrainObserver {
        unsafe fn query_service(
            &self,
            _package_id: &str,
            _package_generation: u64,
            _service_name: &str,
            _requested_version: u32,
            _out_capacity: u32,
            _out_table: *mut c_void,
        ) -> i32 {
            NATIVE_STATUS_UNAVAILABLE
        }

        fn revoke(&self, _package_id: &str, _package_generation: u64) -> i32 {
            NATIVE_STATUS_OK
        }

        fn drain(&self, _package_id: &str, _package_generation: u64, _timeout_ms: u32) -> i32 {
            self.drains.fetch_add(1, Ordering::SeqCst);
            NATIVE_STATUS_OK
        }
    }

    pub(super) struct Fixture {
        context: Option<Box<NativePluginHostContext>>,
        root: PathBuf,
    }

    impl Fixture {
        pub(super) fn new(
            package_id: &str,
            is_server: bool,
            runtime_capability: bool,
            network_enabled: bool,
            generation: u64,
        ) -> Self {
            Self::with_services(
                package_id,
                is_server,
                runtime_capability,
                network_enabled,
                generation,
                Arc::new(NoNativePluginServices),
            )
        }

        fn with_services(
            package_id: &str,
            is_server: bool,
            runtime_capability: bool,
            network_enabled: bool,
            generation: u64,
            services: Arc<dyn NativePluginServiceLifetime>,
        ) -> Self {
            let target_dir = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let root = target_dir.join(format!("native-network-test-{}", uuid::Uuid::new_v4()));
            let package_root = root.join("mods").join(package_id);
            fs::create_dir_all(&package_root).unwrap();
            shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
            let mut config = shroudforge_loader_config::config::read_loader(&root).unwrap();
            config["modules"]["network"]["enabled"] = network_enabled.into();
            fs::write(
                shroudforge_loader_config::paths::loader_config(&root),
                serde_json::to_vec(&config).unwrap(),
            )
            .unwrap();
            let mut capabilities = vec!["runtime-register-dll"];
            if runtime_capability {
                capabilities.push("runtime");
            }
            fs::write(
                package_root.join("mod.json"),
                serde_json::to_vec(&serde_json::json!({
                    "id": package_id,
                    "name": "Native Network Fixture",
                    "version": "1.0.0",
                    "capabilities": capabilities,
                }))
                .unwrap(),
            )
            .unwrap();
            fs::write(
                package_root.join("extended.mod.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schemaVersion": 1,
                    "enabled": true,
                    "targets": [if is_server { "server" } else { "client" }],
                    "settings": {},
                }))
                .unwrap(),
            )
            .unwrap();
            let env = mod_loader::ModEnvironment::load(camino::Utf8Path::from_path(&root).unwrap())
                .unwrap();
            let package = env.mod_registry().get(package_id).unwrap().clone();
            let package_binding = package.package_binding().clone();
            let context = NativePluginHostContext::new(NativePluginHostMetadata {
                package_id: package_id.to_owned(),
                package,
                package_binding,
                package_generation: generation,
                is_server,
                installation_root: root.clone(),
                settings_json: b"{}".to_vec(),
                services,
            })
            .unwrap();
            Self {
                context: Some(Box::new(context)),
                root,
            }
        }

        pub(super) fn context(&self) -> &NativePluginHostContext {
            self.context.as_deref().unwrap()
        }

        fn context_mut(&mut self) -> &mut NativePluginHostContext {
            self.context.as_deref_mut().unwrap()
        }

        fn set_network_enabled(&self, enabled: bool) {
            let mut config = shroudforge_loader_config::config::read_loader_defaults().unwrap();
            config["modules"]["network"]["enabled"] = enabled.into();
            fs::write(
                shroudforge_loader_config::paths::loader_config(&self.root),
                serde_json::to_vec(&config).unwrap(),
            )
            .unwrap();
        }

        fn write_invalid_loader_config(&self) {
            fs::write(
                shroudforge_loader_config::paths::loader_config(&self.root),
                b"{invalid",
            )
            .unwrap();
        }

        fn refresh_native_network_policy(&self) {
            let config = shroudforge_loader_config::config::read_loader(&self.root).ok();
            self.context()
                .update_network_policy(NativeNetworkPolicy::from_loader_config(config.as_ref()))
                .unwrap();
        }

        fn recreate_context_after_loader_config(&mut self, bytes: &[u8]) {
            let current = self.context.take().unwrap();
            let metadata = NativePluginHostMetadata {
                package_id: current.package_id().to_owned(),
                package: current.package.clone(),
                package_binding: current.package_binding.clone(),
                package_generation: current.package_generation,
                is_server: current.process_target == NATIVE_TARGET_SERVER,
                installation_root: self.root.clone(),
                settings_json: current.settings_json.borrow().clone(),
                services: current.services.clone(),
            };
            fs::write(
                shroudforge_loader_config::paths::loader_config(&self.root),
                bytes,
            )
            .unwrap();
            self.context = Some(Box::new(NativePluginHostContext::new(metadata).unwrap()));
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.context.take();
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    pub(super) struct FakeTransport {
        pub(super) status: RefCell<shroudforge_steam_networking::Status>,
        pub(super) messages: RefCell<VecDeque<Option<shroudforge_steam_networking::Message>>>,
        pub(super) sent: RefCell<Vec<(u64, Vec<u8>, i32)>>,
        pub(super) authorized: RefCell<HashSet<u64>>,
        pub(super) receive_channels: RefCell<Vec<i32>>,
        status_reads: Cell<usize>,
        authorized_reads: Cell<usize>,
        receive_error: Cell<bool>,
        panic_status: Cell<bool>,
        pub(super) on_status: RefCell<Option<Box<dyn FnMut()>>>,
        pub(super) on_send: RefCell<Option<Box<dyn FnMut()>>>,
        pub(super) on_receive: RefCell<Option<Box<dyn FnMut()>>>,
        pub(super) on_authorized: RefCell<Option<Box<dyn FnMut()>>>,
    }

    impl FakeTransport {
        pub(super) fn client(remote_server: Option<u64>) -> Self {
            Self::with_status(status(CLIENT_ID, remote_server, true, true, false))
        }

        pub(super) fn server() -> Self {
            Self::with_status(status(SERVER_ID, None, true, true, true))
        }

        fn with_status(status: shroudforge_steam_networking::Status) -> Self {
            Self {
                status: RefCell::new(status),
                messages: RefCell::new(VecDeque::new()),
                sent: RefCell::new(Vec::new()),
                authorized: RefCell::new(HashSet::new()),
                receive_channels: RefCell::new(Vec::new()),
                status_reads: Cell::new(0),
                authorized_reads: Cell::new(0),
                receive_error: Cell::new(false),
                panic_status: Cell::new(false),
                on_status: RefCell::new(None),
                on_send: RefCell::new(None),
                on_receive: RefCell::new(None),
                on_authorized: RefCell::new(None),
            }
        }

        fn on_status(&self, callback: impl FnMut() + 'static) {
            *self.on_status.borrow_mut() = Some(Box::new(callback));
        }

        fn on_send(&self, callback: impl FnMut() + 'static) {
            *self.on_send.borrow_mut() = Some(Box::new(callback));
        }

        fn on_receive(&self, callback: impl FnMut() + 'static) {
            *self.on_receive.borrow_mut() = Some(Box::new(callback));
        }

        fn on_authorized(&self, callback: impl FnMut() + 'static) {
            *self.on_authorized.borrow_mut() = Some(Box::new(callback));
        }

        fn enqueue(&self, message: shroudforge_steam_networking::Message) {
            self.messages.borrow_mut().push_back(Some(message));
        }

        fn message(&self, peer: u64, from: &str, to: &str, payload: &str) {
            self.enqueue(shroudforge_steam_networking::Message {
                peer_steam_id: peer.to_string(),
                payload: shroudforge_steam_networking::mod_messages::encode_mod_message(
                    from, to, payload,
                )
                .unwrap(),
                reliable: true,
            });
        }

        fn set_remote_server(&self, peer: Option<u64>) {
            self.status.borrow_mut().remote_dedicated_server_steam_id =
                peer.map(|id| id.to_string());
        }

        fn set_status_available(&self, available: bool, service_ready: bool) {
            let mut status = self.status.borrow_mut();
            status.available = available;
            status.service_ready = service_ready;
        }
    }

    impl Transport for FakeTransport {
        fn status(&self) -> shroudforge_steam_networking::Status {
            self.status_reads.set(self.status_reads.get() + 1);
            let status = self.status.borrow().clone();
            assert!(!self.panic_status.get(), "test transport status panic");
            if let Some(callback) = self.on_status.borrow_mut().as_mut() {
                callback();
            }
            status
        }

        fn send(&self, peer: u64, payload: &[u8], channel: i32) -> Result<(), String> {
            self.sent
                .borrow_mut()
                .push((peer, payload.to_vec(), channel));
            if let Some(callback) = self.on_send.borrow_mut().as_mut() {
                callback();
            }
            Ok(())
        }

        fn receive(
            &self,
            channel: i32,
        ) -> Result<Option<shroudforge_steam_networking::Message>, String> {
            self.receive_channels.borrow_mut().push(channel);
            if let Some(callback) = self.on_receive.borrow_mut().as_mut() {
                callback();
            }
            if self.receive_error.get() {
                return Err("test transport receive error".into());
            }
            Ok(self.messages.borrow_mut().pop_front().flatten())
        }

        fn is_authorized_server_peer(&self, peer: u64) -> bool {
            self.authorized_reads.set(self.authorized_reads.get() + 1);
            let authorized = self.authorized.borrow().contains(&peer);
            if let Some(callback) = self.on_authorized.borrow_mut().as_mut() {
                callback();
            }
            authorized
        }
    }

    fn status(
        local: u64,
        remote_server: Option<u64>,
        available: bool,
        service_ready: bool,
        is_server: bool,
    ) -> shroudforge_steam_networking::Status {
        shroudforge_steam_networking::Status {
            available,
            local_steam_id: available.then(|| local.to_string()),
            local_dedicated_server_steam_id: is_server.then(|| local.to_string()),
            remote_dedicated_server_steam_id: remote_server.map(|id| id.to_string()),
            service_ready,
        }
    }

    fn receive_call(
        context: &NativePluginHostContext,
        transport: &impl Transport,
        output: &mut [u8],
        metadata: &mut NetworkMessageV1,
    ) -> i32 {
        receive_inner(
            context,
            if output.is_empty() {
                std::ptr::null_mut()
            } else {
                output.as_mut_ptr()
            },
            output.len() as u32,
            size_of::<NetworkMessageV1>() as u32,
            metadata,
            transport,
        )
    }

    fn sentinel_metadata() -> NetworkMessageV1 {
        NetworkMessageV1 {
            size: 0xA5A5_A5A5,
            version: 0x5A5A_5A5A,
            peer_steam_id: 0xA55A_A55A_A55A_A55A,
            payload_bytes: 0xDEAD_BEEF,
            flags: 0xFACE_CAFE,
            reply_token: 0x1122_3344_5566_7788,
        }
    }

    fn sentinel_status() -> NetworkStatusV1 {
        NetworkStatusV1 {
            size: 0xA5A5_A5A5,
            version: 0x5A5A_5A5A,
            process_target: 0xDEAD_BEEF,
            available: 0xFACE_CAFE,
            service_ready: 0x1122_3344,
            reserved: 0x5566_7788,
            local_steam_id: 0xA55A_A55A_A55A_A55A,
            connected_dedicated_server_steam_id: 0x1234_5678_9ABC_DEF0,
        }
    }

    fn assert_sentinel_metadata(metadata: &NetworkMessageV1) {
        assert_eq!(metadata.size, 0xA5A5_A5A5);
        assert_eq!(metadata.version, 0x5A5A_5A5A);
        assert_eq!(metadata.peer_steam_id, 0xA55A_A55A_A55A_A55A);
        assert_eq!(metadata.payload_bytes, 0xDEAD_BEEF);
        assert_eq!(metadata.flags, 0xFACE_CAFE);
        assert_eq!(metadata.reply_token, 0x1122_3344_5566_7788);
    }

    fn assert_sentinel_status(status: &NetworkStatusV1) {
        assert_eq!(status.size, 0xA5A5_A5A5);
        assert_eq!(status.version, 0x5A5A_5A5A);
        assert_eq!(status.process_target, 0xDEAD_BEEF);
        assert_eq!(status.available, 0xFACE_CAFE);
        assert_eq!(status.service_ready, 0x1122_3344);
        assert_eq!(status.reserved, 0x5566_7788);
        assert_eq!(status.local_steam_id, 0xA55A_A55A_A55A_A55A);
        assert_eq!(
            status.connected_dedicated_server_steam_id,
            0x1234_5678_9ABC_DEF0
        );
    }

    #[test]
    fn query_is_owner_bound_and_status_reports_transport_readiness_without_blocking_query() {
        let mut client = Fixture::new("network-client", false, true, true, 7);
        let host = client.context_mut().host_table();
        let host_query = host.query_service.unwrap();
        let service_name = SERVICE.as_bytes();
        let mut service = std::mem::MaybeUninit::<NetworkServiceV1>::uninit();
        assert_eq!(
            unsafe {
                host_query(
                    host.package_context,
                    service_name.as_ptr(),
                    service_name.len() as u32,
                    1,
                    size_of::<NetworkServiceV1>() as u32,
                    service.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_OK
        );
        let service = unsafe { service.assume_init() };
        assert_eq!(service.version, 1);
        assert_eq!(service.context, host.package_context);
        assert_eq!(service.max_payload_bytes, MAX_PAYLOAD_BYTES as u32);
        assert!(service.read_status.is_some());
        assert!(service.send_to_server.is_some());
        assert!(service.receive.is_some());
        assert!(service.reply.is_some());

        let mut too_small = 0xA5A5_A5A5u32;
        assert_eq!(
            unsafe {
                host_query(
                    host.package_context,
                    service_name.as_ptr(),
                    service_name.len() as u32,
                    1,
                    size_of::<NetworkServiceV1>() as u32 - 1,
                    (&mut too_small as *mut u32).cast(),
                )
            },
            NATIVE_STATUS_BUFFER_TOO_SMALL
        );
        assert_eq!(too_small, 0xA5A5_A5A5);

        let mut server = Fixture::new("network-server", true, true, true, 8);
        let server_host = server.context_mut().host_table();
        let mut server_service = std::mem::MaybeUninit::<NetworkServiceV1>::uninit();
        assert_eq!(
            unsafe {
                server_host.query_service.unwrap()(
                    server_host.package_context,
                    service_name.as_ptr(),
                    service_name.len() as u32,
                    1,
                    size_of::<NetworkServiceV1>() as u32,
                    server_service.as_mut_ptr().cast(),
                )
            },
            NATIVE_STATUS_OK
        );
        assert_eq!(unsafe { server_service.assume_init() }.version, 1);

        let transport = FakeTransport::client(Some(SERVER_ID));
        transport.set_status_available(true, false);
        let mut output = NetworkStatusV1 {
            size: 0xAAAA_AAAA,
            version: 99,
            process_target: 99,
            available: 99,
            service_ready: 99,
            reserved: 99,
            local_steam_id: 99,
            connected_dedicated_server_steam_id: 99,
        };
        assert_eq!(
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                &mut output,
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        assert_eq!(output.size, size_of::<NetworkStatusV1>() as u32);
        assert_eq!(output.version, 1);
        assert_eq!(output.process_target, NATIVE_TARGET_CLIENT);
        assert_eq!(output.available, 1);
        assert_eq!(output.service_ready, 0);
        assert_eq!(output.local_steam_id, CLIENT_ID);
        assert_eq!(output.connected_dedicated_server_steam_id, 0);

        let disabled = Fixture::new("network-disabled", false, true, false, 8);
        let transport = FakeTransport::client(Some(SERVER_ID));
        let mut output = std::mem::MaybeUninit::<NetworkStatusV1>::uninit();
        assert_eq!(
            read_status_inner(
                disabled.context(),
                size_of::<NetworkStatusV1>() as u32,
                output.as_mut_ptr(),
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        let output = unsafe { output.assume_init() };
        assert_eq!(output.available, 0);
        assert_eq!(output.service_ready, 0);
        assert_eq!(output.local_steam_id, 0);
        assert_eq!(output.connected_dedicated_server_steam_id, 0);
        assert_eq!(transport.status_reads.get(), 0);

        let mut no_runtime = Fixture::new("network-no-runtime", false, false, true, 9);
        let no_runtime_host = no_runtime.context_mut().host_table();
        let mut output = 0xA5A5_A5A5u32;
        assert_eq!(
            unsafe {
                no_runtime_host.query_service.unwrap()(
                    no_runtime_host.package_context,
                    service_name.as_ptr(),
                    service_name.len() as u32,
                    1,
                    size_of::<NetworkServiceV1>() as u32,
                    (&mut output as *mut u32).cast(),
                )
            },
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(output, 0xA5A5_A5A5);
    }

    #[test]
    fn network_policy_uses_load_snapshot_and_refresh_invalidates_old_replies() {
        let client = Fixture::new("policy-transition", false, true, true, 17);
        assert_eq!(
            client.context().network_policy.get(),
            NativeNetworkPolicy::Enabled
        );
        let transport = FakeTransport::client(Some(SERVER_ID));
        transport.message(
            SERVER_ID,
            "policy-transition",
            "policy-transition",
            "request",
        );
        let mut receive_output = [0u8; 32];
        let mut receive_metadata = sentinel_metadata();
        assert_eq!(
            receive_call(
                client.context(),
                &transport,
                &mut receive_output,
                &mut receive_metadata,
            ),
            NATIVE_STATUS_OK
        );
        let stale_reply_token = receive_metadata.reply_token;

        client.set_network_enabled(false);
        let before_refresh_status_reads = transport.status_reads.get();
        let before_refresh_payload = b"still enabled until refresh";
        assert_eq!(
            send_to_server_inner(
                client.context(),
                before_refresh_payload.as_ptr(),
                before_refresh_payload.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        assert_eq!(transport.sent.borrow().len(), 1);
        assert_eq!(
            transport.status_reads.get(),
            before_refresh_status_reads + 1,
            "callbacks use the owner policy snapshot rather than rereading loader settings"
        );

        client.refresh_native_network_policy();
        assert_eq!(
            client.context().network_policy.get(),
            NativeNetworkPolicy::Disabled
        );
        let mut disabled_status = std::mem::MaybeUninit::<NetworkStatusV1>::uninit();
        let status_reads_after_refresh = transport.status_reads.get();
        assert_eq!(
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                disabled_status.as_mut_ptr(),
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        let disabled_status = unsafe { disabled_status.assume_init() };
        assert_eq!(disabled_status.available, 0);
        assert_eq!(disabled_status.service_ready, 0);
        assert_eq!(transport.status_reads.get(), status_reads_after_refresh);

        let after_disable = b"denied";
        assert_eq!(
            send_to_server_inner(
                client.context(),
                after_disable.as_ptr(),
                after_disable.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        let receive_calls_after_disable = transport.receive_channels.borrow().len();
        assert_eq!(
            receive_call(
                client.context(),
                &transport,
                &mut receive_output,
                &mut receive_metadata,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(
            transport.receive_channels.borrow().len(),
            receive_calls_after_disable
        );
        assert_eq!(
            reply_inner(
                client.context(),
                stale_reply_token,
                after_disable.as_ptr(),
                after_disable.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(transport.sent.borrow().len(), 1);

        client.write_invalid_loader_config();
        client.refresh_native_network_policy();
        assert_eq!(
            client.context().network_policy.get(),
            NativeNetworkPolicy::Unavailable
        );
        let mut unavailable_status = sentinel_status();
        assert_eq!(
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                &mut unavailable_status,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_sentinel_status(&unavailable_status);
        assert_eq!(
            send_to_server_inner(
                client.context(),
                after_disable.as_ptr(),
                after_disable.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        let receive_calls_before_error = transport.receive_channels.borrow().len();
        assert_eq!(
            receive_call(
                client.context(),
                &transport,
                &mut receive_output,
                &mut receive_metadata,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(
            transport.receive_channels.borrow().len(),
            receive_calls_before_error
        );

        client.set_network_enabled(true);
        assert_eq!(
            send_to_server_inner(
                client.context(),
                after_disable.as_ptr(),
                after_disable.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE,
            "a successful disk write is not visible until the scheduled refresh"
        );
        client.refresh_native_network_policy();
        assert_eq!(
            client.context().network_policy.get(),
            NativeNetworkPolicy::Enabled
        );
        assert_eq!(
            reply_inner(
                client.context(),
                stale_reply_token,
                after_disable.as_ptr(),
                after_disable.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE,
            "reenabling networking must not revive an old reply token"
        );
        assert_eq!(
            send_to_server_inner(
                client.context(),
                after_disable.as_ptr(),
                after_disable.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        assert_eq!(transport.sent.borrow().len(), 2);
    }

    #[test]
    fn loader_read_failure_at_context_creation_starts_network_unavailable() {
        let mut client = Fixture::new("policy-init-error", false, true, true, 18);
        client.recreate_context_after_loader_config(b"{invalid");
        assert_eq!(
            client.context().network_policy.get(),
            NativeNetworkPolicy::Unavailable
        );

        let transport = FakeTransport::client(Some(SERVER_ID));
        let mut output = sentinel_status();
        assert_eq!(
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                &mut output,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_sentinel_status(&output);
        assert_eq!(transport.status_reads.get(), 0);
    }

    #[test]
    fn explicit_loader_disable_at_context_creation_starts_network_disabled() {
        let mut client = Fixture::new("policy-init-disabled", false, true, true, 19);
        client.recreate_context_after_loader_config(
            br#"{"schemaVersion":1,"modules":{"network":{"enabled":false}}}"#,
        );
        assert_eq!(
            client.context().network_policy.get(),
            NativeNetworkPolicy::Disabled
        );

        let transport = FakeTransport::client(Some(SERVER_ID));
        let mut status = std::mem::MaybeUninit::<NetworkStatusV1>::uninit();
        assert_eq!(
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                status.as_mut_ptr(),
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        let status = unsafe { status.assume_init() };
        assert_eq!(status.available, 0);
        assert_eq!(status.service_ready, 0);
        assert_eq!(transport.status_reads.get(), 0);

        let payload = b"disabled at load";
        assert_eq!(
            send_to_server_inner(
                client.context(),
                payload.as_ptr(),
                payload.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(
            receive_call(
                client.context(),
                &transport,
                &mut [0u8; 32],
                &mut sentinel_metadata(),
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert!(transport.sent.borrow().is_empty());
        assert!(transport.receive_channels.borrow().is_empty());
    }

    #[test]
    fn send_is_client_only_owner_bound_utf8_bounded_and_never_uses_caller_routing() {
        let client = Fixture::new("owner-client", false, true, true, 12);
        let transport = FakeTransport::client(Some(SERVER_ID));
        let bytes = b"hello";
        assert_eq!(
            send_to_server_inner(
                client.context(),
                bytes.as_ptr(),
                bytes.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        let sent = transport.sent.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, SERVER_ID);
        assert_eq!(
            sent[0].2,
            shroudforge_steam_networking::mod_messages::mod_channel("owner-client")
        );
        let envelope = shroudforge_steam_networking::mod_messages::decode_mod_message(
            &sent[0].1,
            "owner-client",
        )
        .unwrap();
        assert_eq!(envelope.from_mod, "owner-client");
        assert_eq!(envelope.payload, "hello");
        drop(sent);

        let server = Fixture::new("owner-server", true, true, true, 13);
        assert_eq!(
            send_to_server_inner(
                server.context(),
                bytes.as_ptr(),
                bytes.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(transport.sent.borrow().len(), 1);

        let invalid_utf8 = [0xffu8];
        assert_eq!(
            send_to_server_inner(
                client.context(),
                invalid_utf8.as_ptr(),
                invalid_utf8.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_INVALID_ARGUMENT
        );
        let oversized = vec![b'x'; MAX_PAYLOAD_BYTES + 1];
        assert_eq!(
            send_to_server_inner(
                client.context(),
                oversized.as_ptr(),
                oversized.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_TOO_LARGE
        );
        assert_eq!(transport.sent.borrow().len(), 1);

        let maximum_controls = "\u{0001}".repeat(MAX_PAYLOAD_BYTES);
        let final_envelope = shroudforge_steam_networking::mod_messages::encode_mod_message(
            "owner-client",
            "owner-client",
            &maximum_controls,
        )
        .unwrap();
        assert!(final_envelope.len() <= shroudforge_steam_networking::MAX_MESSAGE_BYTES);

        let disabled = Fixture::new("owner-disabled", false, true, false, 14);
        let before_status = transport.status_reads.get();
        assert_eq!(
            send_to_server_inner(
                disabled.context(),
                bytes.as_ptr(),
                bytes.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(transport.sent.borrow().len(), 1);
        assert_eq!(transport.status_reads.get(), before_status);

        let mut denied_output = [0xA5; 16];
        let mut denied_metadata = sentinel_metadata();
        let receive_calls = transport.receive_channels.borrow().len();
        assert_eq!(
            receive_call(
                disabled.context(),
                &transport,
                &mut denied_output,
                &mut denied_metadata,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(transport.receive_channels.borrow().len(), receive_calls);
        assert_eq!(transport.status_reads.get(), before_status);
        assert!(denied_output.iter().all(|byte| *byte == 0xA5));
        assert_sentinel_metadata(&denied_metadata);
    }

    #[test]
    fn receive_uses_one_frame_filters_sender_and_expires_too_small_or_old_replies() {
        let client = Fixture::new("same-owner", false, true, true, 21);
        let transport = FakeTransport::client(Some(SERVER_ID));
        transport.message(SPOOFED_PEER, "same-owner", "same-owner", "spoofed peer");
        transport.message(SERVER_ID, "other-mod", "same-owner", "spoofed sender");
        transport.message(SERVER_ID, "same-owner", "same-owner", "accepted");

        let mut metadata = sentinel_metadata();
        let mut output = [0xA5; 64];
        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_NOT_FOUND
        );
        assert_sentinel_metadata(&metadata);
        assert!(output.iter().all(|byte| *byte == 0xA5));
        assert_eq!(transport.receive_channels.borrow().len(), 1);

        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_NOT_FOUND
        );
        assert_sentinel_metadata(&metadata);
        assert_eq!(transport.receive_channels.borrow().len(), 2);

        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_OK
        );
        assert_eq!(&output[..8], b"accepted");
        assert_eq!(metadata.peer_steam_id, SERVER_ID);
        assert_eq!(metadata.payload_bytes, 8);
        assert_ne!(metadata.reply_token, 0);
        assert_eq!(metadata.flags, 1);
        let token = metadata.reply_token;

        let mut empty = [];
        let mut next_metadata = sentinel_metadata();
        assert_eq!(
            receive_call(client.context(), &transport, &mut empty, &mut next_metadata),
            NATIVE_STATUS_NOT_FOUND
        );
        let reply = b"reply";
        assert_eq!(
            reply_inner(
                client.context(),
                token,
                reply.as_ptr(),
                reply.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert!(transport.sent.borrow().is_empty());

        transport.message(SERVER_ID, "same-owner", "same-owner", "buffer too small");
        let mut tiny = [0xA5; 2];
        let mut small_metadata = sentinel_metadata();
        assert_eq!(
            receive_call(client.context(), &transport, &mut tiny, &mut small_metadata),
            NATIVE_STATUS_BUFFER_TOO_SMALL
        );
        assert_eq!(small_metadata.payload_bytes, 16);
        assert_eq!(small_metadata.reply_token, 0);
        assert_eq!(tiny, [0xA5; 2]);
        let prior_calls = transport.receive_channels.borrow().len();
        let mut no_message = [];
        let mut no_message_metadata = sentinel_metadata();
        assert_eq!(
            receive_call(
                client.context(),
                &transport,
                &mut no_message,
                &mut no_message_metadata,
            ),
            NATIVE_STATUS_NOT_FOUND
        );
        assert_eq!(transport.receive_channels.borrow().len(), prior_calls + 1);
        assert_sentinel_metadata(&no_message_metadata);

        let other_owner = Fixture::new("other-owner", false, true, true, 22);
        assert_eq!(
            reply_inner(
                other_owner.context(),
                token,
                reply.as_ptr(),
                reply.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
    }

    #[test]
    fn server_requires_current_authorized_peer_and_rechecks_before_reply() {
        let server = Fixture::new("same-owner", true, true, true, 31);
        let transport = FakeTransport::server();
        transport.message(SPOOFED_PEER, "same-owner", "same-owner", "unauthorized");
        transport.message(AUTHORIZED_PEER, "same-owner", "same-owner", "authorized");
        transport.authorized.borrow_mut().insert(AUTHORIZED_PEER);
        let mut output = [0xA5; 32];
        let mut metadata = sentinel_metadata();
        assert_eq!(
            receive_call(server.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_NOT_FOUND
        );
        assert_sentinel_metadata(&metadata);
        assert_eq!(transport.receive_channels.borrow().len(), 1);
        assert_eq!(
            receive_call(server.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_OK
        );
        assert_eq!(metadata.peer_steam_id, AUTHORIZED_PEER);
        let token = metadata.reply_token;

        transport.authorized.borrow_mut().remove(&AUTHORIZED_PEER);
        let reply = b"server reply";
        assert_eq!(
            reply_inner(
                server.context(),
                token,
                reply.as_ptr(),
                reply.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert!(transport.sent.borrow().is_empty());

        transport.authorized.borrow_mut().insert(AUTHORIZED_PEER);
        transport.message(AUTHORIZED_PEER, "same-owner", "same-owner", "again");
        assert_eq!(
            receive_call(server.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_OK
        );
        let token = metadata.reply_token;
        assert_eq!(
            reply_inner(
                server.context(),
                token,
                reply.as_ptr(),
                reply.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        let sent = transport.sent.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, AUTHORIZED_PEER);
        let envelope = shroudforge_steam_networking::mod_messages::decode_mod_message(
            &sent[0].1,
            "same-owner",
        )
        .unwrap();
        assert_eq!(envelope.from_mod, "same-owner");
        assert_eq!(envelope.payload, "server reply");
    }

    #[test]
    fn disconnect_status_change_revoke_and_wrong_thread_close_calls_without_transport_work() {
        let mut client = Fixture::new("lifecycle-owner", false, true, true, 41);
        let transport = FakeTransport::client(Some(SERVER_ID));
        transport.message(SERVER_ID, "lifecycle-owner", "lifecycle-owner", "hello");
        let mut output = [0u8; 32];
        let mut metadata = sentinel_metadata();
        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_OK
        );
        let token = metadata.reply_token;

        transport.set_remote_server(Some(OTHER_SERVER_ID));
        let mut status_output = std::mem::MaybeUninit::<NetworkStatusV1>::uninit();
        assert_eq!(
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                status_output.as_mut_ptr(),
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        assert_eq!(
            unsafe { status_output.assume_init() }.connected_dedicated_server_steam_id,
            OTHER_SERVER_ID
        );
        let reply = b"stale";
        assert_eq!(
            reply_inner(
                client.context(),
                token,
                reply.as_ptr(),
                reply.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert!(transport.sent.borrow().is_empty());

        transport.set_remote_server(None);
        let calls_before_disconnect = transport.receive_channels.borrow().len();
        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(
            transport.receive_channels.borrow().len(),
            calls_before_disconnect
        );

        transport.set_remote_server(Some(SERVER_ID));
        let other_thread = std::thread::spawn(|| std::thread::current().id())
            .join()
            .unwrap();
        client.context_mut().owner_thread = other_thread;
        let status_reads = transport.status_reads.get();
        assert_eq!(
            send_to_server_inner(
                client.context(),
                reply.as_ptr(),
                reply.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_WRONG_THREAD
        );
        assert_eq!(transport.status_reads.get(), status_reads);
        assert!(transport.sent.borrow().is_empty());

        client.context_mut().owner_thread = std::thread::current().id();
        client.context().network.revoke();
        client.context().services_revoked.set(true);
        let receive_calls = transport.receive_channels.borrow().len();
        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(transport.receive_channels.borrow().len(), receive_calls);
        assert_eq!(transport.sent.borrow().len(), 0);
    }

    #[test]
    fn oversized_transport_and_payload_frames_are_consumed_without_output_writes() {
        let client = Fixture::new("oversize-owner", false, true, true, 51);
        let transport = FakeTransport::client(Some(SERVER_ID));
        transport.enqueue(shroudforge_steam_networking::Message {
            peer_steam_id: SERVER_ID.to_string(),
            payload: vec![b'x'; shroudforge_steam_networking::MAX_MESSAGE_BYTES + 1],
            reliable: true,
        });
        let mut output = [0xA5; 4];
        let mut metadata = sentinel_metadata();
        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_TOO_LARGE
        );
        assert_eq!(output, [0xA5; 4]);
        assert_sentinel_metadata(&metadata);
        assert_eq!(transport.receive_channels.borrow().len(), 1);

        let oversized_payload = "x".repeat(MAX_PAYLOAD_BYTES + 1);
        transport.message(
            SERVER_ID,
            "oversize-owner",
            "oversize-owner",
            &oversized_payload,
        );
        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_TOO_LARGE
        );
        assert_eq!(output, [0xA5; 4]);
        assert_sentinel_metadata(&metadata);
        assert_eq!(transport.receive_channels.borrow().len(), 2);
    }

    #[test]
    fn same_owner_reentry_is_busy_drain_waits_and_other_owner_can_run() {
        let drain_observer = Arc::new(DrainObserver::default());
        let mut owner = Fixture::with_services(
            "network-owner",
            false,
            true,
            true,
            81,
            drain_observer.clone(),
        );
        let other_owner = Fixture::new("network-other-owner", false, true, true, 82);
        let transport = FakeTransport::client(Some(SERVER_ID));
        let other_transport = FakeTransport::client(Some(SERVER_ID));
        transport.message(SERVER_ID, "network-owner", "network-owner", "seed");
        let mut seed_output = [0u8; 8];
        let mut seed_metadata = sentinel_metadata();
        assert_eq!(
            receive_call(
                owner.context(),
                &transport,
                &mut seed_output,
                &mut seed_metadata,
            ),
            NATIVE_STATUS_OK
        );
        let reply_token = seed_metadata.reply_token;

        let host = owner.context_mut().host_table();
        let package_context = host.package_context as usize;
        let generation = host.package_generation;
        let host_query = host.query_service.unwrap();
        let host_drain = host.drain_services.unwrap();
        let owner_context = owner.context() as *const NativePluginHostContext as usize;
        let other_context = other_owner.context() as *const NativePluginHostContext as usize;
        let transport_ptr = &transport as *const FakeTransport as usize;
        let other_transport_ptr = &other_transport as *const FakeTransport as usize;
        let service_name = SERVICE.as_bytes().to_vec();
        transport.on_status(move || unsafe {
            let owner_context = &*(owner_context as *const NativePluginHostContext);
            let transport = &*(transport_ptr as *const FakeTransport);

            let mut service_table = NetworkServiceV1 {
                size: 0xA5A5_A5A5,
                version: 0x5A5A_5A5A,
                context: 0xA55A_A55Ausize as *mut c_void,
                max_payload_bytes: 0xDEAD_BEEF,
                reserved: 0xFACE_CAFE,
                read_status: None,
                send_to_server: None,
                receive: None,
                reply: None,
            };
            assert_eq!(
                host_query(
                    package_context as *mut c_void,
                    service_name.as_ptr(),
                    service_name.len() as u32,
                    1,
                    size_of::<NetworkServiceV1>() as u32,
                    (&mut service_table as *mut NetworkServiceV1).cast(),
                ),
                NATIVE_STATUS_BUSY
            );
            assert_eq!(service_table.size, 0xA5A5_A5A5);
            assert_eq!(service_table.version, 0x5A5A_5A5A);
            assert_eq!(service_table.context, 0xA55A_A55Ausize as *mut c_void);
            assert_eq!(service_table.max_payload_bytes, 0xDEAD_BEEF);
            assert_eq!(service_table.reserved, 0xFACE_CAFE);
            assert!(service_table.read_status.is_none());
            assert!(service_table.send_to_server.is_none());
            assert!(service_table.receive.is_none());
            assert!(service_table.reply.is_none());

            let mut nested_status = sentinel_status();
            assert_eq!(
                read_status_inner(
                    owner_context,
                    size_of::<NetworkStatusV1>() as u32,
                    &mut nested_status,
                    transport,
                ),
                NATIVE_STATUS_BUSY
            );
            assert_sentinel_status(&nested_status);

            let payload = b"nested";
            assert_eq!(
                send_to_server_inner(
                    owner_context,
                    payload.as_ptr(),
                    payload.len() as u32,
                    transport,
                ),
                NATIVE_STATUS_BUSY
            );

            let mut nested_output = [0xA5; 8];
            let mut nested_metadata = sentinel_metadata();
            assert_eq!(
                receive_call(
                    owner_context,
                    transport,
                    &mut nested_output,
                    &mut nested_metadata,
                ),
                NATIVE_STATUS_BUSY
            );
            assert_eq!(nested_output, [0xA5; 8]);
            assert_sentinel_metadata(&nested_metadata);

            assert_eq!(
                reply_inner(
                    owner_context,
                    reply_token,
                    payload.as_ptr(),
                    payload.len() as u32,
                    transport,
                ),
                NATIVE_STATUS_BUSY
            );
            assert!(owner_context.network.has_active_call());
            assert_eq!(
                host_drain(
                    package_context as *mut c_void,
                    generation,
                    NATIVE_DRAIN_TIMEOUT_MS,
                ),
                NATIVE_STATUS_BUSY
            );

            let other_context = &*(other_context as *const NativePluginHostContext);
            let other_transport = &*(other_transport_ptr as *const FakeTransport);
            let mut other_status = std::mem::MaybeUninit::<NetworkStatusV1>::uninit();
            assert_eq!(
                read_status_inner(
                    other_context,
                    size_of::<NetworkStatusV1>() as u32,
                    other_status.as_mut_ptr(),
                    other_transport,
                ),
                NATIVE_STATUS_OK
            );
            assert!(other_status.assume_init().available != 0);
        });

        let mut outer_status = sentinel_status();
        assert_eq!(
            read_status_inner(
                owner.context(),
                size_of::<NetworkStatusV1>() as u32,
                &mut outer_status,
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        assert_eq!(outer_status.available, 1);
        assert!(!owner.context().network.has_active_call());
        assert!(!other_owner.context().network.has_active_call());
        assert_eq!(transport.status_reads.get(), 2);
        assert_eq!(transport.receive_channels.borrow().len(), 1);
        assert!(transport.sent.borrow().is_empty());
        assert_eq!(other_transport.status_reads.get(), 1);
        assert_eq!(drain_observer.drains.load(Ordering::SeqCst), 0);
        assert_eq!(
            unsafe { host_drain(host.package_context, generation, NATIVE_DRAIN_TIMEOUT_MS,) },
            NATIVE_STATUS_OK
        );
        assert_eq!(drain_observer.drains.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn revoke_during_transport_status_blocks_output_and_subsequent_work() {
        let mut client = Fixture::new("revoke-status", false, true, true, 83);
        let host = client.context_mut().host_table();
        let revoke = host.revoke_services.unwrap();
        let package_context = host.package_context as usize;
        let generation = host.package_generation;
        let transport = FakeTransport::client(Some(SERVER_ID));
        transport.on_status(move || unsafe {
            assert_eq!(
                revoke(package_context as *mut c_void, generation),
                NATIVE_STATUS_OK
            );
        });

        let mut output = sentinel_status();
        assert_eq!(
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                &mut output,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_sentinel_status(&output);
        assert!(!client.context().network.has_active_call());
        let status_reads = transport.status_reads.get();
        let payload = b"after revoke";
        assert_eq!(
            send_to_server_inner(
                client.context(),
                payload.as_ptr(),
                payload.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(transport.status_reads.get(), status_reads);
        assert!(transport.sent.borrow().is_empty());
    }

    #[test]
    fn policy_disable_reenable_during_external_calls_invalidates_the_outer_lease() {
        let client = Fixture::new("policy-transition-send", false, true, true, 84);
        let context_address = client.context() as *const NativePluginHostContext as usize;
        let transport = FakeTransport::client(Some(SERVER_ID));
        transport.on_status(move || unsafe {
            let context = &*(context_address as *const NativePluginHostContext);
            assert!(
                context
                    .update_network_policy(NativeNetworkPolicy::Disabled)
                    .unwrap()
            );
            assert!(
                context
                    .update_network_policy(NativeNetworkPolicy::Enabled)
                    .unwrap()
            );
        });
        let payload = b"transitioned";
        assert_eq!(
            send_to_server_inner(
                client.context(),
                payload.as_ptr(),
                payload.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert!(transport.sent.borrow().is_empty());
        assert_eq!(transport.status_reads.get(), 1);
        assert!(!client.context().network.has_active_call());

        *transport.on_status.borrow_mut() = None;
        assert_eq!(
            send_to_server_inner(
                client.context(),
                payload.as_ptr(),
                payload.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        assert_eq!(transport.sent.borrow().len(), 1);

        let receiving = Fixture::new("policy-transition-receive", false, true, true, 85);
        let receive_context = receiving.context() as *const NativePluginHostContext as usize;
        let receive_transport = FakeTransport::client(Some(SERVER_ID));
        receive_transport.message(
            SERVER_ID,
            "policy-transition-receive",
            "policy-transition-receive",
            "message",
        );
        receive_transport.on_receive(move || unsafe {
            let context = &*(receive_context as *const NativePluginHostContext);
            assert!(
                context
                    .update_network_policy(NativeNetworkPolicy::Disabled)
                    .unwrap()
            );
            assert!(
                context
                    .update_network_policy(NativeNetworkPolicy::Enabled)
                    .unwrap()
            );
        });
        let mut receive_output = [0xA5; 16];
        let mut receive_metadata = sentinel_metadata();
        assert_eq!(
            receive_call(
                receiving.context(),
                &receive_transport,
                &mut receive_output,
                &mut receive_metadata,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(receive_output, [0xA5; 16]);
        assert_sentinel_metadata(&receive_metadata);
        assert!(!receiving.context().network.has_active_call());
    }

    #[test]
    fn revocation_during_server_authorization_does_not_publish_a_reply_route() {
        let mut server = Fixture::new("revoke-authorized", true, true, true, 86);
        let host = server.context_mut().host_table();
        let revoke = host.revoke_services.unwrap();
        let package_context = host.package_context as usize;
        let generation = host.package_generation;
        let transport = FakeTransport::server();
        transport.authorized.borrow_mut().insert(AUTHORIZED_PEER);
        transport.message(
            AUTHORIZED_PEER,
            "revoke-authorized",
            "revoke-authorized",
            "message",
        );
        transport.on_authorized(move || unsafe {
            assert_eq!(
                revoke(package_context as *mut c_void, generation),
                NATIVE_STATUS_OK
            );
        });

        let mut output = [0xA5; 16];
        let mut metadata = sentinel_metadata();
        assert_eq!(
            receive_call(server.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(output, [0xA5; 16]);
        assert_sentinel_metadata(&metadata);
        assert_eq!(transport.authorized_reads.get(), 1);
        assert!(
            server
                .context()
                .network
                .reply_route_for_any()
                .unwrap()
                .is_none()
        );
        assert!(!server.context().network.has_active_call());
    }

    #[test]
    fn send_revocation_reports_admission_state_without_claiming_transport_undo() {
        let mut client = Fixture::new("revoke-send", false, true, true, 87);
        let transport = FakeTransport::client(Some(SERVER_ID));
        transport.message(SERVER_ID, "revoke-send", "revoke-send", "seed");
        let mut output = [0u8; 8];
        let mut metadata = sentinel_metadata();
        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_OK
        );
        let token = metadata.reply_token;
        let host = client.context_mut().host_table();
        let revoke = host.revoke_services.unwrap();
        let package_context = host.package_context as usize;
        let generation = host.package_generation;
        transport.on_send(move || unsafe {
            assert_eq!(
                revoke(package_context as *mut c_void, generation),
                NATIVE_STATUS_OK
            );
        });

        let payload = b"reply";
        assert_eq!(
            reply_inner(
                client.context(),
                token,
                payload.as_ptr(),
                payload.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        // The transport call already happened; the status only reports that
        // owner admission was revoked before this callback returned.
        assert_eq!(transport.sent.borrow().len(), 1);
        assert!(!client.context().network.has_active_call());
        assert_eq!(
            reply_inner(
                client.context(),
                token,
                payload.as_ptr(),
                payload.len() as u32,
                &transport,
            ),
            NATIVE_STATUS_UNAVAILABLE
        );
        assert_eq!(transport.sent.borrow().len(), 1);
    }

    #[test]
    fn network_lease_releases_after_normal_error_and_unwind() {
        let client = Fixture::new("lease-release", false, true, true, 88);
        let transport = FakeTransport::client(Some(SERVER_ID));
        let mut status = sentinel_status();
        assert_eq!(
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                &mut status,
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        assert!(!client.context().network.has_active_call());

        transport.receive_error.set(true);
        let mut output = [0xA5; 8];
        let mut metadata = sentinel_metadata();
        assert_eq!(
            receive_call(client.context(), &transport, &mut output, &mut metadata),
            NATIVE_STATUS_FAILED
        );
        assert_eq!(output, [0xA5; 8]);
        assert_sentinel_metadata(&metadata);
        assert!(!client.context().network.has_active_call());

        transport.receive_error.set(false);
        transport.panic_status.set(true);
        let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                &mut status,
                &transport,
            )
        }));
        assert!(panic.is_err());
        assert!(!client.context().network.has_active_call());
        transport.panic_status.set(false);
        assert_eq!(
            read_status_inner(
                client.context(),
                size_of::<NetworkStatusV1>() as u32,
                &mut status,
                &transport,
            ),
            NATIVE_STATUS_OK
        );
        assert!(!client.context().network.has_active_call());
    }

    #[test]
    fn reply_token_saturation_and_policy_invalidation_never_reuse_old_tokens() {
        let state = OwnerState::default();
        let identity = PeerIdentity {
            local_steam_id: Some(CLIENT_ID),
            connected_server_steam_id: Some(SERVER_ID),
        };
        let old_token = state.issue_reply(89, SERVER_ID, identity).unwrap();
        assert_eq!(old_token, 1);
        state.next_token.set(u64::MAX);
        assert_eq!(
            state.issue_reply(89, SERVER_ID, identity),
            Err(NATIVE_STATUS_UNAVAILABLE)
        );
        assert_eq!(
            state.reply_route(old_token).unwrap().unwrap().token,
            old_token
        );

        state.transition_policy().unwrap();
        assert!(state.reply_route(old_token).unwrap().is_none());
        assert_eq!(state.next_token.get(), u64::MAX);
        assert_eq!(
            state.issue_reply(89, SERVER_ID, identity),
            Err(NATIVE_STATUS_UNAVAILABLE)
        );
        assert!(state.reply_route(old_token).unwrap().is_none());
        state.revoke();
        assert_eq!(state.next_token.get(), u64::MAX);
    }
}
