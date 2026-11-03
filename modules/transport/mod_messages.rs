//! Shared envelope and deterministic-channel rules for same-mod messages.

const PROTOCOL: &str = "shroudforge.mod-message.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModMessage {
    pub from_mod: String,
    pub to_mod: String,
    pub payload: String,
}

/// Serialize the existing v1 envelope. The transport applies its final
/// serialized-message bound after this helper returns.
pub fn encode_mod_message(
    from_mod: &str,
    to_mod: &str,
    payload: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&serde_json::json!({
        "protocol": PROTOCOL,
        "from_mod": from_mod,
        "to_mod": to_mod,
        "payload": payload,
    }))
}

/// Parse exactly the fields accepted by the existing Lua receive_mod path.
/// Other envelope fields are ignored, and from_mod remains sender metadata
/// rather than an authentication claim. Transport authentication is separate.
pub fn decode_mod_message(bytes: &[u8], expected_to_mod: &str) -> Option<ModMessage> {
    let envelope = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
    if envelope["protocol"] != PROTOCOL || envelope["to_mod"].as_str() != Some(expected_to_mod) {
        return None;
    }
    Some(ModMessage {
        from_mod: envelope["from_mod"].as_str()?.to_owned(),
        to_mod: envelope["to_mod"].as_str()?.to_owned(),
        payload: envelope["payload"].as_str()?.to_owned(),
    })
}

/// Keep the Lua and native owner-bound API on the existing deterministic
/// channel. The envelope's full target ID handles hash collisions.
pub fn mod_channel(id: &str) -> i32 {
    let hash = id.bytes().fold(0x811c9dc5u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x01000193)
    });
    ((hash % 65_534) + 1) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_envelope_keeps_existing_lua_fields_and_acceptance_rules() {
        let bytes = encode_mod_message("client-addon", "server-addon", "héllo\n").unwrap();
        let decoded = decode_mod_message(&bytes, "server-addon").unwrap();
        assert_eq!(decoded.from_mod, "client-addon");
        assert_eq!(decoded.to_mod, "server-addon");
        assert_eq!(decoded.payload, "héllo\n");

        let with_unknown = br#"{"protocol":"shroudforge.mod-message.v1","from_mod":"other","to_mod":"server-addon","payload":"ok","extra":true}"#;
        let decoded = decode_mod_message(with_unknown, "server-addon").unwrap();
        assert_eq!(decoded.from_mod, "other");
        assert_eq!(decoded.payload, "ok");
    }

    #[test]
    fn wrong_protocol_target_missing_fields_and_invalid_utf8_are_rejected() {
        for bytes in [
            &br#"{"protocol":"other","from_mod":"a","to_mod":"b","payload":"x"}"#[..],
            &br#"{"protocol":"shroudforge.mod-message.v1","from_mod":"a","to_mod":"other","payload":"x"}"#[..],
            &br#"{"protocol":"shroudforge.mod-message.v1","to_mod":"b","payload":"x"}"#[..],
            &[0xff, 0xfe][..],
        ] {
            assert!(decode_mod_message(bytes, "b").is_none());
        }
    }

    #[test]
    fn channel_is_stable_and_nonzero_for_mod_ids() {
        assert_eq!(
            mod_channel("sf-unlimited-flight"),
            mod_channel("sf-unlimited-flight")
        );
        assert!((1..=65_534).contains(&mod_channel("sf-unlimited-flight")));
    }
}
