use super::super::tests::{CLIENT_ID, FakeTransport, Fixture, SERVER_ID};
use super::*;

fn ctx_ptr(c: &NativePluginHostContext) -> *mut c_void {
    (c as *const NativePluginHostContext).cast_mut().cast()
}
fn install_session(c: &NativePluginHostContext, peer: u64, time: u64) -> Session {
    let session = Session {
        client: [1; 16],
        server: [2; 16],
    };
    with_state(c, |s| s.insert_session(peer, session, true, time))
        .unwrap()
        .unwrap();
    session
}
fn input_frame(
    owner: &str,
    session: Session,
    id: u64,
    total: usize,
    index: usize,
    data: &[u8],
) -> Vec<u8> {
    wire::encode(&Frame {
        kind: wire::DATA,
        owner,
        session,
        transfer: id,
        total,
        index,
        count: wire::fragment_count(total).unwrap(),
        data,
    })
    .unwrap()
}
fn push(t: &FakeTransport, peer: u64, bytes: Vec<u8>) {
    t.messages
        .borrow_mut()
        .push_back(Some(shroudforge_steam_networking::Message {
            peer_steam_id: peer.to_string(),
            payload: bytes,
            reliable: true,
        }));
}
#[test]
fn abi_v2_layout_and_legacy_layout_stay_separate() {
    assert_eq!(size_of::<Service>(), 72);
    assert_eq!(size_of::<Status>(), 56);
    assert_eq!(size_of::<SendResult>(), 24);
    assert_eq!(size_of::<Message>(), 40);
    assert_eq!(size_of::<NetworkServiceV1>(), 56);
    assert_eq!(size_of::<NetworkStatusV1>(), 40);
    assert_eq!(size_of::<NetworkMessageV1>(), 32);
}
#[test]
fn handshake_timeout_reset_and_old_session_replay() {
    let fixture = Fixture::new("v2-handshake", false, true, true, 301);
    let c = fixture.context();
    let t = FakeTransport::client(Some(SERVER_ID));
    let mut report = SendResult::empty();
    begin_inner(c, &t, 0, &mut report).unwrap();
    assert_eq!(report.attempted_frames, 1);
    let hello = t.sent.borrow()[0].1.clone();
    let hello = wire::decode(&hello).unwrap();
    assert_eq!(hello.kind, wire::HELLO);
    assert_eq!(with_state(c, |s| s.peers.len()).unwrap(), 1);
    with_state(c, |s| s.sweep(5000)).unwrap();
    assert_eq!(with_state(c, |s| s.peers.len()).unwrap(), 0);
    let old = hello.session;
    begin_inner(c, &t, 5001, &mut SendResult::empty()).unwrap();
    let new = with_state(c, |s| s.peers[&SERVER_ID].session).unwrap();
    assert_ne!(old.client, new.client);
    with_state(c, |s| {
        s.accept_ack(
            SERVER_ID,
            Session {
                client: old.client,
                server: [9; 16],
            },
            5002,
        )
    })
    .unwrap();
    assert!(with_state(c, |s| s.route(SERVER_ID)).unwrap().is_none());
    with_state(c, |s| {
        s.accept_ack(
            SERVER_ID,
            Session {
                client: new.client,
                server: [9; 16],
            },
            5003,
        )
    })
    .unwrap();
    assert!(with_state(c, |s| s.route(SERVER_ID)).unwrap().is_some());
    assert_eq!(unsafe { reset(ctx_ptr(c)) }, 0);
    assert!(with_state(c, |s| s.peers.is_empty()).unwrap());
}
#[test]
fn full_payload_boundaries_are_submitted_as_exact_raw_fragments() {
    let fixture = Fixture::new("v2-send-boundaries", false, true, true, 302);
    let c = fixture.context();
    let t = FakeTransport::client(Some(SERVER_ID));
    // Observe identity before installing synthetic paired state.
    fresh(&OwnerServiceLease::acquire(c).unwrap(), &t, 0).unwrap();
    install_session(c, SERVER_ID, 0);
    for len in [0, 65536, 65537, 524288] {
        t.sent.borrow_mut().clear();
        let mut data = "\0\n\r\t\u{1}漢字é🙂".repeat(len / 20 + 1).into_bytes();
        data.truncate(len);
        while std::str::from_utf8(&data).is_err() {
            data.pop();
        }
        data.resize(len, b'a');
        let mut report = SendResult::empty();
        send_inner(c, None, data.as_ptr(), len as u32, &t, 1, &mut report).unwrap();
        assert_eq!(
            report.accepted_frames,
            wire::fragment_count(len).unwrap() as u32
        );
        assert_eq!(report.delivery_uncertain, 1);
        let mut reconstructed = Vec::new();
        for (_, bytes, _) in t.sent.borrow().iter() {
            let f = wire::decode(bytes).unwrap();
            reconstructed.extend_from_slice(f.data);
            assert!(bytes.len() <= 65536);
        }
        assert_eq!(reconstructed, data);
    }
    let mut report = SendResult::empty();
    assert_eq!(
        send_inner(c, None, std::ptr::null(), 524289, &t, 2, &mut report),
        Err(NATIVE_STATUS_TOO_LARGE)
    );
    assert_eq!(report.attempted_frames, 0);
}
#[test]
fn receive_out_of_order_duplicate_small_buffer_retry_and_reply_retirement() {
    let fixture = Fixture::new("v2-reassembly", false, true, true, 303);
    let c = fixture.context();
    let t = FakeTransport::client(Some(SERVER_ID));
    fresh(&OwnerServiceLease::acquire(c).unwrap(), &t, 0).unwrap();
    let session = install_session(c, SERVER_ID, 0);
    let data = vec![b'a'; 65537];
    let owner = c.package_id();
    let tail = input_frame(owner, session, 1, data.len(), 1, &data[wire::CHUNK..]);
    push(&t, SERVER_ID, tail.clone());
    push(&t, SERVER_ID, tail);
    push(
        &t,
        SERVER_ID,
        input_frame(owner, session, 1, data.len(), 0, &data[..wire::CHUNK]),
    );
    let mut bytes = vec![0x55; 65537];
    let mut meta = Message::default();
    for _ in 0..2 {
        assert_eq!(
            receive_inner(c, bytes.as_mut_ptr(), 1, 40, &mut meta, &t, 1),
            Err(NATIVE_STATUS_NOT_FOUND)
        );
    }
    assert_eq!(
        receive_inner(c, bytes.as_mut_ptr(), 1, 40, &mut meta, &t, 2),
        Err(NATIVE_STATUS_BUFFER_TOO_SMALL)
    );
    assert_eq!(meta.payload_bytes, 65537);
    assert_eq!(meta.reply_token, 0);
    assert!(bytes.iter().all(|b| *b == 0x55));
    let reads = t.receive_channels.borrow().len();
    receive_inner(
        c,
        bytes.as_mut_ptr(),
        bytes.len() as u32,
        40,
        &mut meta,
        &t,
        3,
    )
    .unwrap();
    assert_eq!(bytes, data);
    assert_ne!(meta.reply_token, 0);
    assert_eq!(t.receive_channels.borrow().len(), reads);
    let mut report = SendResult::empty();
    send_inner(
        c,
        Some(meta.reply_token),
        b"ok".as_ptr(),
        2,
        &t,
        4,
        &mut report,
    )
    .unwrap();
    assert_eq!(
        receive_inner(
            c,
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            40,
            &mut meta,
            &t,
            5
        ),
        Err(NATIVE_STATUS_NOT_FOUND)
    );
    assert_eq!(
        send_inner(
            c,
            Some(meta.reply_token),
            b"ok".as_ptr(),
            2,
            &t,
            6,
            &mut SendResult::empty()
        ),
        Err(NATIVE_STATUS_UNAVAILABLE)
    );
}

