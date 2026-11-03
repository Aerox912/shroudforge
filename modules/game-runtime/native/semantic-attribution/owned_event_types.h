#pragma once

#include <array>
#include <cstdint>
#include <optional>
#include <variant>

namespace SemanticAttribution {

using ScalarAddress = std::uintptr_t;
using SemanticHash = std::uint32_t;
using SemanticKey = std::uint64_t;
using NativeFrameScalar = std::uintptr_t;
using NativeThreadScalar = std::uintptr_t;

class Ledger;
class NativeEventAdapter;
#if defined(SF_SEMANTIC_ATTRIBUTION_TESTING)
namespace testing {
class FixtureIssuer;
}
#endif

// These are logical receipt labels, not native RVAs or patch targets. Only the
// adapter's separately verified profile maps them to captured native sites.
// The selected-window contract, not this enum, controls any future patching.
enum class NativeSite : std::uint16_t {
    RowPassBegin,
    RowReservationBegin,
    RowReservationComplete,
    ContextSetupBegin,
    ContextConstructorBegin,
    ContextBirthCaptured,
    ContextPublicationStore,
    ContextPublicationComplete,
    ContextRetire,
    RowIterationEnd,
    RowPassEnd,
    RowContextClear,
    WorldSetupBegin,
    WorldBirthCaptured,
    WorldPublicationStore,
    WorldPublicationComplete,
    WorldRetireScene,
    WorldRetireAlternative,
    WorldRetireCommon,
    SchemaMutatePopulation,
    SchemaMutateAppend,
    SchemaMutateVectorCallback,
    SchemaMutateReorder,
    RegistrationBegin,
    RegistrationTupleStore,
    WorldRegistrationBatchComplete,
    RegistrationRetire,
    CompileBegin,
    CompileAssociationsRetire,
    CompileNativeOperation,
    CompileReadHash,
    CompileWriteHash,
    CompileReadCount,
    CompileWriteCount,
    CompileComplete,
    RootWorldLoaded,
    DispatchGroupBegin,
    DispatchRecordCaptured,
    DispatchRecordAssociated,
    ParentCreateBegin,
    ParentBirthReturn,
    ChildCreateBegin,
    NodeAllocationReturn,
    ChildCreatorReturn,
    DescriptorTargetAssign,
    ChildTargetStore,
    ChildPayloadCopy,
    NodePayloadComplete,
    RejectedNodeFree,
    RootKickAssignment,
    RootSubmitTailReturn,
    LinkTargetRetire,
    RootLinkAttemptBegin,
    DelayedCompletionTarget,
    DelayedLinkRequestEntered,
    LinkPrevalidate,
    LinkCommitted,
    GateHoldBegin,
    CachedSourceReserved,
    CachedClaim,
    FreshClaim,
    ClaimTransfer,
    WorkCallbackBegin,
    GateHoldRelease,
    CompletionBegin,
    CompletionRelease,
    CachedClaimAbandon,
    NodeRetire,
    ManagerRetireDispatch,
    ManagerRetireEarly,
    RRelocationBegin,
    MaterializeBegin,
    MaterializeOutput,
    MaterializeComplete,
};

namespace detail {
struct RowPassTag;
struct RowIterationTag;
struct ReservationTag;
struct ContextSetupTag;
struct RegistrationOpTag;
struct CompileOpTag;
struct RootAttributionTag;
struct DispatchGroupTag;
struct CreateOpTag;
struct MaterializeOpTag;
struct ContextBirthTag;
struct ContextPublicationTag;
struct WorldBirthTag;
struct WorldPublicationTag;
struct SystemBindingTag;
struct CompileTag;
struct DispatchRecordTag;
struct NodeBirthTag;
struct OriginalSubmitTag;
struct TargetEdgeTag;
struct PublicationTag;
struct ClaimTag;
struct GateHoldTag;
struct LinkAttemptTag;
struct ObservationTag;
}

template <class Tag>
class OpaqueId final {
public:
    OpaqueId() noexcept = default;
    OpaqueId(const OpaqueId&) noexcept = default;
    OpaqueId& operator=(const OpaqueId&) noexcept = default;
    friend bool operator==(const OpaqueId&, const OpaqueId&) noexcept = default;

private:
    OpaqueId(std::uint64_t ledger_incarnation,
             std::uint32_t slot,
             std::uint64_t generation) noexcept
        : ledger_incarnation_(ledger_incarnation), slot_(slot), generation_(generation) {}

