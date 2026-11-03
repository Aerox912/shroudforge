// SPDX-License-Identifier: MIT
#pragma once
#include <stddef.h>
#include <stdint.h>
#if defined(_WIN32)
#define SFAL_PROBE_CALL __cdecl
#else
#define SFAL_PROBE_CALL
#endif
#if defined(_WIN32) && defined(ALPROBE_BUILD)
#define SFAL_PROBE_API __declspec(dllexport)
#else
#define SFAL_PROBE_API
#endif
#define SFAL_PROBE_ABI_V1 1u
#define SFAL_PROBE_MAX_RECORDS_V1 256u

// AL120-07 diagnostic only. This is a PROPOSED host integration contract, not
// an existing ShroudForge service. No function confers inventory authority.
typedef enum SFALProbeSiteV1 { SFAL_PROBE_BUILDER_V1=1, SFAL_PROBE_STOCK_V1=2 } SFALProbeSiteV1;
typedef enum SFALProbePhaseV1 { SFAL_PROBE_PRE_V1=0, SFAL_PROBE_POST_V1=1 } SFALProbePhaseV1;
typedef enum SFALProbeHostServiceBitsV1 {
    SFAL_PROBE_HOST_PROCESS_RETENTION_V1=1u, SFAL_PROBE_HOST_ATOMIC_CALL_PAIR_V1=2u
} SFALProbeHostServiceBitsV1;
typedef enum SFALProbeCaptureFieldBitsV1 {
    SFAL_PROBE_FIELD_STACK_FRAME_V1=1u, SFAL_PROBE_FIELD_EXACT_CALLSITE_V1=2u,
    SFAL_PROBE_FIELD_CALLBACK_CALLER_V1=4u, SFAL_PROBE_FIELD_STACK_QUERY_TAGS_V1=8u,
    SFAL_PROBE_FIELD_HOST_CONTEXT_V1=16u
} SFALProbeCaptureFieldBitsV1;
typedef enum SFALProbeContextBitsV1 {
    SFAL_PROBE_CONTEXT_DISPATCHER_V1=1u, SFAL_PROBE_CONTEXT_ACCESS_SET_V1=2u,
    SFAL_PROBE_CONTEXT_WORLD_V1=4u, SFAL_PROBE_CONTEXT_SOURCE_LIFETIME_V1=8u,
    SFAL_PROBE_CONTEXT_RECIPIENT_LIFETIME_V1=16u, SFAL_PROBE_CONTEXT_CHUNK_TAGS_V1=32u,
    SFAL_PROBE_CONTEXT_GATE_SAMPLES_V1=64u, SFAL_PROBE_CONTEXT_WORLD_CARRIER_V1=128u
} SFALProbeContextBitsV1;
typedef enum SFALProbeRecordFlagsV1 {
    SFAL_PROBE_RECORD_FRAME_VERIFIED_V1=1u, SFAL_PROBE_RECORD_CALLER_AVAILABLE_V1=2u,
    SFAL_PROBE_RECORD_QUERY_TAGS_AVAILABLE_V1=4u, SFAL_PROBE_RECORD_SOURCE_RECIPIENT_TAGS_V1=8u,
    SFAL_PROBE_RECORD_TRANSACTION_FLAGS_V1=16u, SFAL_PROBE_RECORD_JOURNAL_COUNT_V1=32u,
    SFAL_PROBE_RECORD_RESULT_STATUS_V1=64u, SFAL_PROBE_RECORD_HOST_CONTEXT_AVAILABLE_V1=128u,
    SFAL_PROBE_RECORD_HOST_CONTEXT_UNAVAILABLE_V1=256u, SFAL_PROBE_RECORD_CALLER_OUTSIDE_PROFILE_V1=512u
} SFALProbeRecordFlagsV1;
typedef enum SFALProbePrepareStatusV1 {
    SFAL_PROBE_PREPARED_V1=0, SFAL_PROBE_INVALID_REQUEST_V1=1,
    SFAL_PROBE_INVALID_IMAGE_IDENTITY_V1=2, SFAL_PROBE_DIFFERENT_OWNER_V1=3,
    SFAL_PROBE_PROFILE_REFUSED_V1=4, SFAL_PROBE_RANDOM_UNAVAILABLE_V1=5,
    SFAL_PROBE_STATE_REFUSED_V1=6, SFAL_PROBE_PREPARING_V1=7
} SFALProbePrepareStatusV1;
typedef struct SFALProbeFrameV1 {
    uint64_t native_rsp, native_rbp, native_return_pc, diagnostic_token;
    uint64_t arg_rcx, arg_rdx, arg_r8, arg_r9;
    uint64_t stack_args[4];
    uint64_t native_r12, native_r15;
} SFALProbeFrameV1;