struct FailingTransport {
    base: FakeTransport,
    fail: usize,
    calls: Cell<usize>,
}
impl Transport for FailingTransport {
    fn status(&self) -> shroudforge_steam_networking::Status {
        self.base.status()
    }
    fn send(&self, p: u64, b: &[u8], c: i32) -> Result<(), String> {
        let n = self.calls.get() + 1;
        self.calls.set(n);
        self.base.send(p, b, c)?;
        if n == self.fail {
            Err("local rejection".into())
        } else {
            Ok(())
        }
    }
    fn receive(&self, c: i32) -> Result<Option<shroudforge_steam_networking::Message>, String> {
        self.base.receive(c)
    }
    fn is_authorized_server_peer(&self, p: u64) -> bool {
        self.base.is_authorized_server_peer(p)
    }
}
#[test]
fn every_fragment_failure_stops_batch_and_reports_attempted_versus_local_acceptance() {
    let fixture = Fixture::new("v2-partial-send", false, true, true, 304);
    let c = fixture.context();
    let data = vec![b'x'; 524288];
    for fail in 1..=9 {
        let t = FailingTransport {
            base: FakeTransport::client(Some(SERVER_ID)),
            fail,
            calls: Cell::new(0),
        };
        fresh(&OwnerServiceLease::acquire(c).unwrap(), &t, 0).unwrap();
        install_session(c, SERVER_ID, 0);
        let mut report = SendResult::empty();
        assert_eq!(
            send_inner(
                c,
                None,
                data.as_ptr(),
                data.len() as u32,
                &t,
                1,
                &mut report
            ),
            Err(NATIVE_STATUS_FAILED)
        );
        assert_eq!(report.attempted_frames, fail as u32);
        assert_eq!(report.accepted_frames, (fail - 1) as u32);
        assert_eq!(report.delivery_uncertain, 1);
        assert_eq!(t.calls.get(), fail);
    }
}
#[test]
fn reentrant_reset_policy_disable_enable_and_revoke_stop_after_current_fragment() {
    for action in 0..3 {
        let fixture = Fixture::new(
            &format!("v2-reentrant-{action}"),
            false,
            true,
            true,
            305 + action,
        );
        let c = fixture.context();
        let t = FakeTransport::client(Some(SERVER_ID));
        fresh(&OwnerServiceLease::acquire(c).unwrap(), &t, 0).unwrap();
        install_session(c, SERVER_ID, 0);
        let address = ctx_ptr(c) as usize;
        *t.on_send.borrow_mut() = Some(Box::new(move || {
            let ptr = address as *mut c_void;
            let c = unsafe { host_context(ptr) }.unwrap();
            match action {
                0 => assert_eq!(unsafe { reset(ptr) }, NATIVE_STATUS_OK),
                1 => {
                    c.update_network_policy(NativeNetworkPolicy::Disabled)
                        .unwrap();
                    c.update_network_policy(NativeNetworkPolicy::Enabled)
                        .unwrap();
                }
                _ => {
                    assert_eq!(
                        unsafe { host_revoke_services(ptr, c.package_generation) },
                        NATIVE_STATUS_OK
                    );
                }
            }
        }));
        let payload = vec![b'x'; 65537];
        let mut report = SendResult::empty();
        assert_eq!(
            send_inner(
                c,
                None,
                payload.as_ptr(),
                payload.len() as u32,
                &t,
                1,
                &mut report
            ),
            Err(NATIVE_STATUS_UNAVAILABLE)
        );
        assert_eq!(report.attempted_frames, 1);
        assert_eq!(report.accepted_frames, 1);
        assert_eq!(report.delivery_uncertain, 1);
        assert!(with_state(c, |s| s.peers.is_empty() && s.registration.is_none()).unwrap());
        assert!(!c.network.has_active_call());
    }
}
#[test]
fn actual_server_hello_ack_data_authorization_and_observable_identity_retirement() {
    let fixture = Fixture::new("v2-server-wire", true, true, true, 310);
    let c = fixture.context();
    let t = FakeTransport::server();
    t.authorized.borrow_mut().insert(CLIENT_ID);
    let pending = Session {
        client: [9; 16],
        server: [0; 16],
    };
    push(&t, CLIENT_ID, control(c.package_id(), wire::HELLO, pending));
    let mut output = [0x55; 4];
    let mut meta = Message::default();
    assert_eq!(
        receive_inner(c, output.as_mut_ptr(), 4, 40, &mut meta, &t, 0),
        Err(NATIVE_STATUS_NOT_FOUND)
    );
    let ack = t.sent.borrow()[0].1.clone();
    let ack = wire::decode(&ack).unwrap();
    assert_eq!(ack.kind, wire::ACK);
    assert_eq!(ack.session.client, pending.client);
    assert_ne!(ack.session.server, [0; 16]);
    push(
        &t,
        CLIENT_ID,
        input_frame(c.package_id(), ack.session, 1, 3, 0, b"a\0b"),
    );
    receive_inner(c, output.as_mut_ptr(), 4, 40, &mut meta, &t, 1).unwrap();
    assert_eq!(&output[..3], b"a\0b");
    assert_ne!(meta.reply_token, 0);
    t.authorized.borrow_mut().clear();
    assert_eq!(
        send_inner(
            c,
            Some(meta.reply_token),
            b"x".as_ptr(),
            1,
            &t,
            2,
            &mut SendResult::empty()
        ),
        Err(NATIVE_STATUS_UNAVAILABLE)
    );
    assert!(with_state(c, |s| s.peers.is_empty() && s.reply.is_none()).unwrap());
    // A known local identity transition clears all state and queued registration.
    install_session(c, CLIENT_ID, 3);
    t.status.borrow_mut().local_steam_id = Some("999".into());
    let mut status = Status::default();
    assert_eq!(status_inner(c, 56, &mut status, &t, 4), 0);
    assert_eq!(status.logical_peers, 0);
}
#[test]
fn small_send_outputs_invalid_args_and_no_session_report_zero_attempts() {
    let fixture = Fixture::new("v2-output-contract", false, true, true, 311);
    let c = fixture.context();
    let t = FakeTransport::client(Some(SERVER_ID));
    let mut report = SendResult {
        size: 99,
        ..Default::default()
    };
    assert_eq!(
        report_call(23, &mut report, |_| panic!("must not run")),
        NATIVE_STATUS_BUFFER_TOO_SMALL
    );
    assert_eq!(report.size, 99);
    assert_eq!(
        report_call(24, &mut report, |r| send_inner(
            c,
            None,
            b"x".as_ptr(),
            1,
            &t,
            0,
            r
        )),
        NATIVE_STATUS_UNAVAILABLE
    );
    assert_eq!(report, SendResult::empty());
    assert_eq!(
        report_call(24, &mut report, |r| send_inner(
            c,
            None,
            std::ptr::null(),
            1,
            &t,
            0,
            r
        )),
        NATIVE_STATUS_INVALID_ARGUMENT
    );
    assert_eq!(report.attempted_frames, 0);
}

