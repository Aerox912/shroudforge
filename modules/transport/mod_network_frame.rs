//! Binary framing for the opt-in mod-network V2 service. Not an application protocol.
pub const MAGIC: &[u8; 8] = b"SFNET2\r\n";
pub const HEADER: usize = 60;
pub const CHUNK: usize = 61_440;
pub const MAX_PAYLOAD: usize = 524_288;
pub const MAX_FRAME: usize = 65_536;
pub const HELLO: u8 = 1;
pub const ACK: u8 = 2;
pub const DATA: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    pub client: [u8; 16],
    pub server: [u8; 16],
}

#[derive(Debug)]
pub struct Frame<'a> {
    pub kind: u8,
    pub owner: &'a str,
    pub session: Session,
    pub transfer: u64,
    pub total: usize,
    pub index: usize,
    pub count: usize,
    pub data: &'a [u8],
}

/// Only this bounded prefix determines routing. Once a registered owner opts in,
/// its reserved prefix (including malformed remaining fields) belongs to V2.
pub fn routed_owner(bytes: &[u8]) -> Option<&str> {
    if bytes.get(..8)? != MAGIC {
        return None;
    }
    let n = u16::from_le_bytes(bytes.get(10..12)?.try_into().ok()?) as usize;
    if !(1..=256).contains(&n) {
        return None;
    }
    std::str::from_utf8(bytes.get(HEADER..HEADER.checked_add(n)?)?).ok()
}

pub fn fragment_count(total: usize) -> Option<usize> {
    (total <= MAX_PAYLOAD).then(|| total.div_ceil(CHUNK).max(1))
}

pub fn decode(bytes: &[u8]) -> Option<Frame<'_>> {
    if bytes.len() > MAX_FRAME || bytes.get(9)? != &0 {
        return None;
    }
    let owner = routed_owner(bytes)?;
    let kind = *bytes.get(8)?;
    let session = Session {
        client: bytes.get(12..28)?.try_into().ok()?,
        server: bytes.get(28..44)?.try_into().ok()?,
    };
    let transfer = u64::from_le_bytes(bytes.get(44..52)?.try_into().ok()?);
    let total = u32::from_le_bytes(bytes.get(52..56)?.try_into().ok()?) as usize;
    let index = u16::from_le_bytes(bytes.get(56..58)?.try_into().ok()?) as usize;
    let count = u16::from_le_bytes(bytes.get(58..60)?.try_into().ok()?) as usize;
    let data = bytes.get(HEADER.checked_add(owner.len())?..)?;
    if session.client == [0; 16] {
        return None;
    }
    match kind {
        HELLO | ACK => {
            if transfer != 0
                || total != 0
                || index != 0
                || count != 0
                || !data.is_empty()
                || (kind == HELLO && session.server != [0; 16])
                || (kind == ACK && session.server == [0; 16])
            {
                return None;
            }
        }
        DATA => {
            if session.server == [0; 16]
                || transfer == 0
                || count != fragment_count(total)?
                || index >= count
            {
                return None;
            }
            let start = index.checked_mul(CHUNK)?;
            let expected = total.checked_sub(start)?.min(CHUNK);
            if data.len() != expected {
                return None;
            }
        }
        _ => return None,
    }
    Some(Frame {
        kind,
        owner,
        session,
        transfer,
        total,
        index,
        count,
        data,
    })
}

/// Exactly one physical frame allocation; caller never needs an expanded
/// JSON/base64 representation or a vector of all serialized fragments.
pub fn encode(frame: &Frame<'_>) -> Option<Vec<u8>> {
    if !(1..=256).contains(&frame.owner.len()) || frame.data.len() > CHUNK {
        return None;
    }
    let len = HEADER
        .checked_add(frame.owner.len())?
        .checked_add(frame.data.len())?;
    if len > MAX_FRAME {
        return None;
    }
    let mut bytes = Vec::with_capacity(len);
    bytes.extend_from_slice(MAGIC);
    bytes.push(frame.kind);
    bytes.push(0);
    bytes.extend_from_slice(&(frame.owner.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&frame.session.client);
    bytes.extend_from_slice(&frame.session.server);
    bytes.extend_from_slice(&frame.transfer.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(frame.total).ok()?.to_le_bytes());
    bytes.extend_from_slice(&u16::try_from(frame.index).ok()?.to_le_bytes());
    bytes.extend_from_slice(&u16::try_from(frame.count).ok()?.to_le_bytes());
    bytes.extend_from_slice(frame.owner.as_bytes());
    bytes.extend_from_slice(frame.data);
    decode(&bytes)?;
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_binary_boundaries_and_all_truncations() {
        for total in [0, 65_536, 65_537, MAX_PAYLOAD] {
            let input: Vec<u8> = (0..total).map(|n| (n % 256) as u8).collect();
            let mut output = Vec::new();
            for index in 0..fragment_count(total).unwrap() {
                let start = index * CHUNK;
                let bytes = encode(&Frame {
                    kind: DATA,
                    owner: "owner",
                    session: Session {
                        client: [1; 16],
                        server: [2; 16],
                    },
                    transfer: 1,
                    total,
                    index,
                    count: fragment_count(total).unwrap(),
                    data: &input[start..(start + CHUNK).min(total)],
                })
                .unwrap();
                assert!(bytes.len() <= MAX_FRAME);
                let frame = decode(&bytes).unwrap();
                output.extend_from_slice(frame.data);
                for n in [0, 7, 11, 27, 43, 51, 55, 57, 59, bytes.len() - 1] {
                    assert!(decode(&bytes[..n]).is_none());
                }
            }
            assert_eq!(output, input);
        }
        assert_eq!(fragment_count(MAX_PAYLOAD), Some(9));
        assert_eq!(fragment_count(MAX_PAYLOAD + 1), None);
    }
    #[test]
    fn malformed_headers_counts_lengths_nonces_and_owner_bounds_are_refused() {
        let session = Session {
            client: [1; 16],
            server: [2; 16],
        };
        let base = encode(&Frame {
            kind: DATA,
            owner: "a",
            session,
            transfer: 1,
            total: 1,
            index: 0,
            count: 1,
            data: b"x",
        })
        .unwrap();
        for (offset, value) in [(8, 99), (9, 1), (10, 0), (44, 0), (52, 2), (56, 1), (58, 2)] {
            let mut bytes = base.clone();
            bytes[offset] = value;
            assert!(decode(&bytes).is_none(), "offset {offset}");
        }
        for range in [12..28, 28..44] {
            let mut bytes = base.clone();
            bytes[range].fill(0);
            assert!(decode(&bytes).is_none());
        }
        let mut bytes = base.clone();
        bytes.extend_from_slice(b"padding");
        assert!(decode(&bytes).is_none());
        let mut bytes = base.clone();
        bytes[52..56].copy_from_slice(&524289u32.to_le_bytes());
        assert!(decode(&bytes).is_none());
        let owner = "a".repeat(256);
        assert!(
            encode(&Frame {
                kind: DATA,
                owner: &owner,
                session,
                transfer: 1,
                total: 1,
                index: 0,
                count: 1,
                data: b"x"
            })
            .is_some()
        );
        assert!(
            encode(&Frame {
                kind: DATA,
                owner: &"a".repeat(257),
                session,
                transfer: 1,
                total: 1,
                index: 0,
                count: 1,
                data: b"x"
            })
            .is_none()
        );
    }
}
