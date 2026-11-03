//! Frozen AL120-07 observation-only attachment validation, no OS calls.
pub(super) const DLL_SHA256: &str =
    "73e372841dcd5ed763bacec7d6561a10d69cda5eb407646130844bcad2c4be67";
pub(super) const PROFILE_SHA256: &str =
    "177ff857e85bd4f5cb6ca3f6ce3076006304c67c079cb5015cefac06632c26fc";
pub(super) const ABI_SHA256: &str =
    "4bf96ffe72c9ad173e667de31ba2079e08274fedf53fa2f53237bb85615483ca";
pub(super) const EXE_SHA256: &str =
    "d7baa768eb00aa895d9f5a1cd8747ccf66858c7b542f50983092b2e3261588ee";
pub(super) const PACKAGE_ID: &str = "shroudweave-autoloot-owner-probe";
pub(super) const KIND: &str = "autoloot-owner-probe-v1";

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Call {
    pub callsite: u64,
    pub original_entry: u64,
    pub probe_entry: u64,
    pub expected_call: [u8; 5],
    pub reserved: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Attach {
    pub struct_size: u32,
    pub abi_version: u32,
    pub admitted_load_generation: u64,
    pub calls: [Call; 2],
    pub required_host_services: u32,
    pub transfer_capability: u32,
    pub required_capture_fields: u32,
    pub optional_capture_fields: u32,
}

/// Observation of completed host actions, never a retention/held-entry grant.
/// The real Admission and native PairClaim remain the only authority owners.
#[derive(Default)]
pub(super) struct Journal {
    pub retained: bool,
    pub attempted: bool,
    pub prepared: bool,
    pub revoked: bool,
    pub ranges_claimed: bool,
    pub pair_published: bool,
    publication_attempted: bool,
}
impl Journal {
    pub fn begin_prepare(&mut self) -> Result<(), &'static str> {
        if !self.retained || self.attempted || self.revoked {
            return Err("unretained, repeated or revoked preparation");
        }
        self.attempted = true;
        Ok(())
    }
    pub fn finish_prepare(&mut self, success: bool) -> Result<(), &'static str> {
        if !self.attempted || self.prepared || self.revoked || !success {
            return Err("probe preparation refused");
        }
        self.prepared = true;
        Ok(())
    }
    pub fn record_publication(
        &mut self,
        claimed: bool,
        published: bool,
        status: u32,
    ) -> Result<(), &'static str> {
        if self.publication_attempted {
            return Err("publication observation is one-shot");
        }
        self.publication_attempted = true;
        self.ranges_claimed = claimed;
        self.pair_published = published;
        if !self.prepared || self.revoked || !claimed || !published || status != 0 {
            return Err("pair not fully claimed and published");
        }
        Ok(())
    }
    pub fn revoke(&mut self) -> bool {
        let first = !self.revoked;
        self.revoked = true;
        first && self.attempted
    }
    pub fn can_drain(&self) -> bool {
        self.prepared && self.pair_published && !self.revoked
    }
}

// Names and values mirror the immutable imported probe_api_v1.h.
pub(super) const SFAL_PROBE_HOST_PROCESS_RETENTION_V1: u32 = 1;
pub(super) const SFAL_PROBE_HOST_ATOMIC_CALL_PAIR_V1: u32 = 2;
pub(super) const SFAL_PROBE_FIELD_STACK_FRAME_V1: u32 = 1;
pub(super) const SFAL_PROBE_FIELD_EXACT_CALLSITE_V1: u32 = 2;
pub(super) const SFAL_PROBE_FIELD_CALLBACK_CALLER_V1: u32 = 4;
pub(super) const SFAL_PROBE_FIELD_STACK_QUERY_TAGS_V1: u32 = 8;
pub(super) const SFAL_PROBE_FIELD_HOST_CONTEXT_V1: u32 = 16;