#[test]
fn unreliable_fragment_is_refused_and_cannot_complete_a_reliable_message() {
    let fixture = Fixture::new("v2-reliable-gate", false, true, true, 312);
    let c = fixture.context();
    let t = FakeTransport::client(Some(SERVER_ID));
    fresh(&OwnerServiceLease::acquire(c).unwrap(), &t, 0).unwrap();
    let session = install_session(c, SERVER_ID, 0);
    let data = vec![b'a'; 65537];
    push(
        &t,
        SERVER_ID,
        input_frame(
            c.package_id(),
            session,
            1,
            data.len(),
            0,
            &data[..wire::CHUNK],
        ),
    );
    t.messages
        .borrow_mut()
        .push_back(Some(shroudforge_steam_networking::Message {
            peer_steam_id: SERVER_ID.to_string(),
            payload: input_frame(
                c.package_id(),
                session,
                1,
                data.len(),
                1,
                &data[wire::CHUNK..],
            ),
            reliable: false,
        }));
    let mut bytes = vec![0x55; data.len()];
    let mut meta = Message::default();
    for time in 0..2 {
        assert_eq!(
            receive_inner(
                c,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                40,
                &mut meta,
                &t,
                time
            ),
            Err(NATIVE_STATUS_NOT_FOUND)
        );
    }
    assert_eq!(meta.flags, 0);
    assert!(bytes.iter().all(|b| *b == 0x55));
    push(
        &t,
        SERVER_ID,
        input_frame(
            c.package_id(),
            session,
            1,
            data.len(),
            1,
            &data[wire::CHUNK..],
        ),
    );
    receive_inner(
        c,
        bytes.as_mut_ptr(),
        bytes.len() as u32,
        40,
        &mut meta,
        &t,
        2,
    )
    .unwrap();
    assert_eq!(meta.flags, 1);
    assert_eq!(bytes, data);
}

