//! Steam Networking Messages P2P channel exposed to active runtime mods.

use crate as steam_network;
use crate::mod_messages::{decode_mod_message, encode_mod_message, mod_channel};
use mlua::{Lua, Table};
use mod_loader::Mod;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Runtime facts supplied by the Lua host. Transport owns no execution context.
#[derive(Clone, Copy)]
pub struct RuntimeContext {
    pub root: fn(&Lua) -> std::path::PathBuf,
    pub is_server: fn(&Lua) -> bool,
    /// `Some(true)` or `Some(false)` comes from the host's in-memory settings
    /// snapshot; `None` means the settings could not be read.
    pub network_module_enabled: fn(&Lua) -> Option<bool>,
    pub denial_reason: fn(&Lua, &Mod, &str) -> Option<String>,
}

fn authorized_client_ids(root: &std::path::Path) -> Result<Vec<String>, String> {
    let config = shroudforge_loader_config::config::read_loader(&root)?;
    Ok(config
        .pointer("/modules/network/allowedClientSteamIds")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .split([',', ';'])
        .map(str::trim)
        .filter(|peer| {
            (16..=20).contains(&peer.len())
                && peer.bytes().all(|byte| byte.is_ascii_digit())
                && peer.parse::<u64>().is_ok_and(|id| id != 0)
        })
        .map(str::to_owned)
        .collect())
}

fn steam_id(value: &str) -> Result<u64, String> {
    let id = value
        .parse::<u64>()
        .map_err(|_| "peer Steam ID must be a decimal uint64 string".to_owned())?;
    if id == 0 {
        return Err("peer Steam ID must be nonzero".into());
    }
    Ok(id)
}

fn channel(options: &Option<Table>) -> mlua::Result<i32> {
    let channel = match options {
        Some(options) => options.raw_get::<Option<i32>>("channel")?.unwrap_or(0),
        None => 0,
    };
    if !(0..=65_534).contains(&channel) {
        return Err(mlua::Error::runtime(
            "Channel must be between 0 and 65534. 65535 is reserved for Network health checks",
        ));
    }
    Ok(channel)
}

fn network_policy_denial(
    lua: &Lua,
    owner: &Mod,
    operation: &str,
    context: RuntimeContext,
) -> Option<String> {
    match (context.network_module_enabled)(lua) {
        Some(true) => {}
        Some(false) => {
            let mod_id = owner.info().id.clone();
            static WARNED_MODS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
            let mut warned_mods = WARNED_MODS
                .get_or_init(|| Mutex::new(HashSet::new()))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if warned_mods.insert(mod_id.clone()) {
                tracing::warn!(
                    target: "shroudforge::network",
                    mod_id,
                    operation,
                    "Network module is disabled in Settings. The networking service is unavailable until it is enabled"
                );
            }
            return Some("Network module is disabled in Settings. The networking service is unavailable until it is enabled.".into());
        }
        None => return Some("Network module settings are unavailable".into()),
    }
    None
}

fn denied(lua: &Lua, owner: &Mod, operation: &str, context: RuntimeContext) -> Option<String> {
    if let Some(reason) = (context.denial_reason)(lua, owner, operation) {
        return Some(reason);
    }
    if let Some(reason) = network_policy_denial(lua, owner, operation, context) {
        return Some(reason);
    }
    let status = steam_network::status();
    if !status.available {
        Some("Steam Networking Messages is not initialized in this process".into())
    } else if !status.service_ready {
        Some("Steam Network session callback could not be registered".into())
    } else {
        None
    }
}

