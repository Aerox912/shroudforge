//! One shared receive path for legacy/raw Lua, V1 and opt-in V2 owners.
use crate::{Message, mod_messages::mod_channel, mod_network_frame::routed_owner};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
pub const MAX_OWNERS: usize = 128;
pub const MAX_ITEMS: usize = 256;
pub const MAX_ROUTE_ITEMS: usize = 32;
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub struct Router {
    next: u64,
    owners: HashMap<String, u64>,
    queue: VecDeque<(i32, Option<u64>, Message, Instant)>,
    bytes: usize,
    drops: u64,
}
impl Router {
    pub fn register(&mut self, owner: &str) -> Result<u64, String> {
        if owner.is_empty()
            || owner.len() > 256
            || self.owners.len() >= MAX_OWNERS
            || self.owners.contains_key(owner)
        {
            return Err("V2 owner registration unavailable".into());
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or("V2 registration counter exhausted")?;
        self.owners.insert(owner.to_owned(), self.next);
        Ok(self.next)
    }
    pub fn remove(&mut self, token: u64) {
        self.owners.retain(|_, value| *value != token);
        self.queue.retain(|(_, route, message, _)| {
            if *route == Some(token) {
                self.bytes -= message.payload.len();
                false
            } else {
                true
            }
        });
    }
    pub fn admit(&mut self, channel: i32, message: Message) {
        let route = routed_owner(&message.payload)
            .filter(|owner| mod_channel(owner) == channel)
            .and_then(|owner| self.owners.get(owner))
            .copied();
        let len = message.payload.len();
        if self.queue.len() >= MAX_ITEMS
            || len > MAX_BYTES.saturating_sub(self.bytes)
            || self
                .queue
                .iter()
                .filter(|(c, r, _, _)| *c == channel && *r == route)
                .count()
                >= MAX_ROUTE_ITEMS
        {
            self.drops = self.drops.saturating_add(1);
            return;
        }
        self.bytes += len;
        self.queue
            .push_back((channel, route, message, Instant::now()));
    }
    pub fn pop(&mut self, channel: i32, token: Option<u64>) -> Option<Message> {
        let index = self
            .queue
            .iter()
            .position(|(c, r, _, _)| *c == channel && *r == token)?;
        let (_, _, message, _) = self.queue.remove(index)?;
        self.bytes -= message.payload.len();
        Some(message)
    }
    fn sweep(&mut self) {
        self.queue.retain(|(_, _, message, time)| {
            if time.elapsed().as_secs() >= 30 {
                self.bytes -= message.payload.len();
                self.drops = self.drops.saturating_add(1);
                false
            } else {
                true
            }
        });
    }
}
static ROUTER: OnceLock<Mutex<Router>> = OnceLock::new();
static PUMPING: AtomicBool = AtomicBool::new(false);
fn with<T>(f: impl FnOnce(&mut Router) -> T) -> T {
    let mut router = ROUTER
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    router.sweep();
    f(&mut router)
}
pub fn register(owner: &str) -> Result<u64, String> {
    with(|r| r.register(owner))
}
pub fn remove(token: u64) {
    with(|r| r.remove(token))
}
pub fn drops() -> u64 {
    with(|r| r.drops)
}
pub fn queued(channel: i32, token: Option<u64>) -> Option<Message> {
    with(|r| r.pop(channel, token))
}
struct Pump;
impl Drop for Pump {
    fn drop(&mut self) {
        PUMPING.store(false, Ordering::Release);
    }
}
/// At most one physical receive, with no router lock held over the callback.
pub fn poll_with(
    channel: i32,
    token: Option<u64>,
    receive: impl FnOnce(i32) -> Result<Option<Message>, String>,
) -> Result<Option<Message>, String> {
    poll_progress_with(channel, token, receive).map(|outcome| match outcome {
        PollOutcome::Message(message) => Some(message),
        PollOutcome::Routed | PollOutcome::Empty => None,
    })
}

/// A batch must distinguish a consumed packet routed elsewhere from physical
/// emptiness. Routed still consumes one iteration of the caller's fixed budget.
pub enum PollOutcome {
    Message(Message),
    Routed,
    Empty,
}

pub fn poll_progress_with(
    channel: i32,
    token: Option<u64>,
    receive: impl FnOnce(i32) -> Result<Option<Message>, String>,
) -> Result<PollOutcome, String> {
    if let Some(message) = queued(channel, token) {
        return Ok(PollOutcome::Message(message));
    }
    if PUMPING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err("receive pump is busy".into());
    }
    let _pump = Pump;
    let previous = with(|r| r.owners.clone());
    let message = receive(channel)?;
    let progressed = message.is_some();
    if let Some(message) = message {
        with(|r| {
            // If a reserved owner's exact registration retired during the FFI,
            // its in-flight frame must not leak into the legacy queue or into a
            // replacement generation. No router lock spans the receive callback.
            let retired = routed_owner(&message.payload)
                .filter(|o| mod_channel(o) == channel)
                .and_then(|o| previous.get(o).map(|old| r.owners.get(o) != Some(old)))
                .unwrap_or(false);
            if retired {
                r.drops = r.drops.saturating_add(1);
            } else {
                r.admit(channel, message);
            }
        });
    }
    Ok(match queued(channel, token) {
        Some(message) => PollOutcome::Message(message),
        None if progressed => PollOutcome::Routed,
        None => PollOutcome::Empty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_network_frame::*;
    #[test]
    fn full_owner_collision_demux_preserves_raw_and_bounds_overflow() {
        let mut seen = HashMap::new();
        let (a, b) = (0..100_000)
            .find_map(|n| {
                let id = format!("owner-{n}");
                let ch = mod_channel(&id);
                seen.insert(ch, id.clone()).map(|old| (old, id))
            })
            .unwrap();
        let channel = mod_channel(&a);
        assert_eq!(channel, mod_channel(&b));
        let mut r = Router::default();
        let ta = r.register(&a).unwrap();
        let tb = r.register(&b).unwrap();
        let message = |payload| Message {
            peer_steam_id: "1".into(),
            payload,
            reliable: true,
        };
        for owner in [&a, &b] {
            r.admit(
                channel,
                message(
                    encode(&Frame {
                        kind: HELLO,
                        owner,
                        session: Session {
                            client: [1; 16],
                            server: [0; 16],
                        },
                        transfer: 0,
                        total: 0,
                        index: 0,
                        count: 0,
                        data: &[],
                    })
                    .unwrap(),
                ),
            );
        }
        r.admit(channel, message(b"legacy\0raw".to_vec()));
        assert_eq!(r.pop(channel, None).unwrap().payload, b"legacy\0raw");
        assert_eq!(
            decode(&r.pop(channel, Some(tb)).unwrap().payload)
                .unwrap()
                .owner,
            b
        );
        assert_eq!(
            decode(&r.pop(channel, Some(ta)).unwrap().payload)
                .unwrap()
                .owner,
            a
        );
        for _ in 0..40 {
            r.admit(channel, message(vec![0; 524288]));
        }
        assert!(r.bytes <= MAX_BYTES);
        assert!(r.queue.len() <= MAX_ITEMS);
        assert!(r.drops > 0);
        r.remove(ta);
        assert!(r.pop(channel, Some(ta)).is_none());
    }
    #[test]
    fn registration_generation_channel_malformed_and_count_limits() {
        let mut r = Router::default();
        let a = r.register("a").unwrap();
        let channel = mod_channel("a");
        let mut bytes = encode(&Frame {
            kind: HELLO,
            owner: "a",
            session: Session {
                client: [1; 16],
                server: [0; 16],
            },
            transfer: 0,
            total: 0,
            index: 0,
            count: 0,
            data: &[],
        })
        .unwrap();
        bytes[8] = 99;
        let message = |payload| Message {
            peer_steam_id: "1".into(),
            payload,
            reliable: true,
        };
        r.admit(channel + 1, message(bytes.clone()));
        assert_eq!(r.pop(channel + 1, None).unwrap().payload, bytes);
        r.admit(channel, message(bytes));
        assert!(r.pop(channel, None).is_none());
        assert!(r.pop(channel, Some(a)).is_some());
        r.remove(a);
        let b = r.register("a").unwrap();
        r.remove(a);
        assert_eq!(r.owners["a"], b);
        for i in 0..127 {
            r.register(&format!("r{i}")).unwrap();
        }
        assert!(r.register("overflow").is_err());
        for channel in 0..9 {
            for _ in 0..32 {
                r.admit(channel, message(Vec::new()));
            }
        }
        assert_eq!(r.queue.len(), MAX_ITEMS);
        assert!(r.drops > 0);
        for item in &mut r.queue {
            item.3 = Instant::now() - std::time::Duration::from_secs(30);
        }
        r.sweep();
        assert_eq!(r.queue.len(), 0);
        assert_eq!(r.bytes, 0);
        r.next = u64::MAX;
        r.remove(b);
        assert!(r.register("exhausted").is_err());
    }
    #[test]
    fn polling_routes_one_packet_without_locking_across_callback() {
        let owner = "router-reentrant-test";
        let token = register(owner).unwrap();
        let channel = mod_channel(owner);
        let packet = Message {
            peer_steam_id: "1".into(),
            payload: b"normal legacy".to_vec(),
            reliable: true,
        };
        assert!(
            poll_with(channel, Some(token), |_| {
                assert!(drops() < u64::MAX);
                Ok(Some(packet))
            })
            .unwrap()
            .is_none()
        );
        assert_eq!(queued(channel, None).unwrap().payload, b"normal legacy");
        remove(token);
    }
    #[test]
    fn public_receive_paths_preserve_interleaved_v1_raw_and_v2_packets() {
        let owner = "router-public-mixed";
        let token = register(owner).unwrap();
        let channel = mod_channel(owner);
        let v2 = encode(&Frame {
            kind: HELLO,
            owner,
            session: Session {
                client: [1; 16],
                server: [0; 16],
            },
            transfer: 0,
            total: 0,
            index: 0,
            count: 0,
            data: &[],
        })
        .unwrap();
        let v1 = crate::mod_messages::encode_mod_message(owner, owner, "v1 payload").unwrap();
        let raw = b"raw\0bytes".to_vec();
        crate::test_backend(
            Some(crate::Status {
                available: true,
                service_ready: true,
                local_steam_id: Some("1".into()),
                local_dedicated_server_steam_id: None,
                remote_dedicated_server_steam_id: None,
            }),
            [v2.clone(), v1.clone(), raw.clone()]
                .into_iter()
                .map(|payload| Message {
                    peer_steam_id: "2".into(),
                    payload,
                    reliable: true,
                })
                .collect(),
        );
        assert!(crate::receive(channel).unwrap().is_none());
        assert_eq!(
            crate::receive_v2(channel, token).unwrap().unwrap().payload,
            v2
        );
        assert_eq!(crate::receive(channel).unwrap().unwrap().payload, v1);
        assert!(crate::receive_v2(channel, token).unwrap().is_none());
        assert_eq!(crate::receive(channel).unwrap().unwrap().payload, raw);
        crate::test_backend(None, Vec::new());
        remove(token);
    }
    #[test]
    fn retirement_during_physical_receive_cannot_leak_to_legacy_or_new_generation() {
        let owner = "router-retired-inflight";
        let token = register(owner).unwrap();
        let channel = mod_channel(owner);
        let bytes = encode(&Frame {
            kind: HELLO,
            owner,
            session: Session {
                client: [1; 16],
                server: [0; 16],
            },
            transfer: 0,
            total: 0,
            index: 0,
            count: 0,
            data: &[],
        })
        .unwrap();
        let mut replacement = 0;
        assert!(
            poll_with(channel, Some(token), |_| {
                remove(token);
                replacement = register(owner).unwrap();
                Ok(Some(Message {
                    peer_steam_id: "2".into(),
                    payload: bytes,
                    reliable: true,
                }))
            })
            .unwrap()
            .is_none()
        );
        assert!(queued(channel, None).is_none());
        assert!(queued(channel, Some(replacement)).is_none());
        remove(replacement);
    }

    #[test]
    fn per_route_item_limit_drops_only_the_newest_tiny_packet() {
        let mut router = Router::default();
        let channel = 17;
        for value in 0..32 {
            router.admit(
                channel,
                Message {
                    peer_steam_id: "2".into(),
                    payload: vec![value],
                    reliable: true,
                },
            );
        }
        assert_eq!(router.queue.len(), 32);
        assert_eq!(router.bytes, 32);
        assert_eq!(router.drops, 0);
        router.admit(
            channel,
            Message {
                peer_steam_id: "2".into(),
                payload: vec![32],
                reliable: true,
            },
        );
        assert_eq!(router.queue.len(), 32);
        assert_eq!(router.bytes, 32);
        assert_eq!(router.drops, 1);
        for expected in 0..32 {
            assert_eq!(router.pop(channel, None).unwrap().payload, vec![expected]);
        }
        assert!(router.pop(channel, None).is_none());
        assert_eq!(router.bytes, 0);
    }

    #[test]
    fn unknown_owner_prefix_on_a_registered_colliding_channel_stays_legacy() {
        let mut seen = HashMap::new();
        let (known, unknown) = (0..65535)
            .find_map(|n| {
                let id = format!("unknown-case-{n}");
                seen.insert(mod_channel(&id), id.clone())
                    .map(|old| (old, id))
            })
            .unwrap();
        let channel = mod_channel(&known);
        assert_eq!(channel, mod_channel(&unknown));
        let token = register(&known).unwrap();
        struct Cleanup(u64);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::test_backend(None, Vec::new());
                remove(self.0);
            }
        }
        let _cleanup = Cleanup(token);
        let bytes = encode(&Frame {
            kind: HELLO,
            owner: &unknown,
            session: Session {
                client: [1; 16],
                server: [0; 16],
            },
            transfer: 0,
            total: 0,
            index: 0,
            count: 0,
            data: &[],
        })
        .unwrap();
        crate::test_backend(
            Some(crate::Status {
                available: true,
                service_ready: true,
                local_steam_id: Some("1".into()),
                local_dedicated_server_steam_id: None,
                remote_dedicated_server_steam_id: None,
            }),
            vec![Message {
                peer_steam_id: "2".into(),
                payload: bytes.clone(),
                reliable: true,
            }],
        );
        assert!(crate::receive_v2(channel, token).unwrap().is_none());
        assert_eq!(crate::receive(channel).unwrap().unwrap().payload, bytes);
        assert!(queued(channel, Some(token)).is_none());
    }
}