#[test]
fn multibyte_utf8_split_inside_fragment_boundary_reconstructs_exact_bytes() {
    let fixture = Fixture::new("v2-split-unicode", false, true, true, 313);
    let c = fixture.context();
    let t = FakeTransport::client(Some(SERVER_ID));
    fresh(&OwnerServiceLease::acquire(c).unwrap(), &t, 0).unwrap();
    let session = install_session(c, SERVER_ID, 0);
    for (id, character) in [(1, "🙂"), (2, "漢")] {
        let mut payload = vec![b'a'; wire::CHUNK - 1];
        payload.extend_from_slice(character.as_bytes());
        payload.extend_from_slice("é漢字尾".as_bytes());
        assert!(std::str::from_utf8(&payload).is_ok());
        assert!(std::str::from_utf8(&payload[..wire::CHUNK]).is_err());
        assert!(std::str::from_utf8(&payload[wire::CHUNK..]).is_err());
        push(
            &t,
            SERVER_ID,
            input_frame(
                c.package_id(),
                session,
                id,
                payload.len(),
                0,
                &payload[..wire::CHUNK],
            ),
        );
        push(
            &t,
            SERVER_ID,
            input_frame(
                c.package_id(),
                session,
                id,
                payload.len(),
                1,
                &payload[wire::CHUNK..],
            ),
        );
        let mut bytes = vec![0x55; payload.len()];
        let mut meta = Message {
            size: 99,
            ..Default::default()
        };
        assert_eq!(
            receive_inner(
                c,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                40,
                &mut meta,
                &t,
                id * 2
            ),
            Err(NATIVE_STATUS_NOT_FOUND)
        );
        assert!(bytes.iter().all(|b| *b == 0x55));
        assert_eq!(meta.size, 99);
        receive_inner(
            c,
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            40,
            &mut meta,
            &t,
            id * 2 + 1,
        )
        .unwrap();
        assert_eq!(bytes, payload);
        assert_ne!(meta.reply_token, 0);
    }
}