// Copied observations, NOT grants. available_fields identifies actual sampled
// fields. Missing native context remains missing and never becomes capability On.
// Address/Steam/world identity fields MUST be process-local opaque tags; no raw
// pointer or Steam ID is allowed in output. The callback receives a transient
// native frame and must not retain it or invoke network/settings/logging APIs.
typedef struct SFALProbeContextV1 {
    uint32_t struct_size, available_fields;
    uint64_t dispatcher_tag, access_set_tag, scene_tag, world_tag;
    uint64_t source_chunk_tag, recipient_chunk_tag;
    uint32_t source_generation, recipient_generation;
    uint32_t scene_gate_value, registry_gate_value;
    uint64_t native_phase_tag, world_carrier_tag;
} SFALProbeContextV1;
typedef uint32_t (SFAL_PROBE_CALL *SFALProbeCopyContextV1)(
    void* retained_context, uint32_t site, uint32_t phase,
    const SFALProbeFrameV1* transient_frame, SFALProbeContextV1* copied_output);

typedef struct SFALProbeHostV1 {
    uint32_t struct_size, abi_version;
    uint64_t admitted_module, executable_image, admitted_load_generation;
    // Process-retained native context; nullable until SF implements/reviews the
    // callback-safe sampler. Supplying it does not assert writer ownership.
    void* retained_context;
    SFALProbeCopyContextV1 copy_context;
    uint64_t reserved[2];
} SFALProbeHostV1;
typedef struct SFALProbeCallV1 {
    uint64_t callsite, original_entry, probe_entry;
    uint8_t expected_call[5];
    uint8_t reserved[3];
} SFALProbeCallV1;
typedef struct SFALProbeAttachV1 {
    uint32_t struct_size, abi_version;
    uint64_t admitted_load_generation;
    SFALProbeCallV1 calls[2];
    uint32_t required_host_services;
    uint32_t transfer_capability; // ALWAYS zero, including an installed probe.
    uint32_t required_capture_fields, optional_capture_fields;
} SFALProbeAttachV1;
typedef struct SFALProbeStatsV1 {
    uint32_t struct_size, transfer_capability;
    uint64_t builder_pre_hits,builder_post_hits,stock_pre_hits,stock_post_hits;
    uint64_t dropped;
    uint32_t capture_enabled, reserved;
} SFALProbeStatsV1;

typedef struct SFALProbeRecordV1 {
    // Consult flags before reading optional fields. PRE stock records do NOT
    // read/claim result, journal count, or transaction flags. POST stock samples
    // are before the original caller's commit and are never a transfer ACK.
    uint32_t struct_size, site, phase, flags;
    uint64_t sequence, diagnostic_token;
    uint32_t native_caller_rva, callback_caller_rva;
    uint64_t thread_tag, outer_tag, inner_tag;
    uint64_t source_tag, recipient_tag;
    uint64_t query_descriptor_tags[8];
    uint64_t journal_count;
    uint32_t transaction_flags, native_result_status;
    SFALProbeContextV1 context;
} SFALProbeRecordV1;

// Preparation only. No VirtualProtect, executable/data patch, MinHook, module
// pin or worker thread. SF must own and independently admit the retained two-
// CALL publication transaction and the actual loader-bound module generation.
// Publication before successful preparation is forbidden. Host keeps originals,
// DLL mapping, native state and optional sampler alive for process lifetime.
// Raw admitted_module/load-generation values are identity inputs, not permission.
// SF must bind them to its actual admitted image and generation before calling.
// Failure leaves output untouched; no old output is usable after a failure.
// Same owner/thread replays its exact prepared tuple or recorded refusal without
// rebinding originals/re-enabling capture. Different owner/context/generation
// refuses. No restoration/unload/reset API is provided.
#ifdef __cplusplus
extern "C" {
#endif
SFAL_PROBE_API uint32_t SFAL_PROBE_CALL SFALProbePrepareAttachV1(const SFALProbeHostV1*, SFALProbeAttachV1*);
SFAL_PROBE_API uint32_t SFAL_PROBE_CALL SFALProbeDrainV1(uint32_t capacity, SFALProbeRecordV1*, uint32_t* copied, uint64_t* dropped);
SFAL_PROBE_API void SFAL_PROBE_CALL SFALProbeRevokeCaptureV1(void);
SFAL_PROBE_API uint32_t SFAL_PROBE_CALL SFALProbeReadStatsV1(SFALProbeStatsV1*);
#ifdef __cplusplus
}
static_assert(sizeof(SFALProbeFrameV1)==0x70);
static_assert(offsetof(SFALProbeFrameV1,native_r12)==0x60);
static_assert(sizeof(SFALProbeCallV1)==0x20);
static_assert(sizeof(SFALProbeAttachV1)==0x60);
static_assert(sizeof(SFALProbeHostV1)==0x40);
static_assert(sizeof(SFALProbeContextV1)==0x58);
static_assert(sizeof(SFALProbeRecordV1)==0xf8);
static_assert(sizeof(SFALProbeStatsV1)==0x38);
#endif