pub fn create(lua: &Lua, owner: &Mod, context: RuntimeContext) -> mlua::Result<Table> {
    let network = lua.create_table()?;

    let status_owner = owner.clone();
    network.set(
        "status",
        lua.create_function(move |lua, ()| {
            let reason = denied(lua, &status_owner, "runtime.network.status", context);
            let native = if reason.is_none() {
                steam_network::status()
            } else {
                steam_network::Status {
                    available: false,
                    local_steam_id: None,
                    local_dedicated_server_steam_id: None,
                    remote_dedicated_server_steam_id: None,
                    service_ready: false,
                }
            };
            let result = lua.create_table()?;
            let available = reason.is_none() && native.available;
            result.raw_set("available", available)?;
            result.raw_set(
                "role",
                if (context.is_server)(lua) {
                    "server"
                } else {
                    "client"
                },
            )?;
            result.raw_set("local_steam_id", native.local_steam_id)?;
            result.raw_set(
                "local_dedicated_server_steam_id",
                native.local_dedicated_server_steam_id,
            )?;
            result.raw_set(
                "remote_dedicated_server_steam_id",
                native.remote_dedicated_server_steam_id,
            )?;
            result.raw_set("service_ready", native.service_ready)?;
            let root = (context.root)(lua);
            if let Ok(config) = shroudforge_loader_config::config::read_loader(&root) {
                result.raw_set(
                    "configured_server_steam_id",
                    config
                        .pointer("/modules/network/serverSteamId")
                        .and_then(serde_json::Value::as_str)
                        .filter(|value| value.parse::<u64>().is_ok_and(|id| id != 0)),
                )?;
                let allowed = lua.create_table()?;
                for (index, peer) in authorized_client_ids(&root)
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                {
                    allowed.raw_set(index + 1, peer.as_str())?;
                }
                result.raw_set("allowed_client_steam_ids", allowed)?;
            }
            result.raw_set(
                "reason",
                reason.or_else(|| {
                    (!native.available).then(|| {
                        "Steam Networking Messages is not initialized in this process".to_owned()
                    })
                }),
            )?;
            Ok(result)
        })?,
    )?;

    let send_owner = owner.clone();
    network.set(
        "send",
        lua.create_function(
            move |lua, (peer, payload, options): (String, mlua::String, Option<Table>)| {
                if let Some(reason) = denied(lua, &send_owner, "runtime.network.send", context) {
                    return Ok((false, Some(reason)));
                }
                let peer = match steam_id(&peer) {
                    Ok(peer) => peer,
                    Err(reason) => return Ok((false, Some(reason))),
                };
                let channel = channel(&options)?;
                let reliable = match options {
                    Some(options) => options.raw_get::<Option<bool>>("reliable")?.unwrap_or(true),
                    None => true,
                };
                let payload = payload.as_bytes().to_vec();
                Ok(
                    match steam_network::send(peer, &payload, channel, reliable) {
                        Ok(()) => (true, None),
                        Err(reason) => (false, Some(reason)),
                    },
                )
            },
        )?,
    )?;

    let send_mod_owner = owner.clone();
    network.set(
        "send_mod",
        lua.create_function(move |lua, (peer, target_mod, payload, options): (String, String, String, Option<Table>)| {
            if let Some(reason) = denied(lua, &send_mod_owner, "runtime.network.send", context) {
                return Ok((false, Some(reason)));
            }
            let peer = match steam_id(&peer) {
                Ok(peer) => peer,
                Err(reason) => return Ok((false, Some(reason))),
            };
            if target_mod.is_empty() || target_mod.len() > 96 {
                return Ok((false, Some("target mod ID must contain 1 to 96 bytes".into())));
            }
            let reliable = match options {
                Some(options) => options.raw_get::<Option<bool>>("reliable")?.unwrap_or(true),
                None => true,
            };
            let channel = mod_channel(&target_mod);
            let bytes = encode_mod_message(
                &send_mod_owner.info().id,
                &target_mod,
                &payload,
            )
            .map_err(mlua::Error::external)?;
            Ok(match steam_network::send(peer, &bytes, channel, reliable) {
                Ok(()) => (true, None),
                Err(reason) => (false, Some(reason)),
            })
        })?,
    )?;

    let accept_owner = owner.clone();
    network.set(
        "accept",
        lua.create_function(move |lua, peer: String| {
            if let Some(reason) = denied(lua, &accept_owner, "runtime.network.accept", context) {
                return Ok((false, Some(reason)));
            }
            let peer = match steam_id(&peer) {
                Ok(peer) => peer,
                Err(reason) => return Ok((false, Some(reason))),
            };
            if (context.is_server)(lua) && !steam_network::is_authorized_server_peer(peer) {
                return Ok((false, Some("peer is not a currently authenticated and Network-authorized server client".into())));
            }
            Ok(match steam_network::accept(peer) {
                Ok(true) => (true, None),
                Ok(false) => (
                    false,
                    Some("no pending Steam P2P session for this peer".into()),
                ),
                Err(reason) => (false, Some(reason)),
            })
        })?,
    )?;

    let peers_owner = owner.clone();
    network.set(
        "connected_peers",
        lua.create_function(move |lua, ()| {
            if let Some(reason) = denied(
                lua,
                &peers_owner,
                "runtime.network.connected_peers",
                context,
            ) {
                return Ok((None::<Table>, Some(reason)));
            }
            if !(context.is_server)(lua) {
                return Ok((
                    None,
                    Some(
                        "connected_peers is available only in the Dedicated Server runtime".into(),
                    ),
                ));
            }
            let peers = match steam_network::connected_peers() {
                Ok(peers) => peers,
                Err(reason) => return Ok((None, Some(reason))),
            };
            let allowed = match authorized_client_ids(&(context.root)(lua)) {
                Ok(allowed) => allowed,
                Err(reason) => return Ok((None, Some(reason))),
            };
            let allowed =
                (!allowed.is_empty()).then(|| allowed.into_iter().collect::<HashSet<_>>());
            let result = lua.create_table()?;
            for (index, peer) in peers
                .iter()
                .filter(|peer| {
                    allowed
                        .as_ref()
                        .map_or(true, |allowed| allowed.contains(*peer))
                })
                .enumerate()
            {
                result.raw_set(index + 1, peer.as_str())?;
            }
            Ok((Some(result), None))
        })?,
    )?;

    let receive_owner = owner.clone();
    network.set(
        "receive",
        lua.create_function(move |lua, options: Option<Table>| {
            if let Some(reason) = denied(lua, &receive_owner, "runtime.network.receive", context) {
                return Ok((None, Some(reason)));
            }
            let channel = channel(&options)?;
            let limit = match &options {
                Some(options) => options.raw_get::<Option<usize>>("limit")?.unwrap_or(16),
                None => 16,
            };
            if !(1..=32).contains(&limit) {
                return Err(mlua::Error::runtime(
                    "receive limit must be between 1 and 32",
                ));
            }
            let is_server = (context.is_server)(lua);
            let messages = lua.create_table()?;
            let mut output_index = 0;
            for _ in 1..=limit {
                match steam_network::receive_for_batch(channel) {
                    Ok(steam_network::receive_router::PollOutcome::Message(message)) => {
                        if is_server
                            && message.peer_steam_id.parse::<u64>().map_or(true, |peer| {
                                !steam_network::is_authorized_server_peer(peer)
                            })
                        {
                            continue;
                        }
                        let value = lua.create_table()?;
                        value.raw_set("peer_steam_id", message.peer_steam_id)?;
                        value.raw_set("payload", lua.create_string(&message.payload)?)?;
                        value.raw_set("reliable", message.reliable)?;
                        output_index += 1;
                        messages.raw_set(output_index, value)?;
                    }
                    Ok(steam_network::receive_router::PollOutcome::Routed) => continue,
                    Ok(steam_network::receive_router::PollOutcome::Empty) => break,
                    Err(reason) => return Ok((Some(messages), Some(reason))),
                }
            }
            Ok((Some(messages), None))
        })?,
    )?;

    let receive_mod_owner = owner.clone();
    network.set(
        "receive_mod",
        lua.create_function(move |lua, limit: Option<usize>| {
            if let Some(reason) =
                denied(lua, &receive_mod_owner, "runtime.network.receive", context)
            {
                return Ok((None, Some(reason)));
            }
            let limit = limit.unwrap_or(16);
            if !(1..=32).contains(&limit) {
                return Err(mlua::Error::runtime(
                    "receive limit must be between 1 and 32",
                ));
            }
            let is_server = (context.is_server)(lua);
            let messages = lua.create_table()?;
            let mut output_index = 0;
            for _ in 1..=limit {
                match steam_network::receive_for_batch(mod_channel(&receive_mod_owner.info().id)) {
                    Ok(steam_network::receive_router::PollOutcome::Message(message)) => {
                        if is_server
                            && message.peer_steam_id.parse::<u64>().map_or(true, |peer| {
                                !steam_network::is_authorized_server_peer(peer)
                            })
                        {
                            continue;
                        }
                        let Some(envelope) =
                            decode_mod_message(&message.payload, &receive_mod_owner.info().id)
                        else {
                            continue;
                        };
                        let value = lua.create_table()?;
                        value.raw_set("peer_steam_id", message.peer_steam_id)?;
                        value.raw_set("from_mod", envelope.from_mod)?;
                        value.raw_set("to_mod", envelope.to_mod)?;
                        value.raw_set("payload", envelope.payload)?;
                        value.raw_set("reliable", message.reliable)?;
                        output_index += 1;
                        messages.raw_set(output_index, value)?;
                    }
                    Ok(steam_network::receive_router::PollOutcome::Routed) => continue,
                    Ok(steam_network::receive_router::PollOutcome::Empty) => break,
                    Err(reason) => return Ok((Some(messages), Some(reason))),
                }
            }
            Ok((Some(messages), None))
        })?,
    )?;

    Ok(network)
}