pub(super) fn validate_attachment(
    attach: &Attach,
    image: u64,
    module: u64,
    module_size: u64,
    generation: u64,
) -> Result<(), &'static str> {
    if image == 0 || module == 0 || module_size == 0 || generation == 0 {
        return Err("missing actual admitted image/generation");
    }
    let end = module
        .checked_add(module_size)
        .ok_or("module range overflow")?;
    if attach.struct_size != std::mem::size_of::<Attach>() as u32
        || attach.abi_version != 1
        || attach.admitted_load_generation != generation
        || attach.transfer_capability != 0
        || attach.required_host_services
            != SFAL_PROBE_HOST_PROCESS_RETENTION_V1 | SFAL_PROBE_HOST_ATOMIC_CALL_PAIR_V1
        || attach.required_capture_fields
            != SFAL_PROBE_FIELD_STACK_FRAME_V1
                | SFAL_PROBE_FIELD_EXACT_CALLSITE_V1
                | SFAL_PROBE_FIELD_CALLBACK_CALLER_V1
        || attach.optional_capture_fields
            != SFAL_PROBE_FIELD_STACK_QUERY_TAGS_V1 | SFAL_PROBE_FIELD_HOST_CONTEXT_V1
    {
        return Err("probe attachment ABI/owner/capability drift");
    }
    let expected = [
        (0x19fb1e, 0x65cb40, [0xe8, 0x1d, 0xd0, 0x4b, 0]),
        (0x1a020f, 0x1af4a0, [0xe8, 0x8c, 0xf2, 0, 0]),
    ];
    for (call, (site, target, bytes)) in attach.calls.iter().zip(expected) {
        if Some(call.callsite) != image.checked_add(site)
            || Some(call.original_entry) != image.checked_add(target)
            || call.expected_call != bytes
            || call.reserved != [0; 3]
            || call.probe_entry < module
            || call.probe_entry >= end
        {
            return Err("probe callsite/original/entry does not match admitted tuple");
        }
    }
    if attach.calls[0].probe_entry == attach.calls[1].probe_entry {
        return Err("probe entry identities must be distinct");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> Attach {
        let image = 0x140000000;
        Attach {
            struct_size: 96,
            abi_version: 1,
            admitted_load_generation: 9,
            calls: [
                Call {
                    callsite: image + 0x19fb1e,
                    original_entry: image + 0x65cb40,
                    probe_entry: 0x180001000,
                    expected_call: [0xe8, 0x1d, 0xd0, 0x4b, 0],
                    reserved: [0; 3],
                },
                Call {
                    callsite: image + 0x1a020f,
                    original_entry: image + 0x1af4a0,
                    probe_entry: 0x180001100,
                    expected_call: [0xe8, 0x8c, 0xf2, 0, 0],
                    reserved: [0; 3],
                },
            ],
            required_host_services: 3,
            transfer_capability: 0,
            required_capture_fields: 7,
            optional_capture_fields: 24,
        }
    }
    fn accepts(value: &Attach) -> bool {
        validate_attachment(value, 0x140000000, 0x180000000, 0x10000, 9).is_ok()
    }
    #[test]
    fn exact_frozen_observation_tuple_is_accepted() {
        assert!(accepts(&valid()));
    }
    #[test]
    fn abi_owner_service_capture_and_transfer_drift_are_refused() {
        let base = valid();
        for mutate in [
            |v: &mut Attach| v.struct_size = 95,
            |v: &mut Attach| v.abi_version = 2,
            |v: &mut Attach| v.admitted_load_generation = 10,
            |v: &mut Attach| v.required_host_services = 1,
            |v: &mut Attach| v.transfer_capability = 1,
            |v: &mut Attach| v.required_capture_fields = 15,
            |v: &mut Attach| v.optional_capture_fields = 16,
        ] {
            let mut value = base;
            mutate(&mut value);
            assert!(!accepts(&value));
        }
    }
    #[test]
    fn each_native_edge_and_probe_image_is_exact() {
        for i in 0..2 {
            let mut value = valid();
            value.calls[i].callsite += 1;
            assert!(!accepts(&value));
            let mut value = valid();
            value.calls[i].original_entry += 1;
            assert!(!accepts(&value));
            let mut value = valid();
            value.calls[i].expected_call[1] ^= 1;
            assert!(!accepts(&value));
            let mut value = valid();
            value.calls[i].reserved[0] = 1;
            assert!(!accepts(&value));
            let mut value = valid();
            value.calls[i].probe_entry = 0x140001000;
            assert!(!accepts(&value));
        }
        let mut value = valid();
        value.calls[1].probe_entry = value.calls[0].probe_entry;
        assert!(!accepts(&value));
        assert!(validate_attachment(&valid(), 0, 0x180000000, 0x10000, 9).is_err());
        assert!(validate_attachment(&valid(), 0x140000000, u64::MAX - 1, 0x10000, 9).is_err());
        assert!(validate_attachment(&valid(), 0x140000000, 0x180000000, 0x10000, 0).is_err());
    }
    #[test]
    fn imported_call_and_attach_abi_layouts_match() {
        assert_eq!(std::mem::size_of::<Call>(), 32);
        assert_eq!(std::mem::size_of::<Attach>(), 96);
    }
    #[test]
    fn preparation_refusal_and_revoke_never_clear_retention_or_allow_retry() {
        let mut journal = Journal::default();
        assert!(journal.begin_prepare().is_err());
        journal.retained = true;
        journal.begin_prepare().unwrap();
        assert!(journal.finish_prepare(false).is_err());
        assert!(journal.revoke());
        assert!(!journal.revoke());
        assert!(journal.retained);
        assert!(!journal.can_drain());
        assert!(journal.begin_prepare().is_err());
    }
    #[test]
    fn atomic_refusal_and_indeterminate_keep_claim_and_publication_distinct() {
        for (claimed, published, status) in [(false, false, 1), (true, false, 1), (true, true, 2)] {
            let mut journal = Journal {
                retained: true,
                ..Journal::default()
            };
            journal.begin_prepare().unwrap();
            journal.finish_prepare(true).unwrap();
            assert!(
                journal
                    .record_publication(claimed, published, status)
                    .is_err()
            );
            assert!(journal.record_publication(true, true, 0).is_err());
            assert_eq!(journal.ranges_claimed, claimed);
            assert_eq!(journal.pair_published, published);
            assert!(journal.revoke());
            assert!(journal.retained);
            assert!(!journal.can_drain());
            assert!(journal.begin_prepare().is_err());
        }
    }
    #[test]
    fn successful_observation_stop_revokes_once_without_unload_or_republication() {
        let mut journal = Journal {
            retained: true,
            ..Journal::default()
        };
        journal.begin_prepare().unwrap();
        journal.finish_prepare(true).unwrap();
        journal.record_publication(true, true, 0).unwrap();
        assert!(journal.can_drain());
        assert!(journal.revoke());
        assert!(!journal.revoke());
        assert!(!journal.can_drain());
        assert!(journal.retained && journal.ranges_claimed && journal.pair_published);
        assert!(journal.record_publication(true, true, 0).is_err());
    }
}