    std::uint64_t ledger_incarnation_{};
    std::uint32_t slot_{};
    std::uint64_t generation_{};

    friend class Ledger;
};

using RowPassId = OpaqueId<detail::RowPassTag>;
using RowIterationId = OpaqueId<detail::RowIterationTag>;
using ReservationId = OpaqueId<detail::ReservationTag>;
using ContextSetupId = OpaqueId<detail::ContextSetupTag>;
using RegistrationOpId = OpaqueId<detail::RegistrationOpTag>;
using CompileOpId = OpaqueId<detail::CompileOpTag>;
using RootAttributionId = OpaqueId<detail::RootAttributionTag>;
using DispatchGroupId = OpaqueId<detail::DispatchGroupTag>;
using CreateOpId = OpaqueId<detail::CreateOpTag>;
using MaterializeOpId = OpaqueId<detail::MaterializeOpTag>;
using ContextBirthId = OpaqueId<detail::ContextBirthTag>;
using ContextPublicationId = OpaqueId<detail::ContextPublicationTag>;
using WorldBirthId = OpaqueId<detail::WorldBirthTag>;
using WorldPublicationId = OpaqueId<detail::WorldPublicationTag>;
using SystemBindingId = OpaqueId<detail::SystemBindingTag>;
using CompileId = OpaqueId<detail::CompileTag>;
using DispatchRecordId = OpaqueId<detail::DispatchRecordTag>;
using NodeBirthId = OpaqueId<detail::NodeBirthTag>;
using OriginalSubmitId = OpaqueId<detail::OriginalSubmitTag>;
using TargetEdgeId = OpaqueId<detail::TargetEdgeTag>;
using PublicationId = OpaqueId<detail::PublicationTag>;
using ClaimId = OpaqueId<detail::ClaimTag>;
using GateHoldId = OpaqueId<detail::GateHoldTag>;
using LinkAttemptId = OpaqueId<detail::LinkAttemptTag>;
using ObservationId = OpaqueId<detail::ObservationTag>;

// A copied source fact only. It is never used as a live-node or admission guard.
template <class BirthIdentity>
struct BirthStamp final {
    BirthIdentity identity{};
    ScalarAddress address{};
    NativeSite site{};
    NativeFrameScalar frame{};
    friend bool operator==(const BirthStamp&, const BirthStamp&) noexcept = default;
};

class NativeReceipt final {
public:
    NativeReceipt(const NativeReceipt&) noexcept = default;
    NativeReceipt& operator=(const NativeReceipt&) noexcept = default;
    NativeSite Site() const noexcept { return site_; }
    NativeThreadScalar ThreadToken() const noexcept { return thread_; }
    NativeFrameScalar FrameToken() const noexcept { return frame_; }
    std::uint64_t AdapterIncarnation() const noexcept { return adapter_incarnation_; }
    std::uint64_t ProfileIdentity() const noexcept { return profile_identity_; }

private:
    NativeReceipt(NativeSite site,
                  NativeThreadScalar thread,
                  NativeFrameScalar frame,
                  std::uint64_t adapter_incarnation,
                  std::uint64_t profile_identity) noexcept
        : site_(site),
          thread_(thread),
          frame_(frame),
          adapter_incarnation_(adapter_incarnation),
          profile_identity_(profile_identity) {}

    NativeSite site_{};
    NativeThreadScalar thread_{};
    NativeFrameScalar frame_{};
    std::uint64_t adapter_incarnation_{};
    std::uint64_t profile_identity_{};