#[test]
fn newer_transfer_supersedes_incomplete_without_old_fragment_resurrection() {
    let fixture = Fixture::new("v2-transfer-replacement", false, true, true, 314);
    let c = fixture.context();
    let t = FakeTransport::client(Some(SERVER_ID));
    fresh(&OwnerServiceLease::acquire(c).unwrap(), &t, 0).unwrap();
    let session = install_session(c, SERVER_ID, 0);
    let old = vec![b'o'; 65537];
    let new = vec![b'n'; 65537];
    for (id, index, data) in [(1, 0, &old), (2, 1, &new), (1, 1, &old), (2, 0, &new)] {
        let start = index * wire::CHUNK;
        push(
            &t,
            SERVER_ID,
            input_frame(
                c.package_id(),
                session,
                id,
                data.len(),
                index,
                &data[start..(start + wire::CHUNK).min(data.len())],
            ),
        );
    }
    let mut bytes = vec![0x55; new.len()];
    let mut meta = Message::default();
    for time in 0..3 {
        assert_eq!(
            receive_inner(
                c,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                40,
                &mut meta,
                &t,
                time
            ),
            Err(NATIVE_STATUS_NOT_FOUND)
        );
        assert!(bytes.iter().all(|b| *b == 0x55));
    }
    receive_inner(
        c,
        bytes.as_mut_ptr(),
        bytes.len() as u32,
        40,
        &mut meta,
        &t,
        3,
    )
    .unwrap();
    assert_eq!(bytes, new);
    for id in [1, 2] {
        push(
            &t,
            SERVER_ID,
            input_frame(
                c.package_id(),
                session,
                id,
                old.len(),
                1,
                &old[wire::CHUNK..],
            ),
        );
        assert_eq!(
            receive_inner(
                c,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                40,
                &mut meta,
                &t,
                4
            ),
            Err(NATIVE_STATUS_NOT_FOUND)
        );
        assert_eq!(bytes, new);
    }
}
