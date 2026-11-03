//! Host-owned logical sessions and bounded payload retention. No transport calls.
use shroudforge_steam_networking::mod_network_frame::{self as wire, Frame, Session};
use std::{collections::BTreeMap, sync::Mutex};
pub(super) const MAX_PEERS: usize = 64;
pub(super) const MAX_BUFFERS: usize = 4;
pub(super) const OWNER_BYTES: usize = 2 * 1024 * 1024;
pub(super) const GLOBAL_BYTES: usize = 16 * 1024 * 1024;
static RETAINED: Mutex<usize> = Mutex::new(0);

struct Payload {
    bytes: Vec<u8>,
    seen: u16,
    count: usize,
    born: u64,
    progress: u64,
    completed: Option<u64>,
}
impl Payload {
    fn new(total: usize, count: usize, now: u64) -> Option<Self> {
        let mut budget = RETAINED.lock().unwrap_or_else(|p| p.into_inner());
        if total > GLOBAL_BYTES.saturating_sub(*budget) {
            return None;
        }
        // Fallible reservation leaves the global budget unchanged on allocation failure.
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(total).ok()?;
        bytes.resize(total, 0);
        *budget += total;
        Some(Self {
            bytes,
            seen: 0,
            count,
            born: now,
            progress: now,
            completed: None,
        })
    }
}
impl Drop for Payload {
    fn drop(&mut self) {
        let mut n = RETAINED.lock().unwrap_or_else(|p| p.into_inner());
        *n -= self.bytes.len();
    }
}
pub(super) struct Peer {
    pub session: Session,
    pub serial: u64,
    pub ready: bool,
    born: u64,
    pub touched: u64,
    pub highwater: u64,
    payload: Option<Payload>,
}
#[derive(Clone, Copy)]
pub(super) struct Reply {
    pub token: u64,
    pub peer: u64,
    pub session: Session,
    pub serial: u64,
    pub generation: u64,
}
#[derive(Default)]
pub(super) struct State {
    pub registration: Option<u64>,
    pub peers: BTreeMap<u64, Peer>,
    pub reply: Option<Reply>,
    next_serial: u64,
    next_transfer: u64,
    next_reply: u64,
}
impl State {
    pub fn clear(&mut self) {
        if let Some(token) = self.registration.take() {
            shroudforge_steam_networking::receive_router::remove(token);
        }
        self.peers.clear();
        self.reply = None;
    }
    pub fn register(&mut self, owner: &str) -> Result<u64, i32> {
        if let Some(token) = self.registration {
            return Ok(token);
        }
        let token = shroudforge_steam_networking::receive_router::register(owner)
            .map_err(|_| super::NATIVE_STATUS_BUSY)?;
        self.registration = Some(token);
        Ok(token)
    }
    pub fn sweep(&mut self, now: u64) {
        self.peers.retain(|_, p| {
            now.saturating_sub(p.touched) < 60_000
                && (p.ready || now.saturating_sub(p.born) < 5_000)
        });
        for p in self.peers.values_mut() {
            if p.payload.as_ref().is_some_and(|b| {
                if let Some(completed) = b.completed {
                    now.saturating_sub(completed) >= 30_000
                } else {
                    now.saturating_sub(b.progress) >= 5_000 || now.saturating_sub(b.born) >= 30_000
                }
            }) {
                p.payload = None;
            }
        }
        if self.reply.is_some_and(|r| {
            self.peers
                .get(&r.peer)
                .is_none_or(|p| p.serial != r.serial || p.session != r.session)
        }) {
            self.reply = None;
        }
    }
    pub fn retire_peer(&mut self, peer: u64) {
        self.peers.remove(&peer);
        if self.reply.is_some_and(|r| r.peer == peer) {
            self.reply = None;
        }
    }
    pub fn insert_session(
        &mut self,
        peer: u64,
        session: Session,
        ready: bool,
        now: u64,
    ) -> Result<(), i32> {
        if !self.peers.contains_key(&peer) && self.peers.len() >= MAX_PEERS {
            return Err(super::NATIVE_STATUS_BUSY);
        }
        let serial = self
            .next_serial
            .checked_add(1)
            .ok_or(super::NATIVE_STATUS_UNAVAILABLE)?;
        self.next_serial = serial;
        self.retire_peer(peer);
        self.peers.insert(
            peer,
            Peer {
                session,
                serial,
                ready,
                born: now,
                touched: now,
                highwater: 0,
                payload: None,
            },
        );
        Ok(())
    }
    pub fn next_transfer(&mut self) -> Result<u64, i32> {
        self.next_transfer = self
            .next_transfer
            .checked_add(1)
            .ok_or(super::NATIVE_STATUS_UNAVAILABLE)?;
        Ok(self.next_transfer)
    }
    pub fn route(&self, peer: u64) -> Option<(Session, u64)> {
        let p = self.peers.get(&peer)?;
        p.ready.then_some((p.session, p.serial))
    }
    pub fn matching(&self, peer: u64, session: Session, serial: u64) -> bool {
        self.route(peer) == Some((session, serial))
    }
    pub fn accept_ack(&mut self, peer: u64, session: Session, now: u64) {
        if let Some(p) = self.peers.get_mut(&peer) {
            if !p.ready && p.session.client == session.client {
                p.session = session;
                p.ready = true;
                p.touched = now;
            }
        }
    }
    pub fn malformed(&mut self, peer: u64, bytes: &[u8]) {
        if let Some(p) = self.peers.get_mut(&peer) {
            if bytes.get(8) == Some(&wire::DATA)
                && bytes.get(12..28) == Some(p.session.client.as_slice())
                && bytes.get(28..44) == Some(p.session.server.as_slice())
            {
                if let Some(id) = bytes
                    .get(44..52)
                    .and_then(|b| b.try_into().ok())
                    .map(u64::from_le_bytes)
                {
                    if id >= p.highwater {
                        p.highwater = id;
                        p.payload = None;
                    }
                }
            }
        }
    }
    pub fn admit(&mut self, peer: u64, frame: &Frame<'_>, now: u64) {
        let slots = self.peers.values().filter(|p| p.payload.is_some()).count();
        let bytes: usize = self
            .peers
            .values()
            .filter_map(|p| p.payload.as_ref())
            .map(|b| b.bytes.len())
            .sum();
        let Some(p) = self.peers.get_mut(&peer) else {
            return;
        };
        if !p.ready || p.session != frame.session || frame.transfer < p.highwater {
            return;
        }
        if frame.transfer > p.highwater {
            p.highwater = frame.transfer;
            // Undelivered completion wins. New transfer is tombstoned, not queued.
            if p.payload.as_ref().is_some_and(|b| b.completed.is_some()) {
                return;
            }
            let old = p.payload.take();
            let old_len = old.as_ref().map_or(0, |b| b.bytes.len());
            let old_slot = usize::from(old.is_some());
            drop(old);
            if slots - old_slot >= MAX_BUFFERS
                || frame.total > OWNER_BYTES.saturating_sub(bytes - old_len)
            {
                return;
            }
            p.payload = Payload::new(frame.total, frame.count, now);
        }
        let Some(buffer) = p.payload.as_mut() else {
            return;
        };
        // A retained completion has its own id below; higher refused IDs must
        // never allow retransmitted older fragments to mutate it.
        if buffer.completed.is_some() {
            return;
        }
        if buffer.bytes.len() != frame.total || buffer.count != frame.count {
            p.payload = None;
            return;
        }
        let start = frame.index * wire::CHUNK;
        let end = start + frame.data.len();
        let bit = 1u16 << frame.index;
        if buffer.seen & bit != 0 {
            if buffer.bytes[start..end] != *frame.data {
                p.payload = None;
            }
            return; // Identical duplicates renew neither progress nor session idle.
        }
        buffer.bytes[start..end].copy_from_slice(frame.data);
        buffer.seen |= bit;
        buffer.progress = now;
        p.touched = now;
        if buffer.seen == (1u16 << buffer.count) - 1 {
            if std::str::from_utf8(&buffer.bytes).is_ok() {
                buffer.completed = Some(now);
            } else {
                p.payload = None;
            }
        }
    }
    pub fn completed(&self) -> Option<(u64, Session, u64, &[u8])> {
        self.peers.iter().find_map(|(id, p)| {
            p.payload
                .as_ref()
                .filter(|b| b.completed.is_some())
                .map(|b| (*id, p.session, p.serial, b.bytes.as_slice()))
        })
    }
    pub fn prepare_reply(&self, peer: u64, generation: u64) -> Result<Reply, i32> {
        let token = self
            .next_reply
            .checked_add(1)
            .ok_or(super::NATIVE_STATUS_UNAVAILABLE)?;
        let p = self
            .peers
            .get(&peer)
            .ok_or(super::NATIVE_STATUS_UNAVAILABLE)?;
        let reply = Reply {
            token,
            peer,
            session: p.session,
            serial: p.serial,
            generation,
        };
        Ok(reply)
    }
    pub fn publish_reply(&mut self, reply: Reply) {
        self.next_reply = reply.token;
        if let Some(p) = self.peers.get_mut(&reply.peer) {
            p.payload = None;
        }
        self.reply = Some(reply);
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> Session {
        Session {
            client: [1; 16],
            server: [2; 16],
        }
    }
    fn frame(id: u64, total: usize, index: usize, data: &[u8]) -> Frame<'_> {
        Frame {
            kind: wire::DATA,
            owner: "state",
            session: pair(),
            transfer: id,
            total,
            index,
            count: wire::fragment_count(total).unwrap(),
            data,
        }
    }
    #[test]
    fn duplicate_storm_conflict_absolute_and_completed_expiry_have_tombstones() {
        let mut s = State::default();
        s.insert_session(1, pair(), true, 0).unwrap();
        let chunk = vec![b'a'; wire::CHUNK];
        s.admit(1, &frame(1, 65537, 0, &chunk), 0);
        for t in 1..5000 {
            s.admit(1, &frame(1, 65537, 0, &chunk), t);
        }
        s.sweep(5000);
        assert!(s.peers[&1].payload.is_none());
        s.admit(1, &frame(1, 65537, 0, &chunk), 5001);
        assert!(s.peers[&1].payload.is_none());
        s.admit(1, &frame(2, 65537, 0, &chunk), 5002);
        let conflict = vec![b'b'; wire::CHUNK];
        s.admit(1, &frame(2, 65537, 0, &conflict), 5003);
        assert!(s.peers[&1].payload.is_none());
        for index in 0..8 {
            s.admit(
                1,
                &frame(3, wire::MAX_PAYLOAD, index, &chunk),
                6000 + index as u64 * 4000,
            );
        }
        s.sweep(36000);
        assert!(s.peers[&1].payload.is_none());
        s.admit(1, &frame(4, 1, 0, b"x"), 36001);
        assert!(s.completed().is_some());
        s.admit(1, &frame(5, 1, 0, b"y"), 36002);
        assert_eq!(s.completed().unwrap().3, b"x");
        assert_eq!(s.peers[&1].highwater, 5);
        s.sweep(66001);
        assert!(s.completed().is_none());
        s.admit(1, &frame(5, 1, 0, b"y"), 66002);
        assert!(s.completed().is_none());
        s.admit(1, &frame(6, 1, 0, &[255]), 66003);
        assert!(s.completed().is_none());
    }
    #[test]
    fn peer_metadata_payload_slots_and_global_budget_are_separate() {
        let mut owners: Vec<State> = (0..9).map(|_| State::default()).collect();
        let chunk = vec![b'a'; wire::CHUNK];
        for s in &mut owners {
            for id in 1..=64 {
                s.insert_session(id, pair(), true, 0).unwrap();
            }
            assert_eq!(
                s.insert_session(65, pair(), true, 0),
                Err(super::super::NATIVE_STATUS_BUSY)
            );
        }
        for s in owners.iter_mut().take(8) {
            for id in 1..=4 {
                s.admit(id, &frame(1, wire::MAX_PAYLOAD, 0, &chunk), 0);
            }
            assert_eq!(s.peers.values().filter(|p| p.payload.is_some()).count(), 4);
            s.admit(5, &frame(1, wire::MAX_PAYLOAD, 0, &chunk), 0);
            assert!(s.peers[&5].payload.is_none());
            assert_eq!(s.peers.len(), 64);
        }
        owners[8].admit(1, &frame(1, wire::MAX_PAYLOAD, 0, &chunk), 0);
        assert!(owners[8].peers[&1].payload.is_none());
        owners[0].clear();
        owners[8].admit(1, &frame(2, wire::MAX_PAYLOAD, 0, &chunk), 1);
        assert!(owners[8].peers[&1].payload.is_some());
    }
    #[test]
    fn nonwrapping_session_message_and_reply_counters_survive_clear() {
        let mut s = State::default();
        s.insert_session(1, pair(), true, 0).unwrap();
        s.next_serial = u64::MAX;
        assert_eq!(
            s.insert_session(2, pair(), true, 0),
            Err(super::super::NATIVE_STATUS_UNAVAILABLE)
        );
        s.next_transfer = u64::MAX;
        assert_eq!(
            s.next_transfer(),
            Err(super::super::NATIVE_STATUS_UNAVAILABLE)
        );
        s.next_reply = u64::MAX;
        assert!(s.prepare_reply(1, 1).is_err());
        s.clear();
        assert_eq!(s.next_serial, u64::MAX);
        assert_eq!(s.next_transfer, u64::MAX);
        assert_eq!(s.next_reply, u64::MAX);
    }
    #[test]
    fn session_expiry_and_reset_drop_budgets_and_reply_routes() {
        let mut s = State::default();
        s.insert_session(1, pair(), true, 0).unwrap();
        s.admit(1, &frame(1, 1, 0, b"x"), 0);
        let r = s.prepare_reply(1, 7).unwrap();
        s.publish_reply(r);
        assert!(s.reply.is_some());
        s.sweep(60000);
        assert!(s.reply.is_none());
        assert!(s.peers.is_empty());
    }
    #[test]
    fn reply_exhaustion_precedes_copy_and_preserves_completion() {
        use super::super::tests::{FakeTransport, Fixture, SERVER_ID};
        use super::super::v2;
        let fixture = Fixture::new("v2-reply-exhaustion", false, true, true, 333);
        let c = fixture.context();
        let t = FakeTransport::client(Some(SERVER_ID));
        let lease = super::super::OwnerServiceLease::acquire(c).unwrap();
        v2::fresh(&lease, &t, 0).unwrap();
        drop(lease);
        {
            let mut s = c.network.v2.borrow_mut();
            s.insert_session(SERVER_ID, pair(), true, 0).unwrap();
            s.admit(SERVER_ID, &frame(1, 1, 0, b"x"), 0);
            s.next_reply = u64::MAX;
        }
        let mut bytes = [0x55; 8];
        let mut metadata = v2::Message {
            size: 99,
            ..Default::default()
        };
        assert_eq!(
            v2::receive_inner(c, bytes.as_mut_ptr(), 8, 40, &mut metadata, &t, 1),
            Err(super::super::NATIVE_STATUS_UNAVAILABLE)
        );
        assert_eq!(bytes, [0x55; 8]);
        assert_eq!(metadata.size, 99);
        assert!(c.network.v2.borrow().completed().is_some());
    }
}