    friend class NativeEventAdapter;
#if defined(SF_SEMANTIC_ATTRIBUTION_TESTING)
    friend class testing::FixtureIssuer;
#endif
};

enum class EventStatus : std::uint8_t {
    Accepted,
    Pending,
    Committed,
    Revoked,
    Refused,
    InvalidReceipt,
    CapacityExhausted,
    GenerationExhausted,
    Incomplete,
    AlreadyConsumed,
};

using IssuedHandle = std::variant<
    std::monostate,
    RowPassId,
    RowIterationId,
    ReservationId,
    ContextSetupId,
    RegistrationOpId,
    CompileOpId,
    RootAttributionId,
    DispatchGroupId,
    CreateOpId,
    MaterializeOpId,
    ContextBirthId,
    ContextPublicationId,
    WorldBirthId,
    WorldPublicationId,
    SystemBindingId,
    CompileId,
    DispatchRecordId,
    NodeBirthId,
    OriginalSubmitId,
    TargetEdgeId,
    PublicationId,
    ClaimId,
    GateHoldId,
    LinkAttemptId,
    ObservationId>;

struct EventReply final {
    EventStatus status{EventStatus::Refused};
    IssuedHandle issued{};
    IssuedHandle related{};
};

enum class SchemaMutationKind : std::uint8_t {
    Population,
    Append,
    VectorGrowth,
    Reorder,
};

enum class WorldRetireKind : std::uint8_t {
    SceneDisposer,
    AlternativeDisposer,
    CommonOwnerDisposer,
};

enum class NodeRole : std::uint8_t { Parent, Child };
enum class LinkMode : std::uint8_t { AcquiredGate, CallerHeldGate };
enum class ClaimKind : std::uint8_t { Cached, Fresh };
enum class HashDirection : std::uint8_t { Read, Write };
enum class CompletionOrigin : std::uint8_t { WorkerReturn, CachedReturn };

inline constexpr ScalarAddress kApprovedParentCreator = 0x5FC0B0;
inline constexpr ScalarAddress kWorkloadCallback = 0x683800;
inline constexpr std::uint32_t kRootKickSlotLimit = 0x380;
inline constexpr std::uint32_t kMaxMaterializeEntries = 42;
inline constexpr std::uint32_t kMaxDispatchRecords = 64;

struct RowPassBegin final {};
struct RowPassEnd final { RowPassId pass; };
struct RowIterationEnd final { RowIterationId iteration; };
// ReservationBegin snapshots any already-owned context/world/compiler
// associations for this row while the ledger gate is held. The snapshot is
// immutable for the lifetime of this iteration; later native/current-map
// changes cannot refresh it.
struct RowReservationBegin final { RowPassId pass; ScalarAddress row{}; };
struct RowReservationComplete final {
    ReservationId reservation;
    ScalarAddress row{};
    ScalarAddress context{};
};

struct ContextSetupBegin final { RowPassId pass; ScalarAddress row{}; };
struct ContextConstructBegin final {
    std::optional<ContextSetupId> setup;
    ScalarAddress context_address{};
};
struct ContextBirthCaptured final {
    ContextSetupId setup;
    ScalarAddress context{};
    bool initialized{};
};
struct ContextPublicationBegin final {
    ContextSetupId setup;
    ContextBirthId birth;
    ScalarAddress row{};
};
struct ContextPublicationComplete final {
    ReservationId reservation;
    ScalarAddress row{};
    ScalarAddress context{};
    bool native_store_completed{};
};
struct ContextRetire final { ScalarAddress context{}; };
struct RowContextClear final { ScalarAddress row{}; };

struct WorldSetupBegin final {
    std::optional<ContextPublicationId> context;
    ScalarAddress manager{};
    ScalarAddress owner{};
};
struct WorldBirthCaptured final {
    WorldBirthId candidate;
    ScalarAddress world{};
    bool constructor_completed{};
};
struct WorldPublicationBegin final {
    WorldBirthId world;
    ContextPublicationId context;
    ScalarAddress context_address{};
};
struct WorldPublicationComplete final {
    ReservationId reservation;
    ScalarAddress context_address{};
    ScalarAddress world_address{};
    bool native_store_completed{};
};
struct WorldRetire final { ScalarAddress address{}; WorldRetireKind kind{}; };
struct SchemaMutate final {
    std::optional<WorldPublicationId> world;
    ScalarAddress schema{};
    SchemaMutationKind kind{};
};

struct RegistrationBegin final {
    WorldPublicationId world;
    ScalarAddress world_base{};
    ScalarAddress type_descriptor{};
    std::uint8_t disable{};
};
// Captured at the actual per-entry stores. `ordinal` is the observed one-based
// native count, never an ordinal assigned by DescriptorCatalog.
struct RegistrationStore final {
    RegistrationOpId operation;
    ScalarAddress record_slot{};
    ScalarAddress type_descriptor{};
    ScalarAddress callback{};
    std::uint8_t disable{};
    std::uint32_t flags_one{};
    std::uint32_t flags_two{};
    std::uint16_t ordinal{};
};
struct WorldRegistrationBatchComplete final {
    WorldPublicationId world;
    ScalarAddress world_base{};
    std::uint16_t observed_count{};
    bool native_completed{};
};
// Retires only the registration tuple whose captured S slot matches this scalar.
struct RegistrationRetire final { ScalarAddress record_slot{}; };

// Close-only witness at the native schema reset seam. It never creates a
// CompileId or adopts a current schema association.
struct CompileAssociationsRetire final { ScalarAddress schema{}; };

struct CompileBegin final {
    WorldPublicationId world;
    SystemBindingId binding;
    std::variant<RowIterationId, RootAttributionId> owner_scope;
    ScalarAddress schema{};
    std::optional<std::uint32_t> expected_read_entries;
    std::optional<std::uint32_t> expected_write_entries;
};
struct CompileHashCaptured final {
    CompileOpId operation;
    HashDirection direction{};
    SemanticHash hash{};
    ScalarAddress exact_slot{};
    std::uint32_t ordinal{};
    // Captured branch category: 0 for READ and 1 for WRITE.
    std::uint32_t group{};
    // Native flag0 occupies bits 0-7, flag1 bits 8-15; upper 16 bits are zero.
    std::uint32_t flags{};
    bool optional{};
};
struct CompileCountCaptured final {
    CompileOpId operation;
    HashDirection direction{};
    std::uint32_t count{};
};
struct CompileComplete final {
    CompileOpId operation;
    std::uint32_t read_count{};
    std::uint32_t write_count{};
    bool native_completed{};
};

struct RootAttributionBegin final {
    RowPassId row_pass;
    RowIterationId iteration;
    ContextPublicationId context;
    WorldPublicationId world;
    ScalarAddress root{};
    // Actual scalar loaded at RootWorldLoaded; must match the carried world birth.
    ScalarAddress actual_world_scalar{};
    NativeFrameScalar original_frame{};
};
struct DispatchGroupBegin final {
    RootAttributionId root;
    ScalarAddress dispatch_group{};
    NativeFrameScalar original_frame{};
};
struct DispatchRecordCaptured final {
    RootAttributionId root;
    CompileId compile;
    SystemBindingId binding;
    ScalarAddress system_alias{};
    ScalarAddress record_address{};
    std::uint32_t store_path{};
};
struct DispatchRecordAssociated final {
    DispatchGroupId group;
    DispatchRecordId record;
    ScalarAddress manager{};
    ScalarAddress record_address{};
    ScalarAddress system_alias{};
};

struct ParentCreateBegin final {
    RootAttributionId root;
    DispatchGroupId group;
    ScalarAddress descriptor{};
    ScalarAddress callback{};
    ScalarAddress descriptor_parent{};
};
struct DescriptorTargetAssigned final {
    DispatchGroupId group;
    std::optional<NodeBirthId> target_parent;
    ScalarAddress actual_target{};
};
struct NodeCreateBegin final {
    RootAttributionId root;
    DispatchGroupId group;
    NodeBirthId parent;
    ScalarAddress descriptor{};
    ScalarAddress creator_callback{};
    TargetEdgeId target_edge;
};
struct NodeBirthCaptured final {
    CreateOpId operation;
    ScalarAddress node{};
    ScalarAddress owner{};
    bool allocation_succeeded{};
};
struct ParentCreatorReturned final {
    CreateOpId operation;
    ScalarAddress parent{};
    ScalarAddress callback{};
    ScalarAddress descriptor_parent{};
};
struct ChildCreatorReturned final {
    CreateOpId operation;
    NodeBirthId child;
    ScalarAddress returned_child{};
};
struct ChildTargetEdgeCaptured final {
    CreateOpId operation;
    TargetEdgeId edge;
    NodeBirthId parent;
    NodeBirthId child;
    ScalarAddress descriptor_target{};
    ScalarAddress loaded_target{};
    ScalarAddress stored_target{};
};
struct ChildPayloadCaptured final {
    NodeBirthId node;
    ScalarAddress owner{};
    std::uint64_t payload_low{};
    std::uint64_t payload_high{};
    std::uint32_t dispatch_record_count{};
    std::array<DispatchRecordId, kMaxDispatchRecords> dispatch_records{};
};
struct NodePayloadCompleted final { NodeBirthId node; bool native_completed{}; };
struct RootKickAssigned final {
    RootAttributionId root;
    DispatchGroupId group;
    OriginalSubmitId submit;
    NodeBirthId child;
    ScalarAddress stack_slot{};
    ScalarAddress stored_child{};
    ScalarAddress callback{};
    std::uint64_t r15{};
    std::uint64_t native_pending_count{};
    std::uint32_t slot_index{};
    bool store_completed{};
};
struct RootSubmitTailReturned final {
    OriginalSubmitId submit;
    std::optional<LinkAttemptId> attempt;
};

struct RootLinkAttemptBegin final {
    OriginalSubmitId submit;
    ScalarAddress actual_root_slot{};
    ScalarAddress actual_child{};
    ScalarAddress target_owner{};
    LinkMode mode{};
    ScalarAddress actual_gate{};
    std::optional<GateHoldId> gate_hold;
};
// Close-only witness at the common native prelink seam. It retires only an
// already-owned target matching both observed target address and owner scalar.
struct LinkTargetRetire final {
    ScalarAddress actual_target{};
    ScalarAddress target_owner{};
};
struct DelayedLinkAttemptBegin final {
    ClaimId source_claim;
    OriginalSubmitId submit;
    NodeBirthId source_child;
    TargetEdgeId original_edge;
    NodeBirthId target_parent;
    ScalarAddress target_address{};
    ScalarAddress target_owner{};
    LinkMode mode{};
    ScalarAddress actual_gate{};
    std::optional<GateHoldId> gate_hold;
    CompletionOrigin origin{};
    bool actual_last_run{};
    ScalarAddress actual_source{};
    ScalarAddress actual_loaded_target{};
};
struct DelayedLinkRequestEntered final {
    LinkAttemptId attempt;
    ScalarAddress actual_source{};
    ScalarAddress actual_target{};
};
struct LinkPrevalidated final {
    LinkAttemptId attempt;
    ScalarAddress target_address{};
    ScalarAddress target_owner{};
    ScalarAddress actual_gate{};
    bool native_gate_acquired{};
};
struct LinkCommitted final {
    LinkAttemptId attempt;
    ScalarAddress target_address{};
    ScalarAddress target_owner{};
    ScalarAddress actual_gate{};
    bool native_queue_count_increment_completed{};
    bool native_gate_still_held{};
    bool normal_post{};
};

struct GateHoldBegin final {
    ScalarAddress target_owner{};
    ScalarAddress gate_address{};
};
// Captured after the native owner gate is acquired. The following cached
// claim must consume this scalar under the same still-live GateHoldId.
struct CachedSourceReserved final {
    GateHoldId gate_hold;
    ScalarAddress actual_cached_node{};
};
struct QueueClaimCaptured final {
    ClaimKind kind{};
    GateHoldId gate_hold;
    ScalarAddress actual_node{};
    bool native_increment_completed{};
};
struct QueueClaimTransfer final { ClaimId claim; NodeBirthId node; };
struct WorkCallbackBegin final {
    ClaimId claim;
    NodeBirthId node;
    ScalarAddress callback{};
};
struct GateHoldRelease final { GateHoldId hold; ScalarAddress actual_owner{}; };
struct CompletionBegin final { ClaimId claim; NodeBirthId node; };
struct CompletionRelease final { ClaimId claim; NodeBirthId node; };
struct CachedClaimAbandon final { std::optional<ClaimId> claim; };
struct NodeRetire final { std::optional<NodeBirthId> node; ScalarAddress address{}; };
struct RejectedNodeFree final { ScalarAddress address{}; };
enum class ManagerRetireKind : std::uint8_t { DispatchOwner, EarlyRecord };
struct ManagerRetire final { ScalarAddress manager{}; ManagerRetireKind kind{}; };
struct RRelocationBegin final { ScalarAddress container{}; };

struct MaterializeBegin final {
    std::variant<RootAttributionId, ClaimId> source;
    std::optional<PublicationId> publication;
    CompileId compile;
    SystemBindingId binding;
    DispatchRecordId dispatch;
    std::optional<NodeBirthId> parent;
    std::optional<NodeBirthId> child;
};
struct MaterializeEntryCaptured final {
    MaterializeOpId operation;
    ScalarAddress source_entry{};
    std::uint32_t first_ordinal{};
    std::uint32_t final_ordinal{};
    HashDirection direction{};
    std::uint32_t group{};
    ScalarAddress emitted_component_pointer{};
    std::uint32_t component_stride{};
    std::uint32_t original_row_index{};
    ScalarAddress native_output_record_address{};
};
struct MaterializeComplete final {
    MaterializeOpId operation;
    std::uint32_t query_count{};
    std::uint32_t completed_count{};
};

using Event = std::variant<
    RowPassBegin,
    RowPassEnd,
    RowIterationEnd,
    RowReservationBegin,
    RowReservationComplete,
    ContextSetupBegin,
    ContextConstructBegin,
    ContextBirthCaptured,
    ContextPublicationBegin,
    ContextPublicationComplete,
    ContextRetire,
    RowContextClear,
    WorldSetupBegin,
    WorldBirthCaptured,
    WorldPublicationBegin,
    WorldPublicationComplete,
    WorldRetire,
    SchemaMutate,
    RegistrationBegin,
    RegistrationStore,
    WorldRegistrationBatchComplete,
    RegistrationRetire,
    CompileBegin,
    CompileAssociationsRetire,
    CompileHashCaptured,
    CompileCountCaptured,
    CompileComplete,
    RootAttributionBegin,
    DispatchGroupBegin,
    DispatchRecordCaptured,
    DispatchRecordAssociated,
    ParentCreateBegin,
    DescriptorTargetAssigned,
    NodeCreateBegin,
    NodeBirthCaptured,
    ChildCreatorReturned,
    ParentCreatorReturned,
    ChildTargetEdgeCaptured,
    ChildPayloadCaptured,
    NodePayloadCompleted,
    RootKickAssigned,
    RootSubmitTailReturned,
    LinkTargetRetire,
    RootLinkAttemptBegin,
    DelayedLinkAttemptBegin,
    DelayedLinkRequestEntered,
    LinkPrevalidated,
    LinkCommitted,
    GateHoldBegin,
    CachedSourceReserved,
    QueueClaimCaptured,
    QueueClaimTransfer,
    WorkCallbackBegin,
    GateHoldRelease,
    CompletionBegin,
    CompletionRelease,
    CachedClaimAbandon,
    NodeRetire,
    RejectedNodeFree,
    ManagerRetire,
    RRelocationBegin,
    MaterializeBegin,
    MaterializeEntryCaptured,
    MaterializeComplete>;

} // namespace SemanticAttribution