#[cfg(test)]
mod tests {
    use super::{RuntimeContext, create, network_policy_denial};
    use mlua::{Function, Lua, Table};
    use mod_loader::{Mod, ModEnvironment};
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    const POLICY_UNAVAILABLE: usize = 0;
    const POLICY_DISABLED: usize = 1;
    const POLICY_ENABLED: usize = 2;

    static POLICY_STATE: AtomicUsize = AtomicUsize::new(POLICY_UNAVAILABLE);
    static POLICY_READS: AtomicUsize = AtomicUsize::new(0);
    static NEXT_FIXTURE_ID: AtomicUsize = AtomicUsize::new(1);

    #[test]
    fn verifier_lua_mixed_traffic_batch_continues_after_routed_v2() -> mlua::Result<()> {
        let fixture = PackageFixture::new();
        let owner = fixture.package();
        let owner_id = owner.info().id.clone();
        let lua = Lua::new();
        struct BackendReset;
        impl Drop for BackendReset {
            fn drop(&mut self) {
                crate::test_backend(None, Vec::new());
            }
        }
        struct RegistrationReset(u64);
        impl Drop for RegistrationReset {
            fn drop(&mut self) {
                crate::receive_router::remove(self.0);
            }
        }
        let _backend_reset = BackendReset;
        let context = RuntimeContext {
            root: unexpected_config_root,
            is_server,
            network_module_enabled: |_| Some(true),
            denial_reason: no_denial,
        };
        let network = create(&lua, &owner, context)?;
        let registration = crate::receive_router::register(&owner_id).unwrap();
        let _registration_reset = RegistrationReset(registration);
        let channel = crate::mod_messages::mod_channel(&owner_id);
        let hello = crate::mod_network_frame::encode(&crate::mod_network_frame::Frame {
            kind: crate::mod_network_frame::HELLO,
            owner: &owner_id,
            session: crate::mod_network_frame::Session {
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
        let v1 =
            crate::mod_messages::encode_mod_message("sender", &owner_id, "v1 payload").unwrap();
        let raw = b"raw payload".to_vec();
        let status = crate::Status {
            available: true,
            service_ready: true,
            local_steam_id: Some("1".into()),
            local_dedicated_server_steam_id: None,
            remote_dedicated_server_steam_id: Some("2".into()),
        };
        let physical = |payloads: Vec<Vec<u8>>| {
            payloads
                .into_iter()
                .map(|payload| crate::Message {
                    peer_steam_id: "2".into(),
                    payload,
                    reliable: true,
                })
                .collect::<Vec<_>>()
        };

        crate::test_backend(
            Some(status.clone()),
            physical(vec![hello.clone(), v1.clone(), raw.clone()]),
        );
        let options = lua.create_table()?;
        options.set("channel", channel)?;
        options.set("limit", 32)?;
        let (messages, error): (Option<Table>, Option<String>) =
            network.get::<Function>("receive")?.call(options)?;
        assert!(error.is_none());
        let generic_payloads = messages
            .unwrap()
            .sequence_values::<Table>()
            .map(|message| {
                let message = message?;
                message.get::<String>("payload")
            })
            .collect::<mlua::Result<Vec<_>>>()?;
        assert_eq!(
            crate::receive_v2(channel, registration)
                .unwrap()
                .unwrap()
                .payload,
            hello
        );

        crate::test_backend(Some(status), physical(vec![hello, v1.clone()]));
        let (messages, error): (Option<Table>, Option<String>) =
            network.get::<Function>("receive_mod")?.call(32)?;
        assert!(error.is_none());
        let mod_payloads = messages
            .unwrap()
            .sequence_values::<Table>()
            .map(|message| {
                let message = message?;
                message.get::<String>("payload")
            })
            .collect::<mlua::Result<Vec<_>>>()?;

        let expected_generic = vec![String::from_utf8(v1).unwrap(), "raw payload".to_owned()];
        let expected_mod = vec!["v1 payload".to_owned()];
        assert_eq!(
            (generic_payloads, mod_payloads),
            (expected_generic, expected_mod),
            "Lua receive batches stopped after routing the first physical V2 packet"
        );
        Ok(())
    }
    #[test]
    fn legacy_only_lua_batches_still_consume_thirty_two_physical_packets() -> mlua::Result<()> {
        let fixture = PackageFixture::new();
        let owner = fixture.package();
        let lua = Lua::new();
        struct BackendReset;
        impl Drop for BackendReset {
            fn drop(&mut self) {
                crate::test_backend(None, Vec::new());
            }
        }
        let _reset = BackendReset;
        let context = RuntimeContext {
            root: unexpected_config_root,
            is_server,
            network_module_enabled: |_| Some(true),
            denial_reason: no_denial,
        };
        let network = create(&lua, &owner, context)?;
        let status = crate::Status {
            available: true,
            service_ready: true,
            local_steam_id: Some("1".into()),
            local_dedicated_server_steam_id: None,
            remote_dedicated_server_steam_id: Some("2".into()),
        };
        crate::test_backend(
            Some(status.clone()),
            (0..32)
                .map(|n| crate::Message {
                    peer_steam_id: "2".into(),
                    payload: format!("raw-{n}").into_bytes(),
                    reliable: true,
                })
                .collect(),
        );
        let options = lua.create_table()?;
        options.set("limit", 32)?;
        let (messages, error): (Option<Table>, Option<String>) =
            network.get::<Function>("receive")?.call(options)?;
        assert!(error.is_none());
        let messages = messages.unwrap();
        assert_eq!(messages.raw_len(), 32);
        assert_eq!(
            messages.get::<Table>(32)?.get::<String>("payload")?,
            "raw-31"
        );
        crate::test_backend(
            Some(status),
            (0..32)
                .map(|n| crate::Message {
                    peer_steam_id: "2".into(),
                    payload: crate::mod_messages::encode_mod_message(
                        "sender",
                        &owner.info().id,
                        &format!("mod-{n}"),
                    )
                    .unwrap(),
                    reliable: true,
                })
                .collect(),
        );
        let (messages, error): (Option<Table>, Option<String>) =
            network.get::<Function>("receive_mod")?.call(32)?;
        assert!(error.is_none());
        let messages = messages.unwrap();
        assert_eq!(messages.raw_len(), 32);
        assert_eq!(
            messages.get::<Table>(32)?.get::<String>("payload")?,
            "mod-31"
        );
        Ok(())
    }

    struct PackageFixture {
        environment: ModEnvironment,
        root: PathBuf,
    }

    impl PackageFixture {
        fn new() -> Self {
            let base = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let fixture_id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let root = base.join(format!(
                "lua-network-policy-{}-{fixture_id}",
                std::process::id()
            ));
            let package_root = root.join("mods").join("network-test");
            fs::create_dir_all(&package_root).unwrap();
            shroudforge_loader_config::config::initialize_loader_config(&root).unwrap();
            fs::write(
                package_root.join("mod.json"),
                br#"{"id":"network-test","name":"Network test","version":"1.0.0","capabilities":["runtime"]}"#,
            )
            .unwrap();
            fs::write(
                package_root.join("extended.mod.json"),
                br#"{"schemaVersion":1,"enabled":true,"targets":["client"],"settings":{}}"#,
            )
            .unwrap();
            let environment = ModEnvironment::load(root.to_str().unwrap()).unwrap();
            Self { environment, root }
        }

        fn package(&self) -> Mod {
            self.environment
                .mod_registry()
                .get("network-test")
                .unwrap()
                .clone()
        }
    }

    impl Drop for PackageFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn unexpected_config_root(_: &Lua) -> PathBuf {
        panic!("send_mod and receive_mod must not resolve loader config paths")
    }

    fn is_server(_: &Lua) -> bool {
        false
    }

    fn network_module_enabled(_: &Lua) -> Option<bool> {
        POLICY_READS.fetch_add(1, Ordering::Relaxed);
        match POLICY_STATE.load(Ordering::Relaxed) {
            POLICY_UNAVAILABLE => None,
            POLICY_DISABLED => Some(false),
            POLICY_ENABLED => Some(true),
            _ => unreachable!("invalid test policy state"),
        }
    }

    fn no_denial(_: &Lua, _: &Mod, _: &str) -> Option<String> {
        None
    }

    fn expect_policy_denial(
        send_mod: &Function,
        receive_mod: &Function,
        expected: &str,
    ) -> mlua::Result<()> {
        let (sent, send_reason): (bool, Option<String>) =
            send_mod.call(("76561198000000000", "target-mod", "payload", None::<Table>))?;
        assert!(!sent);
        assert!(
            send_reason
                .as_deref()
                .unwrap_or_default()
                .contains(expected)
        );

        let (messages, receive_reason): (Option<Table>, Option<String>) =
            receive_mod.call(None::<usize>)?;
        assert!(messages.is_none());
        assert!(
            receive_reason
                .as_deref()
                .unwrap_or_default()
                .contains(expected)
        );
        Ok(())
    }

    #[test]
    fn routed_only_lua_batches_stop_at_their_physical_limit() -> mlua::Result<()> {
        let fixture = PackageFixture::new();
        let owner = fixture.package();
        let owner_id = owner.info().id.clone();
        let lua = Lua::new();
        struct Cleanup(u64);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::test_backend(None, Vec::new());
                crate::receive_router::remove(self.0);
            }
        }
        let token = crate::receive_router::register(&owner_id).unwrap();
        let _cleanup = Cleanup(token);
        let channel = crate::mod_messages::mod_channel(&owner_id);
        let hello = crate::mod_network_frame::encode(&crate::mod_network_frame::Frame {
            kind: crate::mod_network_frame::HELLO,
            owner: &owner_id,
            session: crate::mod_network_frame::Session {
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
        let v1 = crate::mod_messages::encode_mod_message("sender", &owner_id, "after32").unwrap();
        let network = create(
            &lua,
            &owner,
            RuntimeContext {
                root: unexpected_config_root,
                is_server,
                network_module_enabled: |_| Some(true),
                denial_reason: no_denial,
            },
        )?;
        let status = crate::Status {
            available: true,
            service_ready: true,
            local_steam_id: Some("1".into()),
            local_dedicated_server_steam_id: None,
            remote_dedicated_server_steam_id: Some("2".into()),
        };
        for name in ["receive", "receive_mod"] {
            crate::test_backend(
                Some(status.clone()),
                std::iter::repeat_n(hello.clone(), 32)
                    .chain(std::iter::once(v1.clone()))
                    .map(|payload| crate::Message {
                        peer_steam_id: "2".into(),
                        payload,
                        reliable: true,
                    })
                    .collect(),
            );
            let call = |limit: usize| -> mlua::Result<(Option<Table>, Option<String>)> {
                let f = network.get::<Function>(name)?;
                if name == "receive" {
                    let options = lua.create_table()?;
                    options.set("channel", channel)?;
                    options.set("limit", limit)?;
                    f.call(options)
                } else {
                    f.call(limit)
                }
            };
            let (messages, error) = call(32)?;
            assert!(error.is_none());
            assert_eq!(messages.unwrap().raw_len(), 0);
            let (messages, error) = call(1)?;
            assert!(error.is_none());
            assert_eq!(
                messages.unwrap().raw_len(),
                1,
                "{name} must leave packet33 for the next call"
            );
            for _ in 0..32 {
                assert!(crate::receive_router::queued(channel, Some(token)).is_some());
            }
        }
        Ok(())
    }

    #[test]
    fn send_receive_mod_use_host_policy_snapshot_and_fail_closed_transitions() -> mlua::Result<()> {
        let fixture = PackageFixture::new();
        let owner = fixture.package();
        let lua = Lua::new();
        let context = RuntimeContext {
            root: unexpected_config_root,
            is_server,
            network_module_enabled,
            denial_reason: no_denial,
        };
        let network = create(&lua, &owner, context)?;
        let send_mod = network.get::<Function>("send_mod")?;
        let receive_mod = network.get::<Function>("receive_mod")?;
        POLICY_READS.store(0, Ordering::Relaxed);

        POLICY_STATE.store(POLICY_UNAVAILABLE, Ordering::Relaxed);
        for _ in 0..3 {
            expect_policy_denial(
                &send_mod,
                &receive_mod,
                "Network module settings are unavailable",
            )?;
        }
        assert_eq!(POLICY_READS.load(Ordering::Relaxed), 6);

        POLICY_STATE.store(POLICY_DISABLED, Ordering::Relaxed);
        for _ in 0..3 {
            expect_policy_denial(
                &send_mod,
                &receive_mod,
                "Network module is disabled in Settings",
            )?;
        }
        assert_eq!(POLICY_READS.load(Ordering::Relaxed), 12);

        POLICY_STATE.store(POLICY_ENABLED, Ordering::Relaxed);
        assert_eq!(
            network_policy_denial(&lua, &owner, "runtime.network.send", context),
            None
        );
        assert_eq!(POLICY_READS.load(Ordering::Relaxed), 13);
        Ok(())
    }
}
