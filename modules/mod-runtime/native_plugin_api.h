#ifndef SHROUDFORGE_NATIVE_PLUGIN_API_H
#define SHROUDFORGE_NATIVE_PLUGIN_API_H

#include <stdint.h>

#ifndef SF_NATIVE_PLUGIN_EXPORT
#if defined(_WIN32)
#define SF_NATIVE_PLUGIN_EXPORT __declspec(dllexport)
#else
#define SF_NATIVE_PLUGIN_EXPORT
#endif
#endif

#if defined(_MSC_VER)
#define SF_NATIVE_CALL __cdecl
#else
#define SF_NATIVE_CALL
#endif

#ifdef __cplusplus
extern "C" {
#define SF_NATIVE_EXTERN_C extern "C"
#else
#define SF_NATIVE_EXTERN_C extern
#endif

#define SF_NATIVE_PLUGIN_ABI_V1 1u
#define SF_NATIVE_HOST_ABI_V1 1u
#define SF_NATIVE_MAX_SETTINGS_JSON_BYTES 65536u
#define SF_NATIVE_MAX_ASSET_PATH_BYTES 1024u
#define SF_NATIVE_MAX_ASSET_BYTES 16777216u
#define SF_NATIVE_MAX_LOG_BYTES 4096u
#define SF_NATIVE_DRAIN_TIMEOUT_MS 250u
#define SF_NATIVE_SERVICE_MAP_V1 "shroudforge.map"
#define SF_NATIVE_SERVICE_OVERLAY_V1 "shroudforge.overlay"
#define SF_NATIVE_SERVICE_NOTICES_V1 "shroudforge.notices"
#define SF_NOTICE_MAX_UTF8_BYTES 256u
#define SF_NOTICE_DURATION_MS 3000u
/* Copied owner-bound shroudforge.notices v1 table. show is synchronous on the
 * thread that queried the table. UTF-8 must be nonempty, <=256 bytes and contain
 * no ASCII controls. Fixed appearance/duration, latest message globally wins.
 * Success accepts a transient publication, not proof of on-screen presentation.
 * No pointer is retained. Revocation closes show and drain fences render reads.
 * This public SDK header is standalone, with only the standard stdint.h needed.
 */
typedef struct SFNoticeServiceV1 {
    uint32_t size,version;
    void* context;
    int32_t (*show)(void*,const uint8_t*,uint32_t);
} SFNoticeServiceV1;

typedef int32_t SFNativeStatusV1;
typedef uint32_t SFNativeTargetV1;
typedef uint32_t SFNativeLogLevelV1;

#define SF_NATIVE_STATUS_OK ((SFNativeStatusV1)0)
#define SF_NATIVE_STATUS_INVALID_ARGUMENT ((SFNativeStatusV1)1)
#define SF_NATIVE_STATUS_ABI_MISMATCH ((SFNativeStatusV1)2)
#define SF_NATIVE_STATUS_UNAVAILABLE ((SFNativeStatusV1)3)
#define SF_NATIVE_STATUS_UNKNOWN_SERVICE ((SFNativeStatusV1)4)
#define SF_NATIVE_STATUS_BUFFER_TOO_SMALL ((SFNativeStatusV1)5)
#define SF_NATIVE_STATUS_BUSY ((SFNativeStatusV1)6)
#define SF_NATIVE_STATUS_FAILED ((SFNativeStatusV1)7)
#define SF_NATIVE_STATUS_WRONG_THREAD ((SFNativeStatusV1)8)
#define SF_NATIVE_STATUS_NOT_FOUND ((SFNativeStatusV1)9)
#define SF_NATIVE_STATUS_TOO_LARGE ((SFNativeStatusV1)10)
#define SF_NATIVE_STATUS_CONFLICT ((SFNativeStatusV1)11)
#define SF_NATIVE_STATUS_RESTART_REQUIRED ((SFNativeStatusV1)12)

#define SF_NATIVE_TARGET_CLIENT ((SFNativeTargetV1)1u)
#define SF_NATIVE_TARGET_SERVER ((SFNativeTargetV1)2u)

/* Explicit, irreversible host lifetime, Windows lifecycle plugins only.
 * Query and retain run only during initialize on its runtime owner thread.
 * Query alone retains nothing. OK from retain means the host committed this
 * exact package generation to process lifetime; an exact-owner repeat is OK.
 * It writes the complete result only on OK; errors leave output untouched.
 * out_size must fit SFProcessLifetimeResultV1. No release/reset operation.
 *
 * After retention, teardown still revokes host services, drains and attempts
 * stop. It never calls shutdown or unloads the plugin, including on partial
 * initialize failure. Another lifecycle load of the package is refused before
 * LoadLibrary/entry across AppStates, including different DLL/temp paths.
 * The admitted DLL and the providing SF image are pinned. This registry is
 * shared within that pinned SF image; independently mapped SF copies and other
 * loaders are not coordinated.
 *
 * Producer requirements: DllMain and ShroudForgePluginEntry must be inert.
 * Request retention in initialize BEFORE starting workers or installing hooks.
 * Keep a concrete process-lifetime STATE and its code/dependencies alive; stop
 * must revoke effects/intents, stop workers where possible and preserve all
 * state reachable by late native paths. Never use revoked host services.
 * A retained plugin_context pointer does not adopt arbitrary heap allocations
 * or prevent producer code from freeing them. This service owns no hook range,
 * IAT/Vulkan/HWND chain, input capture, ECS lease or quiescent-unload proof.
 * retained/restart_required describes host lifetime, not feature readiness.
 */
#define SF_NATIVE_SERVICE_PROCESS_LIFETIME_V1 "shroudforge.process-lifetime"
typedef struct SFProcessLifetimeResultV1 {
    uint32_t size;
    uint32_t version;
    uint64_t package_generation;
    uint32_t retained;
    uint32_t restart_required;
} SFProcessLifetimeResultV1;
typedef SFNativeStatusV1(SF_NATIVE_CALL *SFProcessLifetimeRetainFnV1)(
    void *context, uint32_t out_size, SFProcessLifetimeResultV1 *out_result);
typedef struct SFProcessLifetimeServiceV1 {
    uint32_t size;
    uint32_t version;
    void *context;
    SFProcessLifetimeRetainFnV1 retain_until_process_exit;
} SFProcessLifetimeServiceV1;

#ifdef __cplusplus
#define SF_OWNER_ADOPTION_NOEXCEPT noexcept
#else
#define SF_OWNER_ADOPTION_NOEXCEPT
#endif
#define SF_NATIVE_SERVICE_OWNER_ADOPTION_V1 "shroudforge.owner-adoption"
#define SF_OWNER_ADOPTION_VERSION_V1 1u

typedef uint64_t SFOwnerAdoptionHandleV1;
#define SF_OWNER_ADOPTION_NO_HANDLE ((SFOwnerAdoptionHandleV1)0u)

#define SF_OWNER_ADOPTION_PHASE_VACANT 0u
#define SF_OWNER_ADOPTION_PHASE_ACQUIRING 1u
#define SF_OWNER_ADOPTION_PHASE_ADOPTED 2u
#define SF_OWNER_ADOPTION_PHASE_INDETERMINATE 3u
#define SF_OWNER_ADOPTION_PHASE_REVOKED 4u

#define SF_OWNER_CLOSE_NOT_ATTEMPTED 0u
#define SF_OWNER_CLOSE_IN_FLIGHT 1u
#define SF_OWNER_CLOSE_CLOSED 2u
#define SF_OWNER_CLOSE_INDETERMINATE 3u

/* Identity/correlation only. No CALL claim, publisher token or native authority.
 * The host owns this temporary immutable binding. Callbacks retain no pointer
 * to it and copy values into their concrete strong lease if needed. */
typedef struct SFOwnerAdoptionBindingV1 {
    uint32_t size;
    uint32_t version;
    uint64_t package_generation;
    SFOwnerAdoptionHandleV1 owner_handle;
    const void *state_identity;
} SFOwnerAdoptionBindingV1;

/* Returned ownership unit, not an opaque STATE pointer presented as ownership.
 * owned_lease identifies a concrete producer-defined owned strong-reference
 * unit that independently retains the exact state and all code/state leases
 * needed by close_content. Its opaque representation may alias a state/source
 * address only if acquire actually acquires an independent strong ownership unit.
 * Host invokes no destructor/release: this ownership unit lasts until process
 * exit. Complete valid returned descriptors transfer to the host even if the
 * callback's status reports failure. Malformed/partial descriptors are only
 * quarantined, never dereferenced or treated as a validated strong lease.
 */
typedef struct SFOwnerStrongLeaseV1 {
    uint32_t size;
    uint32_t version;
    uint64_t package_generation;
    SFOwnerAdoptionHandleV1 owner_handle;
    const void *state_identity;
    void *owned_lease;
} SFOwnerStrongLeaseV1;

/* Both thunks must be in the Admission's pinned verified primary DLL and in
 * executable committed image memory. Module identity is derived by the host.
 * They run only on its creator thread, with no host lock/RefCell borrow held.
 * All exceptions must be caught inside the producer thunk. C++ builds require
 * noexcept. An exception/SEH escape across this ABI is a contract violation,
 * not something Rust catch_unwind can qualify or recover.
 *
 * acquire only acquires a fresh strong ownership unit. It must not enable content, call
 * the private full-native create_retained path, publish or invoke native ops.
 * source_context is a borrowed producer guard/factory, valid synchronously.
 * The host zero-initializes lease output; the thunk writes one complete V1
 * descriptor when it has a concrete ownership unit. No callback retains host
 * input/output pointers or frees any returned lease after this attempt.
 */
typedef SFNativeStatusV1(SF_NATIVE_CALL *SFOwnerAcquireStrongLeaseFnV1)(
    void *source_context,
    const SFOwnerAdoptionBindingV1 *binding,
    uint32_t out_lease_size,
    SFOwnerStrongLeaseV1 *out_lease) SF_OWNER_ADOPTION_NOEXCEPT;

/* close only fences this consumer's content/intents. It never destroys the
 * strong lease/state, releases code, restores bytes, unloads or reclaims.
 * OK means the consumer gate closed. Error/ambiguity is terminal: no retry.
 */
typedef SFNativeStatusV1(SF_NATIVE_CALL *SFOwnerCloseContentFnV1)(
    void *owned_lease,
    const SFOwnerAdoptionBindingV1 *binding) SF_OWNER_ADOPTION_NOEXCEPT;

typedef struct SFOwnerAdoptionRequestV1 {
    uint32_t size;
    uint32_t version;
    const void *state_identity;
    void *source_context;
    SFOwnerAcquireStrongLeaseFnV1 acquire_strong_lease;
    SFOwnerCloseContentFnV1 close_content;
} SFOwnerAdoptionRequestV1;

/* host_fenced is the host's irreversible gate, not proof of consumer drain.
 * close_state distinguishes actual close callback outcome. lease_retained=1
 * requires a complete validated strong-lease descriptor, including identity
 * echoes. native_operations_available is always zero for this entire service.
 */
typedef struct SFOwnerAdoptionStateV1 {
    uint32_t size;
    uint32_t version;
    uint64_t package_generation;
    SFOwnerAdoptionHandleV1 owner_handle;
    const void *state_identity;
    uint32_t phase;
    uint32_t host_fenced;
    uint32_t lease_retained;
    uint32_t close_state;
    uint32_t native_operations_available;
    SFNativeStatusV1 last_callback_status;
} SFOwnerAdoptionStateV1;

/* Query AND adopt are initialize-only and rechecked on each invocation:
 * exact creator-thread, live SDK host, exposed host bound to the same retained
 * verified-preload Adopted Admission. A captured table cannot adopt later.
 * Only one attempt/consumer is available per Admission. No second root exists.
 *
 * adopt: request_size and request.size must equal sizeof(RequestV1), version=1.
 * out_size must fit StateV1. Every precommit refusal leaves output untouched
 * and invokes no callback. Before invoking acquire, the host permanently
 * latches its preallocated bounded record/handle. No automatic retry follows.
 * After that latch, all recoverable failure/revoke/panic paths return
 * SF_NATIVE_STATUS_RESTART_REQUIRED with a complete INDETERMINATE state.
 * A valid lease is retained even on error, but never labelled ADOPTED then.
 * Only OK plus a complete matching ADOPTED receipt permits the caller to drop
 * its separate ambiguity guard. All ambiguous outcomes keep that guard forever.
 *
 * read_status: captured table + exact handle works on the creator thread after
 * initialize/stop/revoke to inspect this retained record. Errors preserve output.
 * While an external callback is active, it returns BUSY, never partial state.
 * It does not allocate/adopt, invoke producer callbacks or confer authority.
 *
 * revoke: captured table + exact handle may fence after initialize, including
 * after normal host-service revocation. It latches host_fenced before callback
 * work. It invokes close at most once for a validated lease on creator thread.
 * Reentrant revoke latches the fence and returns BUSY; outer processing settles
 * close. OK means host fenced and close returned OK. Indeterminate returns
 * RESTART_REQUIRED. No callback retry, release, restore or reactivation exists.
 * Host-internal revoke applies the same gate even without a consumer call.
 */
typedef struct SFOwnerAdoptionServiceV1 {
    uint32_t size;
    uint32_t version;
    void *context;
    SFNativeStatusV1(SF_NATIVE_CALL *adopt)(
        void *context,
        uint32_t request_size,
        const SFOwnerAdoptionRequestV1 *request,
        uint32_t out_size,
        SFOwnerAdoptionStateV1 *out_state);
    SFNativeStatusV1(SF_NATIVE_CALL *read_status)(
        void *context,
        SFOwnerAdoptionHandleV1 owner_handle,
        uint32_t out_size,
        SFOwnerAdoptionStateV1 *out_state);
    SFNativeStatusV1(SF_NATIVE_CALL *revoke)(
        void *context,
        SFOwnerAdoptionHandleV1 owner_handle);
} SFOwnerAdoptionServiceV1;



/* Owner-bound Steam Networking Messages for the querying package only. This
 * table is available to Runtime-capable packages in either process target;
 * transport readiness is reported by read_status and is not a query-time
 * requirement. All callbacks are synchronous on the querying runtime thread.
 * No package ID or destination Steam ID is accepted from the caller. */
#define SF_NATIVE_SERVICE_NETWORK_V1 "shroudforge.mod-network"
#define SF_NATIVE_MAX_NETWORK_PAYLOAD_BYTES 65536u
#define SF_NATIVE_NETWORK_MESSAGE_RELIABLE 1u

typedef struct SFModNetworkStatusV1 {
    uint32_t size;
    uint32_t version;
    SFNativeTargetV1 process_target;
    uint32_t available;
    uint32_t service_ready;
    uint32_t reserved;
    uint64_t local_steam_id;
    uint64_t connected_dedicated_server_steam_id;
} SFModNetworkStatusV1;

typedef struct SFModNetworkMessageV1 {
    uint32_t size;
    uint32_t version;
    uint64_t peer_steam_id;
    uint32_t payload_bytes;
    uint32_t flags;
    uint64_t reply_token;
} SFModNetworkMessageV1;

typedef SFNativeStatusV1 (SF_NATIVE_CALL *SFModNetworkReadStatusFnV1)(
    void *context,
    uint32_t out_size,
    SFModNetworkStatusV1 *out_status);
typedef SFNativeStatusV1 (SF_NATIVE_CALL *SFModNetworkSendToServerFnV1)(
    void *context,
    const uint8_t *payload_utf8,
    uint32_t payload_length);
typedef SFNativeStatusV1 (SF_NATIVE_CALL *SFModNetworkReceiveFnV1)(
    void *context,
    uint8_t *out_payload_utf8,
    uint32_t capacity,
    uint32_t metadata_size,
    SFModNetworkMessageV1 *out_metadata);
typedef SFNativeStatusV1 (SF_NATIVE_CALL *SFModNetworkReplyFnV1)(
    void *context,
    uint64_t reply_token,
    const uint8_t *payload_utf8,
    uint32_t payload_length);

/* The envelope is shroudforge.mod-message.v1 with from_mod and to_mod both
 * bound to this package. send_to_server targets only the current connected
 * Dedicated Server; receive on a client accepts only that server, while a
 * server accepts only currently authenticated and Network-authorized peers.
 * The payload is UTF-8, never NUL-terminated, and at most 64 KiB. The final
 * serialized envelope remains subject to the transport's 512 KiB limit.
 * Caller buffers are copied synchronously and no caller pointer is retained.
 * receive consumes at most one frame. BUFFER_TOO_SMALL reports payload_bytes
 * in metadata, leaves the payload buffer untouched, consumes that one frame,
 * and returns no reply token. Other errors preserve all output bytes. A reply
 * token routes only to the peer from this owner's latest accepted receive and
 * is invalidated by the next receive, a relevant status peer change, or
 * revocation. reply rechecks the current client-server or server-peer gate.
 * Network permission is initialized from validated loader settings when the
 * plugin loads and refreshed by the runtime status/config refresh, scheduled
 * about once per second. A setting change or config read/validation failure
 * becomes visible at that refresh; native plugin callbacks in the current
 * update tick run before it. A failed config read makes this service
 * unavailable until a later successful refresh. Lua networking keeps its
 * existing per-call settings checks. */
typedef struct SFModNetworkServiceV1 {
    uint32_t size;
    uint32_t version;
    void *context;
    uint32_t max_payload_bytes;
    uint32_t reserved;
    SFModNetworkReadStatusFnV1 read_status;
    SFModNetworkSendToServerFnV1 send_to_server;
    SFModNetworkReceiveFnV1 receive;
    SFModNetworkReplyFnV1 reply;
} SFModNetworkServiceV1;

/* Query shroudforge.mod-network at requested_version=2. V1 is unchanged.
 * Generic logical UTF-8 payload, 0..524288 bytes, including NUL/control bytes.
 * Owner thread only. Copies synchronously; retains no caller pointers.
 * Both endpoints must opt into V2. begin_session explicitly resets a client's
 * logical session and sends one HELLO to its currently host-selected dedicated
 * server. receive advances HELLO/ACK/DATA; no worker, retries or auto replay.
 * read_status does not pump. logical_ready counts negotiated peer sessions,
 * pending_sessions counts pending HELLOs and logical_peers counts metadata.
 * Transport readiness and logical readiness differ. All V2 control/DATA frames
 * must be marked reliable by the transport; unreliable frames are refused.
 * reset retires this owner's sessions, assembly/completion, queue and reply
 * route, and invalidates any reentrant in-flight call. No game callback needed.
 * It is valid while disabled. It does not reopen revoked services.
 *
 * Send result: invalid out_size/out_result preserves output and submits nothing.
 * Otherwise the complete result is written on every return, including errors.
 * attempted_frames increments BEFORE each native send; accepted_frames counts
 * locally successful submissions, including one followed by revocation.
 * delivery_uncertain = attempted_frames != 0, even on OK. OK is local submission
 * only, never delivery, permission, game/World readiness or application effect.
 * Zero attempted_frames proves only this operation submitted nothing.
 * A batch stops on the first failure/revocation; nothing is retried/replayed.
 *
 * receive consumes at most one physical frame and may send one automatic ACK.
 * NOT_FOUND includes negotiation/incomplete/ignored/malformed input. No outputs
 * change on errors except BUFFER_TOO_SMALL with valid metadata: required bytes,
 * peer/session correlation, reliable flag and reply_token=0 are written, payload
 * stays untouched and the completed message survives for bounded retry.
 * A reply token is issued only after the full payload is copied. One latest
 * route per owner, bound to generation, peer and paired session nonces. The next
 * admitted V2 receive, reset, policy transition/revoke, observed identity change,
 * peer deauthorization or session expiry retires it. reply rechecks all gates.
 * Session IDs, Steam IDs and nonces are correlation only. Same-SteamID game
 * reconnect is unobservable. No game connection incarnation or World proof.
 *
 * Existing mod_channel(owner), full owner ID demux. Raw/Lua/V1 traffic retains
 * original bytes; only registered owners reserve V2 prefixes. Shared queue:
 * <=128 owners, <=256 packets, <=32 per route, <=4MiB; overflow drops newest.
 * Queued packets expire lazily after30s. dropped_queue_frames is a process-wide
 * saturating count of overflow/expiry/in-flight retirement drops, not delivery proof.
 * Per owner: <=64 peers, <=4 retained payloads, <=2MiB; globally <=16MiB payload.
 * One assembly/completion per peer. Newer transfers replace incomplete ones;
 * an undelivered completion wins and newer transfer IDs are retired/refused.
 * Completed/aborted/refused IDs cannot resurrect in their logical session.
 * Identical duplicates do not renew expiry; conflicting duplicates abort.
 * Lazy monotonic expiry: pending HELLO5s, assembly5s since new progress and30s
 * absolute, completion30s since completion, logical session60s idle. No timers.
 * These bounded queues/expiry/backpressure can lose messages. No exactly-once
 * guarantee. See the pinned task contract for the complete binary wire layout.
 */
#define SF_NATIVE_SERVICE_NETWORK_V2 SF_NATIVE_SERVICE_NETWORK_V1
#define SF_NATIVE_MAX_NETWORK_PAYLOAD_BYTES_V2 524288u
#define SF_NATIVE_MAX_NETWORK_FRAGMENT_BYTES_V2 61440u
typedef struct SFModNetworkSendResultV2 {
    uint32_t size, version, attempted_frames, accepted_frames;
    uint32_t delivery_uncertain, reserved;
} SFModNetworkSendResultV2;
typedef struct SFModNetworkStatusV2 {
    uint32_t size, version;
    SFNativeTargetV1 process_target;
    uint32_t available, service_ready, logical_ready, pending_sessions, logical_peers;
    uint64_t local_steam_id, connected_dedicated_server_steam_id, dropped_queue_frames;
} SFModNetworkStatusV2;
typedef struct SFModNetworkMessageV2 {
    uint32_t size, version;
    uint64_t peer_steam_id;
    uint32_t payload_bytes, flags;
    uint64_t reply_token, logical_session_id;
} SFModNetworkMessageV2;
typedef struct SFModNetworkServiceV2 {
    uint32_t size, version;
    void *context;
    uint32_t max_payload_bytes, max_fragment_bytes;
    SFNativeStatusV1(SF_NATIVE_CALL *read_status)(void*, uint32_t, SFModNetworkStatusV2*);
    SFNativeStatusV1(SF_NATIVE_CALL *begin_session)(void*, uint32_t, SFModNetworkSendResultV2*);
    SFNativeStatusV1(SF_NATIVE_CALL *reset)(void*);
    SFNativeStatusV1(SF_NATIVE_CALL *send_to_server)(void*, const uint8_t*, uint32_t,
        uint32_t, SFModNetworkSendResultV2*);
    SFNativeStatusV1(SF_NATIVE_CALL *receive)(void*, uint8_t*, uint32_t,
        uint32_t, SFModNetworkMessageV2*);
    SFNativeStatusV1(SF_NATIVE_CALL *reply)(void*, uint64_t, const uint8_t*, uint32_t,
        uint32_t, SFModNetworkSendResultV2*);
} SFModNetworkServiceV2;

#define SF_NATIVE_LOG_ERROR ((SFNativeLogLevelV1)1u)
#define SF_NATIVE_LOG_WARN ((SFNativeLogLevelV1)2u)
#define SF_NATIVE_LOG_INFO ((SFNativeLogLevelV1)3u)
#define SF_NATIVE_LOG_DEBUG ((SFNativeLogLevelV1)4u)
#define SF_NATIVE_LOG_TRACE ((SFNativeLogLevelV1)5u)

/* Named settings use the existing SDL capture/Hotkey Manager. No VK codes.
 * Only declared control:keybind settings of this package are accessible.
 * read_keybind runs on the owner thread, is revoked with the host, and consumes
 * press edges once per setting. First read after load/rebind establishes a
 * baseline. Empty/disabled settings report zero flags. Focus loss and capture
 * suppress both flags while consuming revisions. Tables expire with the host.
 * out_size must fit SFKeybindStateV1; no output is written on errors.
 */
#define SF_NATIVE_SERVICE_KEYBINDS_V1 "shroudforge.keybinds"
#define SF_NATIVE_SERVICE_KEYBINDS_V2 SF_NATIVE_SERVICE_KEYBINDS_V1
#define SF_KEYBIND_DOWN 1u
#define SF_KEYBIND_PRESSED 2u
typedef struct SFKeybindStateV1 {
    uint32_t size;
    uint32_t flags;
} SFKeybindStateV1;
typedef struct SFKeybindServiceV1 {
    uint32_t size;
    uint32_t version;
    void *context;
    SFNativeStatusV1(SF_NATIVE_CALL *read_keybind)(void *context,
        const uint8_t *setting_key_utf8, uint32_t setting_key_length,
        uint32_t out_size, SFKeybindStateV1 *out_state);
} SFKeybindServiceV1;

/* Version 2 reports focus and Hotkey Manager capture separately. ACCEPTED
 * means only that the host accepts this client input based on those states.
 * It does not mean gameplay is active or authorize a game action. UNKNOWN
 * means a state could not be read. UNKNOWN and BLOCKED both consume press
 * edges and return no DOWN/PRESSED flags, so an edge cannot replay later. */
#define SF_INPUT_STATUS_UNKNOWN 0u
#define SF_INPUT_STATUS_INACTIVE 1u
#define SF_INPUT_STATUS_ACTIVE 2u
#define SF_INPUT_ADMISSION_UNKNOWN 0u
#define SF_INPUT_ADMISSION_BLOCKED 1u
#define SF_INPUT_ADMISSION_ACCEPTED 2u
typedef struct SFKeybindStateV2 {
    uint32_t size;
    uint32_t flags;
    uint32_t focus_status;
    uint32_t capture_status;
    uint32_t admission_status;
} SFKeybindStateV2;
typedef struct SFKeybindServiceV2 {
    uint32_t size;
    uint32_t version;
    void *context;
    SFNativeStatusV1(SF_NATIVE_CALL *read_keybind)(void *context,
        const uint8_t *setting_key_utf8, uint32_t setting_key_length,
        uint32_t out_size, SFKeybindStateV2 *out_state);
} SFKeybindServiceV2;

/* Owner-bound settings persistence for this package only. Snapshot writes one
 * JSON object containing both revision and values. Patch accepts a partial
 * JSON object and expected 64-character SHA-256 revision. On CONFLICT the
 * patch made no change. No mod ID or filesystem path is accepted. */
#define SF_NATIVE_SERVICE_SETTINGS_V1 "shroudforge.settings"
typedef struct SFSettingsServiceV1 {
    uint32_t size;
    uint32_t version;
    void *context;
    SFNativeStatusV1(SF_NATIVE_CALL *read_snapshot)(void *context,
        uint8_t *destination, uint32_t capacity, uint32_t *out_required_bytes);
    SFNativeStatusV1(SF_NATIVE_CALL *patch)(void *context,
        const uint8_t *expected_revision_utf8, uint32_t expected_revision_length,
        const uint8_t *patch_json_utf8, uint32_t patch_json_length);
} SFSettingsServiceV1;

typedef struct SFNativeHostV1 SFNativeHostV1;

typedef void(SF_NATIVE_CALL *SFNativeLogFnV1)(
    void *package_context,
    SFNativeLogLevelV1 level,
    const uint8_t *message_utf8,
    uint32_t message_length);

typedef SFNativeStatusV1(SF_NATIVE_CALL *SFNativeReadSettingsJsonFnV1)(
    void *package_context,
    uint8_t *destination,
    uint32_t capacity,
    uint32_t *out_required_bytes);

/*
 * Returns the current settings object's settingValues JSON as UTF-8, without a
 * trailing NUL. On BUFFER_TOO_SMALL, out_required_bytes is exact and no bytes
 * are written. This callback is restricted to the runtime owner thread.
 */
/*
 * Control-plane package-relative read through the loader's package filesystem.
 * On BUFFER_TOO_SMALL, required_bytes is exact and no bytes are written. On
 * TOO_LARGE, required_bytes is zero because the asset exceeded the fixed cap.
 */
typedef SFNativeStatusV1(SF_NATIVE_CALL *SFNativeReadAssetFnV1)(
    void *package_context,
    const uint8_t *relative_path_utf8,
    uint32_t relative_path_length,
    uint8_t *destination,
    uint32_t capacity,
    uint32_t *out_required_bytes);

/* out_capacity bounds every write to out_table for the requested service. */
typedef SFNativeStatusV1(SF_NATIVE_CALL *SFNativeQueryServiceFnV1)(
    void *package_context,
    const uint8_t *service_name_utf8,
    uint32_t service_name_length,
    uint32_t requested_version,
    uint32_t out_capacity,
    void *out_table);

/* These owner-bound hooks must return BUSY until all host callbacks have left. */
typedef SFNativeStatusV1(SF_NATIVE_CALL *SFNativeRevokeServicesFnV1)(
    void *package_context,
    uint64_t package_generation);

typedef SFNativeStatusV1(SF_NATIVE_CALL *SFNativeDrainServicesFnV1)(
    void *package_context,
    uint64_t package_generation,
    uint32_t timeout_ms);

struct SFNativeHostV1 {
    uint32_t struct_size;
    uint32_t abi_version;
    const uint8_t *package_id_utf8;
    uint32_t package_id_length;
    uint64_t package_generation;
    SFNativeTargetV1 process_target;
    uint32_t reserved;
    void *package_context;
    SFNativeLogFnV1 log;
    SFNativeReadSettingsJsonFnV1 read_settings_json;
    SFNativeReadAssetFnV1 read_asset;
    SFNativeQueryServiceFnV1 query_service;
    SFNativeRevokeServicesFnV1 revoke_services;
    SFNativeDrainServicesFnV1 drain_services;
};

typedef SFNativeStatusV1(SF_NATIVE_CALL *SFNativeInitializeFnV1)(
    void *plugin_context,
    const SFNativeHostV1 *host);

typedef SFNativeStatusV1(SF_NATIVE_CALL *SFNativeUpdateFnV1)(
    void *plugin_context,
    const SFNativeHostV1 *host,
    uint64_t elapsed_microseconds);

typedef SFNativeStatusV1(SF_NATIVE_CALL *SFNativeStopFnV1)(
    void *plugin_context,
    const SFNativeHostV1 *host);

typedef SFNativeStatusV1(SF_NATIVE_CALL *SFNativeShutdownFnV1)(
    void *plugin_context,
    const SFNativeHostV1 *host);

typedef struct SFNativePluginV1 {
    uint32_t struct_size;
    uint32_t abi_version;
    void *plugin_context;
    SFNativeInitializeFnV1 initialize;
    SFNativeUpdateFnV1 update;
    SFNativeStopFnV1 stop;
    SFNativeShutdownFnV1 shutdown;
} SFNativePluginV1;

/*
 * ABI and lifetime contract:
 * - Entry must reject unsupported requested_abi, NULL out_plugin, or an
 *   out_size smaller than sizeof(SFNativePluginV1) before writing any byte.
 *   It negotiates the table only; initialization belongs in initialize after
 *   the host's asynchronous DLL load has completed.
 * - DllMain and entry must not start workers or install hooks. Windows calls
 *   DllMain before SF can negotiate lifetime. Use the explicit process-lifetime
 *   service during initialize before starting effects that require retention.
 * - The host table, package ID bytes, and package_context stay valid through
 *   initialize/update/stop/shutdown and the host's final service drain.
 * - The plugin must stop its worker threads and stop using every host pointer
 *   before shutdown returns. Service callbacks may outlive stop, so the host
 *   independently revokes and drains them before unloading plugin code.
 * - read_settings_json, read_asset, query_service, revoke_services, and
 *   drain_services run on the runtime control thread. read_asset accepts a
 *   UTF-8 package-relative path of at most SF_NATIVE_MAX_ASSET_PATH_BYTES and reads at most
 *   SF_NATIVE_MAX_ASSET_BYTES. It never resolves against the process CWD.
 * - The host never passes C++ or Rust-owned objects or allocators across this
 *   ABI. Strings and buffers are byte spans with explicit lengths/capacities.
 */
SF_NATIVE_EXTERN_C SF_NATIVE_PLUGIN_EXPORT SFNativeStatusV1 SF_NATIVE_CALL
ShroudForgePluginEntry(
    uint32_t requested_abi,
    uint32_t out_size,
    SFNativePluginV1 *out_plugin);

#ifdef __cplusplus
}
#endif

#endif
