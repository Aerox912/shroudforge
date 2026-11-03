#include "owned_ledger.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <limits>
#include <memory>
#include <mutex>
#include <new>
#include <type_traits>

namespace SemanticAttribution {
namespace {

std::atomic<std::uint64_t> g_next_incarnation{1};

bool NextIncarnation(std::uint64_t& value) noexcept {
    auto current = g_next_incarnation.load(std::memory_order_relaxed);
    while (current != 0 && current != std::numeric_limits<std::uint64_t>::max()) {
        if (g_next_incarnation.compare_exchange_weak(
                current, current + 1, std::memory_order_relaxed)) {
            value = current;
            return true;
        }
    }
    return false;
}

bool AddAddress(ScalarAddress left, ScalarAddress right, ScalarAddress& result) noexcept {
    if (right > std::numeric_limits<ScalarAddress>::max() - left) return false;
    result = left + right;
    return true;
}

bool MulAddAddress(ScalarAddress base,
                   std::uint64_t index,
                   std::uint64_t stride,
                   ScalarAddress& result) noexcept {
    if (stride != 0 && index > std::numeric_limits<ScalarAddress>::max() / stride) return false;
    return AddAddress(base, static_cast<ScalarAddress>(index * stride), result);
}

bool RegistrationSlot(ScalarAddress world_base,
                      std::uint16_t ordinal,
                      ScalarAddress& result) noexcept {
    if (ordinal == 0) return false;
    ScalarAddress start{};
    if (!AddAddress(world_base, 0x746350, start)) return false;
    return MulAddAddress(start, static_cast<std::uint64_t>(ordinal - 1),
                         0x1F20, result);
}

template <class T>
bool AddSize(std::size_t& total, std::size_t count = 1) noexcept {
    if (count > (std::numeric_limits<std::size_t>::max() - total) / sizeof(T)) {
        return false;
    }
    total += count * sizeof(T);
    const auto alignment = alignof(T) - 1;
    if (alignment > std::numeric_limits<std::size_t>::max() - total) {
        return false;
    }
    total += alignment;
    return true;
}

template <class T>
T* Carve(std::span<std::byte> storage, std::size_t& offset, std::size_t count = 1) noexcept {
    if (count == 0 || count > std::numeric_limits<std::size_t>::max() / sizeof(T)) {
        return nullptr;
    }
    const auto bytes = count * sizeof(T);
    if (offset > storage.size()) {
        return nullptr;
    }
    void* address = storage.data() + offset;
    std::size_t remaining = storage.size() - offset;
    void* aligned = std::align(alignof(T), bytes, address, remaining);
    if (aligned == nullptr) {
        return nullptr;
    }
    auto* result = static_cast<T*>(aligned);
    const auto* base = storage.data();
    offset = static_cast<std::size_t>(reinterpret_cast<const std::byte*>(result) - base)
        + bytes;
    return result;
}

template <class T>
bool SameHandle(const IssuedHandle& handle, const T& value) noexcept {
    const auto* typed = std::get_if<T>(&handle);
    return typed != nullptr && *typed == value;
}

NativeSite SiteFor(const Event& event) noexcept {
    return std::visit([](const auto& item) noexcept -> NativeSite {
        using T = std::remove_cvref_t<decltype(item)>;
        if constexpr (std::is_same_v<T, RowPassBegin>) return NativeSite::RowPassBegin;
        else if constexpr (std::is_same_v<T, RowPassEnd>) return NativeSite::RowPassEnd;
        else if constexpr (std::is_same_v<T, RowIterationEnd>) return NativeSite::RowIterationEnd;
        else if constexpr (std::is_same_v<T, RowReservationBegin>) return NativeSite::RowReservationBegin;
        else if constexpr (std::is_same_v<T, RowReservationComplete>) return NativeSite::RowReservationComplete;
        else if constexpr (std::is_same_v<T, ContextSetupBegin>) return NativeSite::ContextSetupBegin;
        else if constexpr (std::is_same_v<T, ContextConstructBegin>) return NativeSite::ContextConstructorBegin;
        else if constexpr (std::is_same_v<T, ContextBirthCaptured>) return NativeSite::ContextBirthCaptured;
        else if constexpr (std::is_same_v<T, ContextPublicationBegin>) return NativeSite::ContextPublicationStore;
        else if constexpr (std::is_same_v<T, ContextPublicationComplete>) return NativeSite::ContextPublicationComplete;
        else if constexpr (std::is_same_v<T, ContextRetire>) return NativeSite::ContextRetire;
        else if constexpr (std::is_same_v<T, RowContextClear>) return NativeSite::RowContextClear;
        else if constexpr (std::is_same_v<T, WorldSetupBegin>) return NativeSite::WorldSetupBegin;
        else if constexpr (std::is_same_v<T, WorldBirthCaptured>) return NativeSite::WorldBirthCaptured;
        else if constexpr (std::is_same_v<T, WorldPublicationBegin>) return NativeSite::WorldPublicationStore;
        else if constexpr (std::is_same_v<T, WorldPublicationComplete>) return NativeSite::WorldPublicationComplete;
        else if constexpr (std::is_same_v<T, WorldRetire>) {
            switch (item.kind) {
            case WorldRetireKind::SceneDisposer: return NativeSite::WorldRetireScene;
            case WorldRetireKind::AlternativeDisposer: return NativeSite::WorldRetireAlternative;
            case WorldRetireKind::CommonOwnerDisposer: return NativeSite::WorldRetireCommon;
            }
            return NativeSite::WorldRetireCommon;
        }
        else if constexpr (std::is_same_v<T, SchemaMutate>) {
            switch (item.kind) {
            case SchemaMutationKind::Population: return NativeSite::SchemaMutatePopulation;
            case SchemaMutationKind::Append: return NativeSite::SchemaMutateAppend;
            case SchemaMutationKind::VectorGrowth: return NativeSite::SchemaMutateVectorCallback;
            case SchemaMutationKind::Reorder: return NativeSite::SchemaMutateReorder;
            }
            return NativeSite::SchemaMutatePopulation;
        }
        else if constexpr (std::is_same_v<T, RegistrationBegin>) return NativeSite::RegistrationBegin;
        else if constexpr (std::is_same_v<T, RegistrationStore>) return NativeSite::RegistrationTupleStore;
        else if constexpr (std::is_same_v<T, WorldRegistrationBatchComplete>) return NativeSite::WorldRegistrationBatchComplete;
        else if constexpr (std::is_same_v<T, RegistrationRetire>) return NativeSite::RegistrationRetire;
        else if constexpr (std::is_same_v<T, CompileBegin>) return NativeSite::CompileBegin;
        else if constexpr (std::is_same_v<T, CompileAssociationsRetire>) return NativeSite::CompileAssociationsRetire;
        else if constexpr (std::is_same_v<T, CompileHashCaptured>) {
            return item.direction == HashDirection::Read
                ? NativeSite::CompileReadHash : NativeSite::CompileWriteHash;
        }
        else if constexpr (std::is_same_v<T, CompileCountCaptured>) {
            return item.direction == HashDirection::Read
                ? NativeSite::CompileReadCount : NativeSite::CompileWriteCount;
        }
        else if constexpr (std::is_same_v<T, CompileComplete>) return NativeSite::CompileComplete;
        else if constexpr (std::is_same_v<T, RootAttributionBegin>) return NativeSite::RootWorldLoaded;
        else if constexpr (std::is_same_v<T, DispatchGroupBegin>) return NativeSite::DispatchGroupBegin;
        else if constexpr (std::is_same_v<T, DispatchRecordCaptured>) return NativeSite::DispatchRecordCaptured;
        else if constexpr (std::is_same_v<T, DispatchRecordAssociated>) return NativeSite::DispatchRecordAssociated;
        else if constexpr (std::is_same_v<T, ParentCreateBegin>) return NativeSite::ParentCreateBegin;
        else if constexpr (std::is_same_v<T, DescriptorTargetAssigned>) return NativeSite::DescriptorTargetAssign;
        else if constexpr (std::is_same_v<T, NodeCreateBegin>) return NativeSite::ChildCreateBegin;
        else if constexpr (std::is_same_v<T, NodeBirthCaptured>) return NativeSite::NodeAllocationReturn;
        else if constexpr (std::is_same_v<T, ParentCreatorReturned>) return NativeSite::ParentBirthReturn;
        else if constexpr (std::is_same_v<T, ChildCreatorReturned>) return NativeSite::ChildCreatorReturn;
        else if constexpr (std::is_same_v<T, ChildTargetEdgeCaptured>) return NativeSite::ChildTargetStore;
        else if constexpr (std::is_same_v<T, ChildPayloadCaptured>) return NativeSite::ChildPayloadCopy;
        else if constexpr (std::is_same_v<T, NodePayloadCompleted>) return NativeSite::NodePayloadComplete;
        else if constexpr (std::is_same_v<T, RootKickAssigned>) return NativeSite::RootKickAssignment;
        else if constexpr (std::is_same_v<T, RootSubmitTailReturned>) return NativeSite::RootSubmitTailReturn;
        else if constexpr (std::is_same_v<T, LinkTargetRetire>) return NativeSite::LinkTargetRetire;
        else if constexpr (std::is_same_v<T, RootLinkAttemptBegin>) return NativeSite::RootLinkAttemptBegin;
        else if constexpr (std::is_same_v<T, DelayedLinkAttemptBegin>) return NativeSite::DelayedCompletionTarget;
        else if constexpr (std::is_same_v<T, DelayedLinkRequestEntered>) return NativeSite::DelayedLinkRequestEntered;
        else if constexpr (std::is_same_v<T, LinkPrevalidated>) return NativeSite::LinkPrevalidate;
        else if constexpr (std::is_same_v<T, LinkCommitted>) return NativeSite::LinkCommitted;
        else if constexpr (std::is_same_v<T, GateHoldBegin>) return NativeSite::GateHoldBegin;
        else if constexpr (std::is_same_v<T, CachedSourceReserved>) return NativeSite::CachedSourceReserved;
        else if constexpr (std::is_same_v<T, QueueClaimCaptured>) {
            return item.kind == ClaimKind::Cached ? NativeSite::CachedClaim : NativeSite::FreshClaim;
        }
        else if constexpr (std::is_same_v<T, QueueClaimTransfer>) return NativeSite::ClaimTransfer;
        else if constexpr (std::is_same_v<T, WorkCallbackBegin>) return NativeSite::WorkCallbackBegin;
        else if constexpr (std::is_same_v<T, GateHoldRelease>) return NativeSite::GateHoldRelease;
        else if constexpr (std::is_same_v<T, CompletionBegin>) return NativeSite::CompletionBegin;
        else if constexpr (std::is_same_v<T, CompletionRelease>) return NativeSite::CompletionRelease;
        else if constexpr (std::is_same_v<T, CachedClaimAbandon>) return NativeSite::CachedClaimAbandon;
        else if constexpr (std::is_same_v<T, NodeRetire>) return NativeSite::NodeRetire;
        else if constexpr (std::is_same_v<T, RejectedNodeFree>) return NativeSite::RejectedNodeFree;
        else if constexpr (std::is_same_v<T, ManagerRetire>) {
            return item.kind == ManagerRetireKind::DispatchOwner
                ? NativeSite::ManagerRetireDispatch : NativeSite::ManagerRetireEarly;
        }
        else if constexpr (std::is_same_v<T, RRelocationBegin>) return NativeSite::RRelocationBegin;
        else if constexpr (std::is_same_v<T, MaterializeBegin>) return NativeSite::MaterializeBegin;
        else if constexpr (std::is_same_v<T, MaterializeEntryCaptured>) return NativeSite::MaterializeOutput;
        else if constexpr (std::is_same_v<T, MaterializeComplete>) return NativeSite::MaterializeComplete;
        else return NativeSite::RowPassBegin;
    }, event);
}

struct RowPassData final { bool open{}; };
struct RowIterationData final {
    RowPassId pass{};
    ReservationId reservation{};
    ScalarAddress row{};
    ContextPublicationId context{};
    WorldPublicationId world{};
    std::uint32_t snapshot_count{};
    bool completed{};
    bool ended{};
};
struct RowReservationData final {
    RowPassId pass{};
    RowIterationId iteration{};
    ScalarAddress row{};
    ContextPublicationId snapshot_context{};
    WorldPublicationId snapshot_world{};
    std::uint32_t snapshot_count{};
    bool completed{};
};
struct ContextSetupData final {
    RowPassId pass{};
    RowIterationId iteration{};
    ScalarAddress row{};
    bool constructor_started{};
    ContextBirthId birth{};
};
struct ContextBirthData final {
    ContextSetupId setup{};
    ScalarAddress address{};
    NativeSite site{};
    NativeFrameScalar frame{};
};
struct ContextReservationData final {
    ContextSetupId setup{};
    ContextBirthId birth{};
    ScalarAddress row{};
};
struct ContextPublicationData final {
    ContextSetupId setup{};
    ContextBirthId birth{};
    ScalarAddress row{};
    ScalarAddress address{};
    WorldPublicationId current_world{};
};
struct WorldBirthData final {
    ContextPublicationId context{};
    ScalarAddress manager{};
    ScalarAddress owner{};
    ScalarAddress address{};
    bool constructor_completed{};
};
struct WorldReservationData final {
    WorldBirthId birth{};
    ContextPublicationId context{};
    ScalarAddress context_address{};
};
struct WorldPublicationData final {
    WorldBirthId birth{};
    ContextPublicationId context{};
    ScalarAddress context_address{};
    ScalarAddress address{};
    ScalarAddress manager{};
    ScalarAddress owner{};
    std::uint64_t registration_batch_generation{};
    bool registration_batch_open{};
};
struct RegistrationOperationData final {
    WorldPublicationId world{};
    ScalarAddress world_base{};
    ScalarAddress type_descriptor{};
    std::uint8_t disable{};
    std::uint64_t batch_generation{};
    SystemBindingId binding{};
    NativeThreadScalar thread{};
    NativeFrameScalar frame{};
};
struct SystemBindingData final {
    WorldPublicationId world{};
    ScalarAddress world_base{};
    ScalarAddress record_slot{};
    ScalarAddress type_descriptor{};
    ScalarAddress callback{};
    std::uint32_t flags_one{};
    std::uint32_t flags_two{};
    std::uint16_t ordinal{};
    std::uint64_t batch_generation{};
    RegistrationOpId operation{};
    bool batch_committed{};
};
using CompileScope = std::variant<RowIterationId, RootAttributionId>;
struct CompileOperationData final {
    WorldPublicationId world{};
    SystemBindingId binding{};
    CompileScope scope{};
    ScalarAddress schema{};
    std::optional<std::uint32_t> expected_read{};
    std::optional<std::uint32_t> expected_write{};
    std::uint32_t read_count{};
    std::uint32_t write_count{};
    std::uint32_t read_entries{};
    std::uint32_t write_entries{};
    bool saw_read_count{};
    bool saw_write_count{};
};
struct CompiledData final {
    WorldPublicationId world{};
    SystemBindingId binding{};
    CompileScope scope{};
    ScalarAddress schema{};
    std::uint32_t read_count{};
    std::uint32_t write_count{};
};
struct RootData final {
    RowPassId pass{};
    RowIterationId iteration{};
    ContextPublicationId context{};
    WorldPublicationId world{};
    ScalarAddress root{};
    ScalarAddress actual_world{};
    NativeFrameScalar frame{};
    bool bracket_closed{};
};
struct DispatchGroupData final {
    RootAttributionId root{};
    ScalarAddress address{};
    NativeFrameScalar frame{};
    DispatchRecordId record{};
    std::uint32_t record_count{};
    CompileId compile{};
    SystemBindingId binding{};
    ScalarAddress system_alias{};
    ScalarAddress manager{};
};
struct DispatchRecordData final {
    RootAttributionId root{};
    CompileId compile{};
    SystemBindingId binding{};
    ScalarAddress system_alias{};
    ScalarAddress record_address{};
    ScalarAddress manager{};
    std::uint32_t store_path{};
    DispatchGroupId group{};
    bool associated{};
};
struct CreateOperationData final {
    NodeRole role{};
    RootAttributionId root{};
    DispatchGroupId group{};
    NodeBirthId parent{};
    TargetEdgeId edge{};
    ScalarAddress descriptor{};
    ScalarAddress callback{};
    ScalarAddress descriptor_parent{};
    NodeBirthId birth{};
    bool allocated{};
    bool payload_complete{};
    bool creator_returned{};
};
struct NodeBirthData final {
    NodeRole role{};
    ScalarAddress address{};
    NativeFrameScalar birth_frame{};
    ScalarAddress owner{};
    RootAttributionId root{};
    DispatchGroupId group{};
    NodeBirthId parent{};
    TargetEdgeId edge{};
    ContextPublicationId context{};
    WorldPublicationId world{};
    CompileId compile{};
    SystemBindingId binding{};
    DispatchRecordId dispatch{};
    OriginalSubmitId submit{};
    std::uint64_t payload_low{};
    std::uint64_t payload_high{};
    bool payload_captured{};
    bool target_stored{};
    bool payload_complete{};
    bool creator_returned{};
    bool root_kicked{};
    std::uint32_t dispatch_record_count{};
    std::array<DispatchRecordId, kMaxDispatchRecords> dispatch_records{};
};
struct TargetEdgeData final {
    DispatchGroupId group{};
    NodeBirthId parent{};
    ScalarAddress target{};
    NodeBirthId child{};
    bool child_stored{};
};
struct OriginalSubmitData final {
    NodeBirthId child{};
    RootAttributionId root{};
    DispatchGroupId group{};
    ScalarAddress stack_slot{};
    LinkAttemptId attempt{};
    bool root_kicked{};
    bool tail_returned{};
};
struct PublicationData final {
    NodeBirthId target{};
    RootAttributionId root{};
    ContextPublicationId context{};
    WorldPublicationId world{};
    SystemBindingId binding{};
    CompileId compile{};
    DispatchRecordId dispatch{};
    NodeBirthId parent{};
    NodeBirthId child{};
    OriginalSubmitId submit{};
    ScalarAddress target_owner{};
    BirthStamp<NodeBirthId> parent_stamp{};
    BirthStamp<NodeBirthId> child_stamp{};
    DispatchGroupId group{};
    std::uint32_t dispatch_record_count{};
    std::array<DispatchRecordId, kMaxDispatchRecords> dispatch_records{};
};
enum class LinkKind : std::uint8_t { Root, Delayed };
struct LinkAttemptData final {
    LinkKind kind{};
    OriginalSubmitId submit{};
    ClaimId source_claim{};
    NodeBirthId source_child{};
    TargetEdgeId original_edge{};
    NodeBirthId target_parent{};
    ScalarAddress root_slot{};
    ScalarAddress source_address{};
    ScalarAddress target_address{};
    ScalarAddress target_owner{};
    ScalarAddress gate{};
    LinkMode mode{};
    std::optional<GateHoldId> gate_hold{};
    CompletionOrigin origin{};
    bool actual_last_run{};
    bool request_entered{};
    bool prevalidated{};
    bool native_gate_acquired{};
    bool native_queue_increment{};
    bool gate_still_held{};
    bool normal_post{};
    PublicationId publication{};
};
struct GateHoldData final {
    ScalarAddress owner{};
    ScalarAddress gate{};
    ScalarAddress cached_node{};
    PublicationId cached_publication{};
    bool cached_reserved{};
};
enum class ClaimState : std::uint8_t { Claimed, Transferred, Working, Completing, Released };
struct ClaimData final {
    PublicationId publication{};
    NodeBirthId node{};
    GateHoldId gate_hold{};
    ClaimKind kind{};
    ClaimState state{ClaimState::Claimed};
    LinkAttemptId delayed_attempt{};
    bool gate_released{};
    bool early_completion{};
};
struct MaterializeData final {
    std::variant<RootAttributionId, ClaimId> source{};
    std::optional<PublicationId> publication{};
    CompileId compile{};
    SystemBindingId binding{};
    DispatchRecordId dispatch{};
    std::optional<NodeBirthId> parent{};
    std::optional<NodeBirthId> child{};
    std::uint32_t entry_count{};
    NativeThreadScalar thread{};
    NativeFrameScalar frame{};
};

using RecordData = std::variant<
    std::monostate,
    RowPassData,
    RowIterationData,
    RowReservationData,
    ContextSetupData,
    ContextBirthData,
    ContextReservationData,
    ContextPublicationData,
    WorldBirthData,
    WorldReservationData,
    WorldPublicationData,
    RegistrationOperationData,
    SystemBindingData,
    CompileOperationData,
    CompiledData,
    RootData,
    DispatchGroupData,
    DispatchRecordData,
    CreateOperationData,
    NodeBirthData,
    TargetEdgeData,
    OriginalSubmitData,
    PublicationData,
    LinkAttemptData,
    GateHoldData,
    ClaimData,
    MaterializeData>;

struct Record final {
    std::uint64_t generation{};
    bool active{};
    IssuedHandle id{};
    IssuedHandle related{};
    RecordData data{};
};

enum class EvidenceKind : std::uint8_t {
    Free,
    RegistrationTuple,
    CompileQuery,
    SnapshotCompile,
    MaterializeEntry,
};
struct EvidenceEntry final {
    bool active{};
    EvidenceKind kind{EvidenceKind::Free};
    IssuedHandle owner{};
    IssuedHandle first{};
    IssuedHandle second{};
    ScalarAddress address{};
    ScalarAddress type_descriptor{};
    ScalarAddress callback{};
    SemanticHash hash{};
    SemanticKey key{};
    std::uint32_t ordinal{};
    std::uint32_t flags_one{};
    std::uint32_t flags_two{};
    std::uint32_t group{};
    std::uint32_t flags{};
    std::uint32_t component_stride{};
    ScalarAddress source_entry{};
    ScalarAddress emitted_component_pointer{};
    ScalarAddress native_output_record_address{};
    std::uint32_t first_ordinal{};
    std::uint32_t final_ordinal{};
    std::uint32_t original_row_index{};
    HashDirection direction{};
    bool optional{};
    std::uint32_t capture_sequence{};
};

struct ObservationSlot final {
    std::uint64_t generation{};
    bool active{};
    ObservationId id{};
    OwnedObservation observation{};
};

} // namespace

template <class Id>
Id Ledger::MintId(std::uint64_t incarnation,
                  std::uint32_t slot,
                  std::uint64_t generation) noexcept {
    return Id(incarnation, slot, generation);
}

struct Ledger::Impl final {
    explicit Impl(Limits configured,
                  DescriptorCatalog descriptor_catalog,
                  std::uint64_t ledger_incarnation,
                  Record* record_storage,
                  EvidenceEntry* evidence_storage,
                  ObservationSlot* observation_storage) noexcept
        : limits(configured),
          catalog(descriptor_catalog),
          incarnation(ledger_incarnation),
          records(record_storage),
          evidence(evidence_storage),
          observations(observation_storage) {}

    Limits limits{};
    DescriptorCatalog catalog{};
    std::uint64_t incarnation{};
    Record* records{};
    EvidenceEntry* evidence{};
    ObservationSlot* observations{};
    std::mutex gate{};
    bool closed{};
    bool generation_exhausted{};

    template <class Id>
    Record* FindRecord(Id id) noexcept {
        if (id == Id{}) return nullptr;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            if (record.active && SameHandle(record.id, id)) return &record;
        }
        return nullptr;
    }

    template <class Id>
    const Record* FindRecord(Id id) const noexcept {
        if (id == Id{}) return nullptr;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            const auto& record = records[i];
            if (record.active && SameHandle(record.id, id)) return &record;
        }
        return nullptr;
    }

    template <class Data, class Id>
    Data* Find(Id id) noexcept {
        auto* record = FindRecord(id);
        return record == nullptr ? nullptr : std::get_if<Data>(&record->data);
    }

    template <class Data, class Id>
    const Data* Find(Id id) const noexcept {
        const auto* record = FindRecord(id);
        return record == nullptr ? nullptr : std::get_if<Data>(&record->data);
    }

    bool ContainsDispatchRecord(
        const std::array<DispatchRecordId, kMaxDispatchRecords>& records_to_check,
        std::uint32_t count,
        DispatchRecordId record_id) const noexcept {
        if (record_id == DispatchRecordId{} || count > records_to_check.size()) return false;
        for (std::uint32_t i = 0; i < count; ++i) {
            if (records_to_check[i] == record_id) return true;
        }
        return false;
    }

    template <class Id, class Data>
    Record* Allocate(Ledger& ledger, Data data, EventStatus& failure) noexcept {
        failure = EventStatus::CapacityExhausted;
        bool saw_exhausted = false;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& slot = records[i];
            if (slot.active) continue;
            if (slot.generation == std::numeric_limits<std::uint64_t>::max()) {
                saw_exhausted = true;
                continue;
            }
            const auto next_generation = slot.generation + 1;
            slot = Record{};
            slot.generation = next_generation;
            slot.active = true;
            slot.id = ledger.MintId<Id>(incarnation, i + 1, next_generation);
            slot.data = std::move(data);
            failure = EventStatus::Refused;
            return &slot;
        }
        if (saw_exhausted) {
            generation_exhausted = true;
            failure = EventStatus::GenerationExhausted;
        }
        return nullptr;
    }

    template <class NewId, class Data>
    bool Promote(Ledger& ledger, Record& record, Data data) noexcept {
        if (!record.active || record.generation == std::numeric_limits<std::uint64_t>::max()) {
            generation_exhausted = true;
            record.active = false;
            return false;
        }
        const IssuedHandle previous = record.id;
        std::uint32_t slot_number = 0;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            if (&records[i] == &record) {
                slot_number = i + 1;
                break;
            }
        }
        if (slot_number == 0) {
            record.active = false;
            return false;
        }
        ++record.generation;
        record.id = ledger.MintId<NewId>(incarnation, slot_number, record.generation);
        record.related = {};
        record.data = std::move(data);
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            auto& entry = evidence[i];
            if (entry.active && entry.owner == previous) entry.owner = record.id;
        }
        return true;
    }

    void RetireRecord(Record& record) noexcept {
        if (!record.active) return;
        const IssuedHandle previous = record.id;
        record.active = false;
        record.related = {};
        record.data = std::monostate{};
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            if (evidence[i].active && evidence[i].owner == previous) evidence[i].active = false;
        }
    }

    template <class Id>
    EvidenceEntry* AllocateEvidence(Id owner, EvidenceKind kind) noexcept {
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            if (evidence[i].active) continue;
            evidence[i] = EvidenceEntry{};
            evidence[i].active = true;
            evidence[i].kind = kind;
            evidence[i].owner = owner;
            return &evidence[i];
        }
        return nullptr;
    }

    template <class Id>
    std::uint32_t EvidenceCount(Id owner, EvidenceKind kind) const noexcept {
        std::uint32_t count = 0;
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            if (evidence[i].active && evidence[i].kind == kind && SameHandle(evidence[i].owner, owner)) {
                ++count;
            }
        }
        return count;
    }

    SemanticKey CatalogKey(SemanticHash hash) const noexcept {
        for (const auto& entry : catalog.entries) {
            if (entry.semantic_hash == hash) return entry.semantic_key;
        }
        return 0;
    }

    template <class Id>
    void RetireEvidence(Id owner) noexcept {
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            if (evidence[i].active && SameHandle(evidence[i].owner, owner)) evidence[i].active = false;
        }
    }

    bool SnapshotContains(RowIterationId iteration,
                          SystemBindingId binding,
                          CompileId compile) const noexcept {
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            const auto& item = evidence[i];
            if (item.active && item.kind == EvidenceKind::SnapshotCompile
                && SameHandle(item.owner, iteration)
                && SameHandle(item.first, binding)
                && SameHandle(item.second, compile)) return true;
        }
        return false;
    }

    bool BindingContainsAlias(SystemBindingId binding, ScalarAddress alias) const noexcept {
        const auto* data = Find<SystemBindingData>(binding);
        return data != nullptr && data->batch_committed && alias != 0
            && data->type_descriptor == alias;
    }

    bool CompileEligible(const RootData& root,
                         RootAttributionId root_id,
                         CompileId compile_id,
                         SystemBindingId binding_id) const noexcept {
        const auto* compile = Find<CompiledData>(compile_id);
        const auto* binding = Find<SystemBindingData>(binding_id);
        if (compile == nullptr || binding == nullptr
            || compile->world != root.world || binding->world != root.world
            || compile->binding != binding_id || !binding->batch_committed) return false;
        if (const auto* row_owner = std::get_if<RowIterationId>(&compile->scope)) {
            return *row_owner == root.iteration
                || SnapshotContains(root.iteration, binding_id, compile_id);
        }
        if (const auto* root_owner = std::get_if<RootAttributionId>(&compile->scope)) {
            return *root_owner == root_id
                || SnapshotContains(root.iteration, binding_id, compile_id);
        }
        return false;
    }

    bool CompileHasQuery(CompileId compile,
                         ScalarAddress slot,
                         std::uint32_t ordinal,
                         std::uint32_t group,
                         HashDirection direction,
                         EvidenceEntry& result) const noexcept {
        std::uint32_t matches = 0;
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            const auto& item = evidence[i];
            if (item.active && item.kind == EvidenceKind::CompileQuery
                && SameHandle(item.owner, compile)
                && item.address == slot && item.ordinal == ordinal
                && item.group == group && item.direction == direction) {
                result = item;
                ++matches;
            }
        }
        return matches == 1;
    }

    bool MaterializeOwnerLive(const MaterializeData& data,
                              RootAttributionId& root_id,
                              ContextPublicationId& context_id,
                              WorldPublicationId& world_id,
                              const PublicationData*& publication) const noexcept {
        publication = nullptr;
        if (const auto* source_root = std::get_if<RootAttributionId>(&data.source)) {
            const auto* root = Find<RootData>(*source_root);
            if (root == nullptr || data.publication || data.parent || data.child) return false;
            root_id = *source_root;
            context_id = root->context;
            world_id = root->world;
            if (!CompileEligible(*root, root_id, data.compile, data.binding)) return false;
        } else if (const auto* source_claim = std::get_if<ClaimId>(&data.source)) {
            const auto* claim = Find<ClaimData>(*source_claim);
            const auto* node = claim == nullptr ? nullptr : Find<NodeBirthData>(claim->node);
            publication = data.publication ? Find<PublicationData>(*data.publication) : nullptr;
            if (claim == nullptr || claim->state != ClaimState::Working || !claim->gate_released
                || node == nullptr || node->role != NodeRole::Child || publication == nullptr
                || claim->publication != *data.publication || publication->target != claim->node
                || publication->child != claim->node || !data.parent || !data.child
                || publication->parent != *data.parent || publication->child != *data.child
                || publication->parent_stamp.identity != *data.parent
                || publication->child_stamp.identity != *data.child
                || node->parent != *data.parent || node->edge == TargetEdgeId{}
                || node->root != publication->root || node->context != publication->context
                || node->world != publication->world || node->compile != data.compile
                || node->binding != data.binding || node->group != publication->group
                || !ContainsDispatchRecord(publication->dispatch_records,
                                           publication->dispatch_record_count,
                                           node->dispatch)
                || !ContainsDispatchRecord(publication->dispatch_records,
                                           publication->dispatch_record_count,
                                           data.dispatch)) return false;
            root_id = publication->root;
            context_id = publication->context;
            world_id = publication->world;
            const auto* root = Find<RootData>(root_id);
            if (root == nullptr || root->context != context_id || root->world != world_id
                || !CompileEligible(*root, root_id, data.compile, data.binding)
                || publication->compile != data.compile || publication->binding != data.binding
                || publication->group == DispatchGroupId{}
                || !ContainsDispatchRecord(publication->dispatch_records,
                                           publication->dispatch_record_count,
                                           publication->dispatch)) return false;
        } else {
            return false;
        }

        if (Find<ContextPublicationData>(context_id) == nullptr
            || Find<WorldPublicationData>(world_id) == nullptr
            || Find<SystemBindingData>(data.binding) == nullptr
            || Find<CompiledData>(data.compile) == nullptr) return false;
        const auto* dispatch = Find<DispatchRecordData>(data.dispatch);
        if (dispatch == nullptr || dispatch->root != root_id || !dispatch->associated
            || dispatch->compile != data.compile || dispatch->binding != data.binding
            || !BindingContainsAlias(data.binding, dispatch->system_alias)) return false;
        if (publication != nullptr) {
            const auto* group = Find<DispatchGroupData>(publication->group);
            return group != nullptr && group->root == root_id
                && dispatch->group == publication->group
                && dispatch->manager == group->manager
                && dispatch->compile == group->compile
                && dispatch->binding == group->binding
                && dispatch->system_alias == group->system_alias
                && publication->dispatch_record_count != 0
                && publication->dispatch_record_count == group->record_count
                && ContainsDispatchRecord(publication->dispatch_records,
                                          publication->dispatch_record_count,
                                          data.dispatch);
        }
        return true;
    }

    bool HasLiveWorld(WorldPublicationId world) const noexcept {
        return Find<WorldPublicationData>(world) != nullptr;
    }

    void InvalidateObservations() noexcept {
        for (std::uint32_t i = 0; i < limits.max_observations; ++i) {
            if (!observations[i].active) continue;
            const auto& value = observations[i].observation;
            if (Find<ContextPublicationData>(value.context) == nullptr
                || Find<WorldPublicationData>(value.world) == nullptr
                || Find<SystemBindingData>(value.binding) == nullptr
                || Find<CompiledData>(value.compile) == nullptr
                || (value.publication != PublicationId{}
                    && Find<PublicationData>(value.publication) == nullptr)
                || (value.parent != NodeBirthId{}
                    && Find<NodeBirthData>(value.parent) == nullptr)) {
                observations[i].active = false;
            }
        }
    }

    void RevokeRoot(RootAttributionId root) noexcept;
    void RevokeCompile(CompileId compile) noexcept;
    void RevokeBinding(SystemBindingId binding) noexcept;
    void RevokeWorld(WorldPublicationId world) noexcept;
    void RevokeContext(ContextPublicationId context) noexcept;
    void RevokeNode(NodeBirthId node) noexcept;
    void RevokeAddress(ScalarAddress address) noexcept;
    void CloseContextSetup(ContextSetupId setup) noexcept;
    void CloseRootBracket(RootAttributionId root) noexcept;
    void CloseIteration(RowIterationId iteration) noexcept;
    void ClosePass(RowPassId pass) noexcept;
    bool RootHasLiveDependents(RootAttributionId root) const noexcept;
    bool RootIsOpen(RootAttributionId root) const noexcept;
    void SweepClosedLifecycle() noexcept;
    EventReply Observe(Ledger& ledger, const NativeReceipt& receipt, const Event& event) noexcept;

    template <class T>
    EventReply Apply(Ledger& ledger, const NativeReceipt& receipt, const T& item) noexcept;
};

void Ledger::Impl::RevokeCompile(CompileId compile_id) noexcept {
    if (compile_id == CompileId{}) return;
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* op = std::get_if<CompileOperationData>(&record.data)) {
            (void)op;
            continue;
        }
        if (const auto* compiled = std::get_if<CompiledData>(&record.data);
            compiled != nullptr && SameHandle(record.id, compile_id)) {
            RetireRecord(record);
        } else if (const auto* dispatch = std::get_if<DispatchRecordData>(&record.data);
                   dispatch != nullptr && dispatch->compile == compile_id) {
            RetireRecord(record);
        } else if (const auto* publication = std::get_if<PublicationData>(&record.data);
                   publication != nullptr && publication->compile == compile_id) {
            RetireRecord(record);
        } else if (const auto* materialize = std::get_if<MaterializeData>(&record.data);
                   materialize != nullptr && materialize->compile == compile_id) {
            RetireRecord(record);
        }
    }
    RetireEvidence(compile_id);
    InvalidateObservations();
}

void Ledger::Impl::RevokeBinding(SystemBindingId binding_id) noexcept {
    if (binding_id == SystemBindingId{}) return;
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* compiled = std::get_if<CompiledData>(&record.data);
            compiled != nullptr && compiled->binding == binding_id) {
            const auto* id = std::get_if<CompileId>(&record.id);
            if (id != nullptr) RevokeCompile(*id);
        }
    }
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* op = std::get_if<CompileOperationData>(&record.data);
            op != nullptr && op->binding == binding_id) {
            RetireRecord(record);
        } else if (const auto* dispatch = std::get_if<DispatchRecordData>(&record.data);
                   dispatch != nullptr && dispatch->binding == binding_id) {
            RetireRecord(record);
        } else if (const auto* publication = std::get_if<PublicationData>(&record.data);
                   publication != nullptr && publication->binding == binding_id) {
            RetireRecord(record);
        } else if (const auto* materialize = std::get_if<MaterializeData>(&record.data);
                   materialize != nullptr && materialize->binding == binding_id) {
            RetireRecord(record);
        } else if (std::get_if<SystemBindingData>(&record.data) != nullptr
                   && SameHandle(record.id, binding_id)) {
            RetireRecord(record);
        }
    }
    RetireEvidence(binding_id);
    InvalidateObservations();
}

void Ledger::Impl::RevokeRoot(RootAttributionId root_id) noexcept {
    if (root_id == RootAttributionId{}) return;
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* group = std::get_if<DispatchGroupData>(&record.data);
            group != nullptr && group->root == root_id) {
            RetireRecord(record);
        } else if (const auto* dispatch = std::get_if<DispatchRecordData>(&record.data);
                   dispatch != nullptr && dispatch->root == root_id) {
            RetireRecord(record);
        } else if (const auto* create = std::get_if<CreateOperationData>(&record.data);
                   create != nullptr && create->root == root_id) {
            RetireRecord(record);
        } else if (const auto* node = std::get_if<NodeBirthData>(&record.data);
                   node != nullptr && node->root == root_id) {
            const auto* id = std::get_if<NodeBirthId>(&record.id);
            if (id != nullptr) RevokeNode(*id);
        } else if (const auto* submit = std::get_if<OriginalSubmitData>(&record.data);
                   submit != nullptr && submit->root == root_id) {
            RetireRecord(record);
        } else if (const auto* publication = std::get_if<PublicationData>(&record.data);
                   publication != nullptr && publication->root == root_id) {
            RetireRecord(record);
        } else if (const auto* attempt = std::get_if<LinkAttemptData>(&record.data);
                   attempt != nullptr) {
            const auto* attempt_submit = Find<OriginalSubmitData>(attempt->submit);
            const auto* attempt_claim = Find<ClaimData>(attempt->source_claim);
            const auto* attempt_publication = attempt_claim == nullptr
                ? nullptr : Find<PublicationData>(attempt_claim->publication);
            if ((attempt_submit != nullptr && attempt_submit->root == root_id)
                || (attempt_publication != nullptr && attempt_publication->root == root_id)) {
                RetireRecord(record);
            }
        } else if (const auto* claim = std::get_if<ClaimData>(&record.data);
                   claim != nullptr) {
            const auto* claim_publication = Find<PublicationData>(claim->publication);
            if (claim_publication != nullptr && claim_publication->root == root_id) RetireRecord(record);
        } else if (const auto* materialize = std::get_if<MaterializeData>(&record.data);
                   materialize != nullptr) {
            bool matches = false;
            if (const auto* source_root = std::get_if<RootAttributionId>(&materialize->source)) {
                matches = *source_root == root_id;
            }
            const auto* materialize_publication = materialize->publication
                ? Find<PublicationData>(*materialize->publication) : nullptr;
            if (matches || (materialize_publication != nullptr
                    && materialize_publication->root == root_id)) {
                RetireRecord(record);
            }
        } else if (std::get_if<RootData>(&record.data) != nullptr
                   && SameHandle(record.id, root_id)) {
            RetireRecord(record);
        }
    }
    for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
        auto& item = evidence[i];
        if (!item.active || item.kind != EvidenceKind::SnapshotCompile) continue;
        const auto* iteration_id = std::get_if<RowIterationId>(&item.owner);
        const auto* root_data = iteration_id == nullptr ? nullptr : Find<RowIterationData>(*iteration_id);
        if (root_data == nullptr) item.active = false;
    }
    for (std::uint32_t i = 0; i < limits.max_observations; ++i) {
        if (observations[i].active && observations[i].observation.root == root_id) {
            observations[i].active = false;
        }
    }
    InvalidateObservations();
}

void Ledger::Impl::RevokeWorld(WorldPublicationId world_id) noexcept {
    if (world_id == WorldPublicationId{}) return;
    WorldBirthId associated_birth{};
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* compiled = std::get_if<CompiledData>(&record.data);
            compiled != nullptr && compiled->world == world_id) {
            const auto* id = std::get_if<CompileId>(&record.id);
            if (id != nullptr) RevokeCompile(*id);
        }
    }
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* binding = std::get_if<SystemBindingData>(&record.data);
            binding != nullptr && binding->world == world_id) {
            const auto* id = std::get_if<SystemBindingId>(&record.id);
            if (id != nullptr) RevokeBinding(*id);
        } else if (const auto* root = std::get_if<RootData>(&record.data);
                   root != nullptr && root->world == world_id) {
            const auto* id = std::get_if<RootAttributionId>(&record.id);
            if (id != nullptr) RevokeRoot(*id);
        } else if (const auto* registration_op = std::get_if<RegistrationOperationData>(&record.data);
                   registration_op != nullptr && registration_op->world == world_id) {
            RetireRecord(record);
        } else if (const auto* compile_op = std::get_if<CompileOperationData>(&record.data);
                   compile_op != nullptr && compile_op->world == world_id) {
            RetireRecord(record);
        } else if (const auto* world = std::get_if<WorldPublicationData>(&record.data);
                   world != nullptr && SameHandle(record.id, world_id)) {
            const auto context_id = world->context;
            associated_birth = world->birth;
            RetireRecord(record);
            if (auto* context = Find<ContextPublicationData>(context_id)) {
                context->current_world = {};
            }
        } else if (const auto* pending = std::get_if<WorldReservationData>(&record.data);
                   pending != nullptr && pending->birth != WorldBirthId{}) {
            // Pending reservations are closed by their matching world birth below.
        }
    }
    if (associated_birth != WorldBirthId{}) {
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            if (!record.active) continue;
            if (const auto* pending = std::get_if<WorldReservationData>(&record.data);
                pending != nullptr && pending->birth == associated_birth) {
                RetireRecord(record);
            } else if (const auto* birth = std::get_if<WorldBirthData>(&record.data);
                       birth != nullptr && SameHandle(record.id, associated_birth)) {
                RetireRecord(record);
            }
        }
    }
    InvalidateObservations();
}

void Ledger::Impl::RevokeContext(ContextPublicationId context_id) noexcept {
    if (context_id == ContextPublicationId{}) return;
    ContextBirthId associated_birth{};
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* world = std::get_if<WorldPublicationData>(&record.data);
            world != nullptr && world->context == context_id) {
            const auto* id = std::get_if<WorldPublicationId>(&record.id);
            if (id != nullptr) RevokeWorld(*id);
        }
    }
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* root = std::get_if<RootData>(&record.data);
            root != nullptr && root->context == context_id) {
            const auto* id = std::get_if<RootAttributionId>(&record.id);
            if (id != nullptr) RevokeRoot(*id);
        } else if (const auto* context = std::get_if<ContextPublicationData>(&record.data);
                   context != nullptr && SameHandle(record.id, context_id)) {
            associated_birth = context->birth;
            RetireRecord(record);
        }
    }
    if (associated_birth != ContextBirthId{}) {
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            if (!record.active) continue;
            if (const auto* pending = std::get_if<ContextReservationData>(&record.data);
                pending != nullptr && pending->birth == associated_birth) {
                RetireRecord(record);
            } else if (const auto* birth = std::get_if<ContextBirthData>(&record.data);
                       birth != nullptr && SameHandle(record.id, associated_birth)) {
                if (auto* setup = Find<ContextSetupData>(birth->setup)) setup->birth = {};
                RetireRecord(record);
            }
        }
    }
    InvalidateObservations();
}

void Ledger::Impl::RevokeNode(NodeBirthId node_id) noexcept {
    if (node_id == NodeBirthId{}) return;
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* pub = std::get_if<PublicationData>(&record.data);
            pub != nullptr && (pub->target == node_id || pub->parent == node_id)) {
            RetireRecord(record);
        } else if (const auto* claim = std::get_if<ClaimData>(&record.data);
                   claim != nullptr && claim->node == node_id) {
            RetireRecord(record);
        } else if (const auto* attempt = std::get_if<LinkAttemptData>(&record.data);
                   attempt != nullptr
                   && (attempt->source_child == node_id || attempt->target_parent == node_id)) {
            RetireRecord(record);
        } else if (const auto* submit = std::get_if<OriginalSubmitData>(&record.data);
                   submit != nullptr && submit->child == node_id) {
            RetireRecord(record);
        } else if (const auto* edge = std::get_if<TargetEdgeData>(&record.data);
                   edge != nullptr && (edge->parent == node_id || edge->child == node_id)) {
            RetireRecord(record);
        } else if (const auto* op = std::get_if<CreateOperationData>(&record.data);
                   op != nullptr && (op->parent == node_id || op->birth == node_id)) {
            RetireRecord(record);
        } else if (const auto* materialize = std::get_if<MaterializeData>(&record.data);
                   materialize != nullptr) {
            bool matches = materialize->parent == node_id || materialize->child == node_id;
            if (const auto* claim_id = std::get_if<ClaimId>(&materialize->source)) {
                const auto* active_claim = Find<ClaimData>(*claim_id);
                matches = matches || (active_claim != nullptr && active_claim->node == node_id);
            }
            if (matches) RetireRecord(record);
        } else if (const auto* birth = std::get_if<NodeBirthData>(&record.data);
                   birth != nullptr && SameHandle(record.id, node_id)) {
            RetireRecord(record);
        }
    }
    InvalidateObservations();
}

bool Ledger::Impl::RootIsOpen(RootAttributionId root_id) const noexcept {
    const auto* root = Find<RootData>(root_id);
    return root != nullptr && !root->bracket_closed;
}

void Ledger::Impl::CloseContextSetup(ContextSetupId setup_id) noexcept {
    auto* setup = Find<ContextSetupData>(setup_id);
    if (setup == nullptr) return;

    bool published = false;
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        const auto* context = std::get_if<ContextPublicationData>(&records[i].data);
        if (records[i].active && context != nullptr && context->setup == setup_id) {
            published = true;
            break;
        }
    }
    if (published) {
        setup->pass = {};
        setup->iteration = {};
        return;
    }

    const auto birth_id = setup->birth;
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        const auto* reservation = std::get_if<ContextReservationData>(&record.data);
        if (reservation != nullptr && reservation->setup == setup_id) {
            RetireRecord(record);
            continue;
        }
        const auto* birth = std::get_if<ContextBirthData>(&record.data);
        if (birth != nullptr && birth->setup == setup_id
            && (birth_id == ContextBirthId{} || SameHandle(record.id, birth_id))) {
            RetireRecord(record);
        }
    }
    if (auto* setup_record = FindRecord(setup_id)) RetireRecord(*setup_record);
}

void Ledger::Impl::CloseRootBracket(RootAttributionId root_id) noexcept {
    auto* root = Find<RootData>(root_id);
    if (root == nullptr) return;
    root->bracket_closed = true;
    root->pass = {};

    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* operation = std::get_if<CompileOperationData>(&record.data)) {
            const auto* scope = std::get_if<RootAttributionId>(&operation->scope);
            if (scope != nullptr && *scope == root_id) {
                RetireEvidence(std::get<CompileOpId>(record.id));
                RetireRecord(record);
            }
        }
    }
}

void Ledger::Impl::CloseIteration(RowIterationId iteration_id) noexcept {
    auto* iteration = Find<RowIterationData>(iteration_id);
    if (iteration == nullptr || iteration->ended) return;
    iteration->ended = true;
    iteration->pass = {};
    const auto reservation_id = iteration->reservation;
    iteration->reservation = {};

    if (auto* reservation_record = FindRecord(reservation_id)) RetireRecord(*reservation_record);
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* reservation = std::get_if<RowReservationData>(&record.data);
            reservation != nullptr && reservation->iteration == iteration_id) {
            RetireRecord(record);
        } else if (const auto* setup = std::get_if<ContextSetupData>(&record.data);
                   setup != nullptr && setup->iteration == iteration_id) {
            const auto* setup_id = std::get_if<ContextSetupId>(&record.id);
            if (setup_id != nullptr) CloseContextSetup(*setup_id);
        }
    }

    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* root = std::get_if<RootData>(&record.data);
            root != nullptr && root->iteration == iteration_id) {
            const auto* root_id = std::get_if<RootAttributionId>(&record.id);
            if (root_id != nullptr) CloseRootBracket(*root_id);
        } else if (const auto* operation = std::get_if<CompileOperationData>(&record.data)) {
            bool owns_iteration = false;
            if (const auto* row_scope = std::get_if<RowIterationId>(&operation->scope)) {
                owns_iteration = *row_scope == iteration_id;
            } else if (const auto* root_scope = std::get_if<RootAttributionId>(&operation->scope)) {
                const auto* scoped_root = Find<RootData>(*root_scope);
                owns_iteration = scoped_root != nullptr && scoped_root->iteration == iteration_id;
            }
            if (owns_iteration) {
                RetireEvidence(std::get<CompileOpId>(record.id));
                RetireRecord(record);
            }
        }
    }
}

void Ledger::Impl::ClosePass(RowPassId pass_id) noexcept {
    auto* pass = Find<RowPassData>(pass_id);
    if (pass == nullptr || !pass->open) return;
    pass->open = false;

    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* setup = std::get_if<ContextSetupData>(&record.data);
            setup != nullptr && setup->pass == pass_id) {
            const auto* setup_id = std::get_if<ContextSetupId>(&record.id);
            if (setup_id != nullptr) CloseContextSetup(*setup_id);
        } else if (const auto* iteration = std::get_if<RowIterationData>(&record.data);
                   iteration != nullptr && iteration->pass == pass_id) {
            const auto* iteration_id = std::get_if<RowIterationId>(&record.id);
            if (iteration_id != nullptr) CloseIteration(*iteration_id);
        } else if (const auto* reservation = std::get_if<RowReservationData>(&record.data);
                   reservation != nullptr && reservation->pass == pass_id) {
            const auto* iteration_id = std::get_if<RowIterationId>(&record.related);
            if (iteration_id != nullptr) CloseIteration(*iteration_id);
            RetireRecord(record);
        } else if (const auto* root = std::get_if<RootData>(&record.data);
                   root != nullptr && root->pass == pass_id) {
            const auto* root_id = std::get_if<RootAttributionId>(&record.id);
            if (root_id != nullptr) CloseRootBracket(*root_id);
        }
    }

    SweepClosedLifecycle();
}

bool Ledger::Impl::RootHasLiveDependents(RootAttributionId root_id) const noexcept {
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        const auto& record = records[i];
        if (!record.active) continue;
        if (const auto* node = std::get_if<NodeBirthData>(&record.data);
            node != nullptr && node->root == root_id) return true;
        if (const auto* publication = std::get_if<PublicationData>(&record.data);
            publication != nullptr && publication->root == root_id) return true;
        if (const auto* claim = std::get_if<ClaimData>(&record.data)) {
            const auto* node = Find<NodeBirthData>(claim->node);
            const auto* publication = Find<PublicationData>(claim->publication);
            if ((node != nullptr && node->root == root_id)
                || (publication != nullptr && publication->root == root_id)) return true;
        }
        if (const auto* materialize = std::get_if<MaterializeData>(&record.data)) {
            if (const auto* direct_root = std::get_if<RootAttributionId>(&materialize->source);
                direct_root != nullptr && *direct_root == root_id) return true;
            const auto* dispatch = Find<DispatchRecordData>(materialize->dispatch);
            if (dispatch != nullptr && dispatch->root == root_id) return true;
            const auto* publication = materialize->publication
                ? Find<PublicationData>(*materialize->publication) : nullptr;
            if (publication != nullptr && publication->root == root_id) return true;
            if (const auto* claim_id = std::get_if<ClaimId>(&materialize->source)) {
                const auto* claim = Find<ClaimData>(*claim_id);
                const auto* node = claim == nullptr ? nullptr : Find<NodeBirthData>(claim->node);
                const auto* claim_publication = claim == nullptr
                    ? nullptr : Find<PublicationData>(claim->publication);
                if ((node != nullptr && node->root == root_id)
                    || (claim_publication != nullptr && claim_publication->root == root_id)) return true;
            }
        }
    }
    return false;
}

void Ledger::Impl::SweepClosedLifecycle() noexcept {
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto* root = std::get_if<RootData>(&records[i].data);
        if (!records[i].active || root == nullptr || !root->bracket_closed) continue;
        const auto* root_id_ptr = std::get_if<RootAttributionId>(&records[i].id);
        if (root_id_ptr == nullptr || RootHasLiveDependents(*root_id_ptr)) continue;
        const auto root_id = *root_id_ptr;

        // Edges keep group handles only while their nodes are live. Reclaim
        // stale edges before deleting the owning groups.
        for (std::uint32_t j = 0; j < limits.max_records; ++j) {
            auto& candidate = records[j];
            const auto* edge = std::get_if<TargetEdgeData>(&candidate.data);
            const auto* group = edge == nullptr ? nullptr : Find<DispatchGroupData>(edge->group);
            if (candidate.active && edge != nullptr && group != nullptr && group->root == root_id) {
                RetireRecord(candidate);
            }
        }
        for (std::uint32_t j = 0; j < limits.max_records; ++j) {
            auto& candidate = records[j];
            if (!candidate.active) continue;
            if (const auto* create = std::get_if<CreateOperationData>(&candidate.data);
                create != nullptr && create->root == root_id) {
                RetireRecord(candidate);
            } else if (const auto* attempt = std::get_if<LinkAttemptData>(&candidate.data)) {
                bool matches = false;
                const auto* source_submit = Find<OriginalSubmitData>(attempt->submit);
                if (source_submit != nullptr && source_submit->root == root_id) matches = true;
                const auto* source_claim = Find<ClaimData>(attempt->source_claim);
                const auto* source_node = source_claim == nullptr
                    ? nullptr : Find<NodeBirthData>(source_claim->node);
                const auto* source_publication = source_claim == nullptr
                    ? nullptr : Find<PublicationData>(source_claim->publication);
                if ((source_node != nullptr && source_node->root == root_id)
                    || (source_publication != nullptr
                        && source_publication->root == root_id)) matches = true;
                if (matches) RetireRecord(candidate);
            } else if (const auto* submit = std::get_if<OriginalSubmitData>(&candidate.data);
                       submit != nullptr && submit->root == root_id) {
                RetireRecord(candidate);
            } else if (const auto* group = std::get_if<DispatchGroupData>(&candidate.data);
                       group != nullptr && group->root == root_id) {
                RetireRecord(candidate);
            } else if (const auto* dispatch = std::get_if<DispatchRecordData>(&candidate.data);
                       dispatch != nullptr && dispatch->root == root_id) {
                RetireRecord(candidate);
            } else if (const auto* claim = std::get_if<ClaimData>(&candidate.data)) {
                const auto* node = Find<NodeBirthData>(claim->node);
                const auto* publication = Find<PublicationData>(claim->publication);
                if ((node != nullptr && node->root == root_id)
                    || (publication != nullptr && publication->root == root_id)) {
                    RetireRecord(candidate);
                }
            } else if (const auto* materialize = std::get_if<MaterializeData>(&candidate.data)) {
                const auto* direct_root = std::get_if<RootAttributionId>(&materialize->source);
                const auto* materialize_dispatch = Find<DispatchRecordData>(materialize->dispatch);
                const auto* publication = materialize->publication
                    ? Find<PublicationData>(*materialize->publication) : nullptr;
                const auto* claim_id = std::get_if<ClaimId>(&materialize->source);
                const auto* materialize_claim = claim_id == nullptr
                    ? nullptr : Find<ClaimData>(*claim_id);
                const auto* source_node = materialize_claim == nullptr
                    ? nullptr : Find<NodeBirthData>(materialize_claim->node);
                const auto* source_publication = materialize_claim == nullptr
                    ? nullptr : Find<PublicationData>(materialize_claim->publication);
                if ((direct_root != nullptr && *direct_root == root_id)
                    || (materialize_dispatch != nullptr && materialize_dispatch->root == root_id)
                    || (publication != nullptr && publication->root == root_id)
                    || (source_node != nullptr && source_node->root == root_id)
                    || (source_publication != nullptr
                        && source_publication->root == root_id)) RetireRecord(candidate);
            }
        }
        if (auto* root_record = FindRecord(root_id)) RetireRecord(*root_record);
    }

    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        auto* iteration = std::get_if<RowIterationData>(&record.data);
        if (!record.active || iteration == nullptr || !iteration->ended) continue;
        const auto* iteration_id = std::get_if<RowIterationId>(&record.id);
        if (iteration_id == nullptr) continue;
        bool retained = false;
        for (std::uint32_t j = 0; j < limits.max_records; ++j) {
            const auto& candidate = records[j];
            if (!candidate.active) continue;
            if (const auto* root = std::get_if<RootData>(&candidate.data);
                root != nullptr && root->iteration == *iteration_id) retained = true;
            if (const auto* reservation = std::get_if<RowReservationData>(&candidate.data);
                reservation != nullptr && reservation->iteration == *iteration_id) retained = true;
            if (const auto* setup = std::get_if<ContextSetupData>(&candidate.data);
                setup != nullptr && setup->iteration == *iteration_id) retained = true;
            if (const auto* operation = std::get_if<CompileOperationData>(&candidate.data)) {
                if (const auto* row_scope = std::get_if<RowIterationId>(&operation->scope);
                    row_scope != nullptr && *row_scope == *iteration_id) retained = true;
            }
        }
        if (!retained) {
            RetireEvidence(*iteration_id);
            RetireRecord(record);
        }
    }

    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        const auto* pass = std::get_if<RowPassData>(&record.data);
        const auto* pass_id = std::get_if<RowPassId>(&record.id);
        if (!record.active || pass == nullptr || pass->open || pass_id == nullptr) continue;
        bool retained = false;
        for (std::uint32_t j = 0; j < limits.max_records; ++j) {
            const auto& candidate = records[j];
            if (!candidate.active) continue;
            if (const auto* iteration = std::get_if<RowIterationData>(&candidate.data);
                iteration != nullptr && iteration->pass == *pass_id) retained = true;
            if (const auto* reservation = std::get_if<RowReservationData>(&candidate.data);
                reservation != nullptr && reservation->pass == *pass_id) retained = true;
            if (const auto* setup = std::get_if<ContextSetupData>(&candidate.data);
                setup != nullptr && setup->pass == *pass_id) retained = true;
            if (const auto* root = std::get_if<RootData>(&candidate.data);
                root != nullptr && root->pass == *pass_id) retained = true;
        }
        if (!retained) RetireRecord(record);
    }
}

void Ledger::Impl::RevokeAddress(ScalarAddress address) noexcept {
    if (address == 0) return;
    for (std::uint32_t i = 0; i < limits.max_records; ++i) {
        auto& record = records[i];
        if (!record.active) continue;
        if (const auto* context = std::get_if<ContextPublicationData>(&record.data);
            context != nullptr && context->address == address) {
            const auto* id = std::get_if<ContextPublicationId>(&record.id);
            if (id != nullptr) RevokeContext(*id);
        } else if (const auto* birth = std::get_if<ContextBirthData>(&record.data);
                   birth != nullptr && birth->address == address) {
            if (auto* setup = Find<ContextSetupData>(birth->setup)) setup->birth = {};
            const auto birth_id = std::get<ContextBirthId>(record.id);
            for (std::uint32_t j = 0; j < limits.max_records; ++j) {
                auto& pending_record = records[j];
                const auto* pending = std::get_if<ContextReservationData>(&pending_record.data);
                if (pending_record.active && pending != nullptr && pending->birth == birth_id) {
                    RetireRecord(pending_record);
                }
            }
            RetireRecord(record);
        } else if (const auto* world = std::get_if<WorldPublicationData>(&record.data);
                   world != nullptr && (world->address == address
                       || world->manager == address || world->owner == address)) {
            const auto* id = std::get_if<WorldPublicationId>(&record.id);
            if (id != nullptr) RevokeWorld(*id);
        } else if (const auto* world_birth = std::get_if<WorldBirthData>(&record.data);
                   world_birth != nullptr && (world_birth->address == address || world_birth->manager == address
                       || world_birth->owner == address)) {
            const auto* id = std::get_if<WorldBirthId>(&record.id);
            if (id != nullptr) {
                for (std::uint32_t j = 0; j < limits.max_records; ++j) {
                    auto& pending_record = records[j];
                    const auto* pending = std::get_if<WorldReservationData>(&pending_record.data);
                    if (pending_record.active && pending != nullptr && pending->birth == *id) {
                        RetireRecord(pending_record);
                    }
                }
                RetireRecord(record);
            }
        } else if (const auto* node = std::get_if<NodeBirthData>(&record.data);
                   node != nullptr && node->address == address) {
            const auto* id = std::get_if<NodeBirthId>(&record.id);
            if (id != nullptr) RevokeNode(*id);
        } else if (const auto* compiled = std::get_if<CompiledData>(&record.data);
                   compiled != nullptr && compiled->schema == address) {
            const auto* id = std::get_if<CompileId>(&record.id);
            if (id != nullptr) RevokeCompile(*id);
        } else if (const auto* binding = std::get_if<SystemBindingData>(&record.data);
                   binding != nullptr && binding->record_slot == address) {
            const auto* id = std::get_if<SystemBindingId>(&record.id);
            if (id != nullptr) RevokeBinding(*id);
        } else if (const auto* dispatch = std::get_if<DispatchRecordData>(&record.data);
                   dispatch != nullptr && (dispatch->manager == address
                       || dispatch->record_address == address
                       || dispatch->system_alias == address)) {
            RevokeRoot(dispatch->root);
        } else if (const auto* root = std::get_if<RootData>(&record.data);
                   root != nullptr && root->root == address) {
            const auto* id = std::get_if<RootAttributionId>(&record.id);
            if (id != nullptr) RevokeRoot(*id);
        }
    }
    InvalidateObservations();
}

template <class T>
EventReply Ledger::Impl::Apply(Ledger& ledger,
                               const NativeReceipt& receipt,
                               const T& item) noexcept {
    const auto empty = [](EventStatus status) noexcept {
        return EventReply{status, {}, {}};
    };
    const auto one = [](const Record* record, EventStatus status) noexcept {
        return record == nullptr ? EventReply{status, {}, {}}
                                 : EventReply{status, record->id, record->related};
    };
    const auto record_failure = [](EventStatus status) noexcept {
        return EventReply{status, {}, {}};
    };
    if (closed) return empty(EventStatus::Refused);

    if constexpr (std::is_same_v<T, RowPassBegin>) {
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            const auto* pass = std::get_if<RowPassData>(&records[i].data);
            if (records[i].active && pass != nullptr && pass->open) return empty(EventStatus::Refused);
        }
        EventStatus failure{};
        auto* record = Allocate<RowPassId>(ledger, RowPassData{true}, failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, RowPassEnd>) {
        auto* pass = Find<RowPassData>(item.pass);
        auto* pass_record = FindRecord(item.pass);
        if (pass == nullptr || pass_record == nullptr || !pass->open) return empty(EventStatus::Refused);
        ClosePass(item.pass);
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, RowIterationEnd>) {
        auto* iteration = Find<RowIterationData>(item.iteration);
        auto* iteration_record = FindRecord(item.iteration);
        if (iteration == nullptr || iteration_record == nullptr || iteration->ended) {
            return empty(EventStatus::Refused);
        }
        CloseIteration(item.iteration);
        SweepClosedLifecycle();
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, RowReservationBegin>) {
        auto* pass = Find<RowPassData>(item.pass);
        if (pass == nullptr || !pass->open || item.row == 0) return empty(EventStatus::Refused);

        // A re-entry replaces the one pending reservation for this row pass.
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            const auto* pending = std::get_if<RowReservationData>(&record.data);
            if (record.active && pending != nullptr && pending->pass == item.pass) {
                const auto prior_iteration = pending->iteration;
                RetireEvidence(prior_iteration);
                RetireRecord(record);
                if (auto* old_iteration = FindRecord(prior_iteration)) RetireRecord(*old_iteration);
            }
        }

        ContextPublicationId snapshot_context{};
        WorldPublicationId snapshot_world{};
        std::uint32_t snapshot_count = 0;
        bool duplicate_context = false;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            const auto* context = std::get_if<ContextPublicationData>(&records[i].data);
            if (!records[i].active || context == nullptr || context->row != item.row) continue;
            const auto* id = std::get_if<ContextPublicationId>(&records[i].id);
            if (id == nullptr) continue;
            if (snapshot_context != ContextPublicationId{}) duplicate_context = true;
            snapshot_context = *id;
            snapshot_world = context->current_world;
        }
        if (duplicate_context) return empty(EventStatus::Refused);
        if (snapshot_world != WorldPublicationId{}
            && Find<WorldPublicationData>(snapshot_world) == nullptr) snapshot_world = {};

        if (snapshot_world != WorldPublicationId{}) {
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                const auto* compile = std::get_if<CompiledData>(&records[i].data);
                if (!records[i].active || compile == nullptr || compile->world != snapshot_world) continue;
                const auto* binding = Find<SystemBindingData>(compile->binding);
                if (binding != nullptr && binding->batch_committed) ++snapshot_count;
            }
        }
        if (snapshot_count > limits.max_compile_entries) return empty(EventStatus::CapacityExhausted);

        EventStatus failure{};
        auto* reservation_record = Allocate<ReservationId>(
            ledger,
            RowReservationData{item.pass, {}, item.row, snapshot_context, snapshot_world,
                               snapshot_count, false},
            failure);
        if (reservation_record == nullptr) return empty(failure);
        auto* iteration_record = Allocate<RowIterationId>(
            ledger,
            RowIterationData{item.pass, {}, item.row, snapshot_context, snapshot_world,
                             snapshot_count, false},
            failure);
        if (iteration_record == nullptr) {
            RetireRecord(*reservation_record);
            return empty(failure);
        }
        const auto reservation_id = std::get<ReservationId>(reservation_record->id);
        const auto iteration_id = std::get<RowIterationId>(iteration_record->id);
        reservation_record->related = iteration_id;
        std::get<RowReservationData>(reservation_record->data).iteration = iteration_id;
        std::get<RowIterationData>(iteration_record->data).reservation = reservation_id;

        if (snapshot_count != 0) {
            bool capacity_failure = false;
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                const auto* compile = std::get_if<CompiledData>(&records[i].data);
                if (!records[i].active || compile == nullptr || compile->world != snapshot_world) continue;
                const auto* binding = Find<SystemBindingData>(compile->binding);
                if (binding == nullptr || !binding->batch_committed) continue;
                auto* entry = AllocateEvidence(iteration_id, EvidenceKind::SnapshotCompile);
                if (entry == nullptr) {
                    capacity_failure = true;
                    break;
                }
                entry->first = compile->binding;
                entry->second = std::get<CompileId>(records[i].id);
            }
            if (capacity_failure) {
                RetireEvidence(iteration_id);
                RetireRecord(*reservation_record);
                RetireRecord(*iteration_record);
                return empty(EventStatus::CapacityExhausted);
            }
        }
        return {EventStatus::Pending, reservation_record->id, iteration_record->id};
    }
    else if constexpr (std::is_same_v<T, RowReservationComplete>) {
        auto* reservation = Find<RowReservationData>(item.reservation);
        auto* reservation_record = FindRecord(item.reservation);
        if (reservation == nullptr || reservation_record == nullptr || reservation->completed
            || reservation->row != item.row || reservation->snapshot_context == ContextPublicationId{}
            || item.context == 0) return empty(EventStatus::Refused);
        auto* iteration = Find<RowIterationData>(reservation->iteration);
        if (iteration == nullptr) return empty(EventStatus::Refused);
        const auto selected = reservation->snapshot_context;
        const auto* context = Find<ContextPublicationData>(selected);
        if (context == nullptr || context->row != item.row || context->address != item.context) {
            return empty(EventStatus::Refused);
        }
        reservation->completed = true;
        iteration->completed = true;
        iteration->context = selected;
        iteration->world = reservation->snapshot_world;
        RetireRecord(*reservation_record);
        return {EventStatus::Accepted, selected, {}};
    }
    else if constexpr (std::is_same_v<T, ContextSetupBegin>) {
        auto* pass = Find<RowPassData>(item.pass);
        if (pass == nullptr || !pass->open || item.row == 0) return empty(EventStatus::Refused);
        RowIterationId iteration_id{};
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            const auto* reservation = std::get_if<RowReservationData>(&records[i].data);
            if (!records[i].active || reservation == nullptr || reservation->pass != item.pass
                || reservation->row != item.row || reservation->completed
                || reservation->snapshot_context != ContextPublicationId{}
                || reservation->snapshot_world != WorldPublicationId{}) continue;
            if (iteration_id != RowIterationId{}) return empty(EventStatus::Refused);
            iteration_id = reservation->iteration;
        }
        if (iteration_id == RowIterationId{}) return empty(EventStatus::Refused);
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            auto* setup = std::get_if<ContextSetupData>(&record.data);
            if (record.active && setup != nullptr && setup->pass == item.pass && setup->row == item.row) {
                RetireRecord(record);
            }
        }
        EventStatus failure{};
        auto* record = Allocate<ContextSetupId>(
            ledger, ContextSetupData{item.pass, iteration_id, item.row, false, {}}, failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, ContextConstructBegin>) {
        bool matched_setup = false;
        if (item.setup) {
            auto* setup = Find<ContextSetupData>(*item.setup);
            if (setup != nullptr && setup->row != 0 && item.context_address != 0) {
                setup->constructor_started = true;
                matched_setup = true;
            }
        }
        RevokeAddress(item.context_address);
        return empty(matched_setup ? EventStatus::Accepted : EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, ContextBirthCaptured>) {
        auto* setup = Find<ContextSetupData>(item.setup);
        if (setup == nullptr || !setup->constructor_started || item.context == 0
            || !item.initialized) return empty(EventStatus::Incomplete);
        EventStatus failure{};
        auto* record = Allocate<ContextBirthId>(
            ledger,
            ContextBirthData{item.setup, item.context, receipt.Site(), receipt.FrameToken()},
            failure);
        if (record == nullptr) return empty(failure);
        setup->birth = std::get<ContextBirthId>(record->id);
        return one(record, EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, ContextPublicationBegin>) {
        // The native constructor return carries the actual row even when its
        // setup or birth is unknown. Close only publications on that row first.
        if (item.row != 0) {
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                const auto* context = std::get_if<ContextPublicationData>(&records[i].data);
                if (!records[i].active || context == nullptr || context->row != item.row) continue;
                const auto* id = std::get_if<ContextPublicationId>(&records[i].id);
                if (id != nullptr) RevokeContext(*id);
            }
        }
        auto* setup = Find<ContextSetupData>(item.setup);
        auto* birth = Find<ContextBirthData>(item.birth);
        if (setup == nullptr || birth == nullptr || setup->birth != item.birth
            || birth->setup != item.setup || birth->address == 0 || setup->row != item.row
            || !setup->constructor_started) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* record = Allocate<ReservationId>(
            ledger, ContextReservationData{item.setup, item.birth, item.row}, failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, ContextPublicationComplete>) {
        auto* reservation_record = FindRecord(item.reservation);
        auto* reservation = Find<ContextReservationData>(item.reservation);
        if (reservation_record == nullptr || reservation == nullptr || !item.native_store_completed
            || reservation->row != item.row) return empty(EventStatus::Incomplete);
        auto* setup = Find<ContextSetupData>(reservation->setup);
        auto* birth = Find<ContextBirthData>(reservation->birth);
        if (setup == nullptr || birth == nullptr || setup->birth != reservation->birth
            || birth->address != item.context || item.context == 0) {
            RetireRecord(*reservation_record);
            return empty(EventStatus::Refused);
        }
        const auto setup_id = reservation->setup;
        const auto setup_iteration = setup->iteration;
        ContextPublicationData publication{setup_id, reservation->birth,
                                           item.row, item.context, {}};
        if (!Promote<ContextPublicationId>(ledger, *reservation_record, publication)) {
            return empty(EventStatus::GenerationExhausted);
        }
        const auto publication_id = std::get<ContextPublicationId>(reservation_record->id);
        if (auto* committed_setup = Find<ContextSetupData>(setup_id)) {
            committed_setup->pass = {};
            committed_setup->iteration = {};
        }
        if (auto* initial_iteration = Find<RowIterationData>(setup_iteration);
            initial_iteration != nullptr
            && initial_iteration->row == item.row
            && initial_iteration->context == ContextPublicationId{}) {
            const auto initial_reservation = initial_iteration->reservation;
            if (auto* initial_reservation_record = FindRecord(initial_reservation)) {
                RetireRecord(*initial_reservation_record);
            }
            RetireEvidence(setup_iteration);
            if (auto* initial_iteration_record = FindRecord(setup_iteration)) {
                RetireRecord(*initial_iteration_record);
            }
        }
        return {EventStatus::Committed, publication_id, {}};
    }
    else if constexpr (std::is_same_v<T, ContextRetire>) {
        RevokeAddress(item.context);
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, RowContextClear>) {
        bool found = false;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            const auto* context = std::get_if<ContextPublicationData>(&records[i].data);
            if (!records[i].active || context == nullptr || context->row != item.row) continue;
            const auto* id = std::get_if<ContextPublicationId>(&records[i].id);
            if (id != nullptr) {
                RevokeContext(*id);
                found = true;
            }
        }
        return empty(found ? EventStatus::Revoked : EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, WorldSetupBegin>) {
        if (item.manager == 0 || item.owner == 0) return empty(EventStatus::Refused);
        // The native setup resets a world slot by its actual owner B before
        // constructing the replacement, including when the old slot was unbound.
        RevokeAddress(item.owner);
        if (!item.context) return empty(EventStatus::Revoked);
        const auto* context = Find<ContextPublicationData>(*item.context);
        if (context == nullptr) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* record = Allocate<WorldBirthId>(
            ledger, WorldBirthData{*item.context, item.manager, item.owner, 0, false}, failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, WorldBirthCaptured>) {
        auto* birth = Find<WorldBirthData>(item.candidate);
        if (birth == nullptr || item.world == 0 || !item.constructor_completed) {
            return empty(EventStatus::Incomplete);
        }
        RevokeAddress(item.world);
        birth = Find<WorldBirthData>(item.candidate);
        if (birth == nullptr) return empty(EventStatus::Refused);
        birth->address = item.world;
        birth->constructor_completed = true;
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, WorldPublicationBegin>) {
        // The store seam supplies the actual context scalar even when lineage
        // handles are absent. Retire matching World publications before trying
        // to prove positive birth/context ancestry; keep Context itself alive.
        if (item.context_address != 0) {
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                const auto* publication = std::get_if<WorldPublicationData>(&records[i].data);
                if (!records[i].active || publication == nullptr
                    || publication->context_address != item.context_address) continue;
                const auto* id = std::get_if<WorldPublicationId>(&records[i].id);
                if (id != nullptr) RevokeWorld(*id);
            }
        }
        auto* birth = Find<WorldBirthData>(item.world);
        auto* context = Find<ContextPublicationData>(item.context);
        if (birth == nullptr || context == nullptr || !birth->constructor_completed
            || birth->context != item.context || context->address != item.context_address
            || item.context_address == 0) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* record = Allocate<ReservationId>(
            ledger, WorldReservationData{item.world, item.context, item.context_address}, failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, WorldPublicationComplete>) {
        auto* reservation_record = FindRecord(item.reservation);
        auto* reservation = Find<WorldReservationData>(item.reservation);
        if (reservation_record == nullptr || reservation == nullptr || !item.native_store_completed) {
            return empty(EventStatus::Incomplete);
        }
        auto* birth = Find<WorldBirthData>(reservation->birth);
        auto* context = Find<ContextPublicationData>(reservation->context);
        if (birth == nullptr || context == nullptr || birth->address != item.world_address
            || reservation->context_address != item.context_address
            || context->address != item.context_address || item.world_address == 0) {
            RetireRecord(*reservation_record);
            return empty(EventStatus::Refused);
        }
        const auto context_id = reservation->context;
        WorldPublicationData publication{reservation->birth, context_id,
                                         item.context_address, item.world_address,
                                         birth->manager, birth->owner, 0, false};
        if (!Promote<WorldPublicationId>(ledger, *reservation_record, publication)) {
            return empty(EventStatus::GenerationExhausted);
        }
        const auto publication_id = std::get<WorldPublicationId>(reservation_record->id);
        context = Find<ContextPublicationData>(context_id);
        if (context != nullptr) context->current_world = publication_id;
        return {EventStatus::Committed, publication_id, {}};
    }
    else if constexpr (std::is_same_v<T, WorldRetire>) {
        if (item.address == 0) return empty(EventStatus::Refused);
        if (item.kind == WorldRetireKind::CommonOwnerDisposer) {
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                auto& record = records[i];
                if (!record.active) continue;
                if (const auto* publication = std::get_if<WorldPublicationData>(&record.data);
                    publication != nullptr && publication->owner == item.address) {
                    if (const auto* id = std::get_if<WorldPublicationId>(&record.id)) {
                        RevokeWorld(*id);
                    }
                } else if (const auto* birth = std::get_if<WorldBirthData>(&record.data);
                           birth != nullptr && birth->owner == item.address) {
                    const auto birth_id = std::get<WorldBirthId>(record.id);
                    for (std::uint32_t j = 0; j < limits.max_records; ++j) {
                        auto& pending_record = records[j];
                        const auto* pending = std::get_if<WorldReservationData>(&pending_record.data);
                        if (pending_record.active && pending != nullptr && pending->birth == birth_id) {
                            RetireRecord(pending_record);
                        }
                    }
                    RetireRecord(record);
                }
            }
            InvalidateObservations();
        } else {
            RevokeAddress(item.address);
        }
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, SchemaMutate>) {
        bool found = false;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            if (!record.active) continue;
            if (const auto* compiled = std::get_if<CompiledData>(&record.data);
                compiled != nullptr && compiled->schema == item.schema
                && (!item.world || compiled->world == *item.world)) {
                const auto* id = std::get_if<CompileId>(&record.id);
                if (id != nullptr) {
                    RevokeCompile(*id);
                    found = true;
                }
            } else if (const auto* op = std::get_if<CompileOperationData>(&record.data);
                       op != nullptr && op->schema == item.schema
                       && (!item.world || op->world == *item.world)) {
                RetireRecord(record);
                found = true;
            }
        }
        InvalidateObservations();
        return empty(found ? EventStatus::Revoked : EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, RegistrationBegin>) {
        auto* world = Find<WorldPublicationData>(item.world);
        if (world == nullptr || item.world_base == 0 || item.world_base != world->address
            || item.type_descriptor == 0 || item.disable != 0) return empty(EventStatus::Refused);

        if (!world->registration_batch_open) {
            const auto prior_generation = world->registration_batch_generation;
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                auto& record = records[i];
                if (!record.active) continue;
                if (const auto* pending = std::get_if<RegistrationOperationData>(&record.data);
                    pending != nullptr && pending->world == item.world
                    && pending->batch_generation == prior_generation) {
                    if (pending->binding != SystemBindingId{}) RevokeBinding(pending->binding);
                    RetireRecord(record);
                } else if (const auto* candidate = std::get_if<SystemBindingData>(&record.data);
                           candidate != nullptr && candidate->world == item.world
                           && candidate->batch_generation == prior_generation) {
                    const auto* candidate_id = std::get_if<SystemBindingId>(&record.id);
                    if (candidate_id != nullptr && !candidate->batch_committed) RevokeBinding(*candidate_id);
                }
            }
            if (world->registration_batch_generation == std::numeric_limits<std::uint64_t>::max()) {
                generation_exhausted = true;
                return empty(EventStatus::GenerationExhausted);
            }
            ++world->registration_batch_generation;
            if (world->registration_batch_generation == 0) {
                generation_exhausted = true;
                return empty(EventStatus::GenerationExhausted);
            }
            world->registration_batch_open = true;
        }

        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            if (!record.active) continue;
            if (const auto* pending = std::get_if<RegistrationOperationData>(&record.data);
                pending != nullptr && pending->world == item.world
                && pending->type_descriptor == item.type_descriptor
                && pending->thread == receipt.ThreadToken()
                && pending->frame == receipt.FrameToken()) {
                if (pending->binding != SystemBindingId{}) RevokeBinding(pending->binding);
                RetireRecord(record);
            }
        }
        EventStatus failure{};
        auto* record = Allocate<RegistrationOpId>(
            ledger,
            RegistrationOperationData{item.world, item.world_base, item.type_descriptor,
                                      item.disable, world->registration_batch_generation, {},
                                      receipt.ThreadToken(), receipt.FrameToken()},
            failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, RegistrationStore>) {
        auto* operation = Find<RegistrationOperationData>(item.operation);
        if (operation == nullptr || operation->binding != SystemBindingId{}
            || item.record_slot == 0 || item.type_descriptor != operation->type_descriptor
            || item.disable != operation->disable || item.ordinal == 0
            || item.ordinal > limits.max_registration_entries
            || operation->thread != receipt.ThreadToken()
            || operation->frame != receipt.FrameToken()) return empty(EventStatus::Refused);
        ScalarAddress expected_slot{};
        if (!RegistrationSlot(operation->world_base, item.ordinal, expected_slot)
            || item.record_slot != expected_slot) return empty(EventStatus::Refused);
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            const auto* existing = std::get_if<SystemBindingData>(&record.data);
            if (!record.active || existing == nullptr || existing->world != operation->world
                || existing->record_slot != item.record_slot) continue;
            const auto* id = std::get_if<SystemBindingId>(&record.id);
            if (id != nullptr) RevokeBinding(*id);
        }
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            const auto* existing = std::get_if<SystemBindingData>(&records[i].data);
            if (!records[i].active || existing == nullptr
                || existing->world != operation->world
                || existing->batch_generation != operation->batch_generation) continue;
            if (existing->ordinal == item.ordinal || existing->record_slot == item.record_slot) {
                return empty(EventStatus::Refused);
            }
        }
        const auto world_id = operation->world;
        const auto world_base = operation->world_base;
        const auto type_descriptor = operation->type_descriptor;
        const auto batch_generation = operation->batch_generation;
        EventStatus failure{};
        auto* binding_record = Allocate<SystemBindingId>(
            ledger,
            SystemBindingData{world_id, world_base, item.record_slot, type_descriptor,
                              item.callback, item.flags_one, item.flags_two, item.ordinal,
                              batch_generation, item.operation, false},
            failure);
        if (binding_record == nullptr) return empty(failure);
        const auto binding_id = std::get<SystemBindingId>(binding_record->id);
        operation = Find<RegistrationOperationData>(item.operation);
        if (operation == nullptr) {
            RetireRecord(*binding_record);
            return empty(EventStatus::Refused);
        }
        operation->binding = binding_id;
        return {EventStatus::Pending, binding_id, {}};
    }
    else if constexpr (std::is_same_v<T, WorldRegistrationBatchComplete>) {
        auto* world = Find<WorldPublicationData>(item.world);
        if (world == nullptr || world->address != item.world_base || !world->registration_batch_open
            || item.observed_count == 0
            || item.observed_count > limits.max_registration_entries) {
            return empty(EventStatus::Incomplete);
        }
        const auto batch_generation = world->registration_batch_generation;
        std::uint32_t candidate_count = 0;
        std::uint32_t operation_count = 0;
        bool complete = true;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            if (!records[i].active) continue;
            const auto* operation = std::get_if<RegistrationOperationData>(&records[i].data);
            if (operation != nullptr && operation->world == item.world
                && operation->world_base == item.world_base
                && operation->batch_generation == batch_generation) {
                ++operation_count;
                complete = complete && operation->binding != SystemBindingId{};
            }
        }
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            if (!records[i].active) continue;
            if (const auto* binding = std::get_if<SystemBindingData>(&records[i].data);
                binding != nullptr && binding->world == item.world
                && binding->world_base == item.world_base
                && binding->batch_generation == batch_generation) {
                ++candidate_count;
                const auto* binding_id = std::get_if<SystemBindingId>(&records[i].id);
                bool has_original_operation = false;
                for (std::uint32_t j = 0; j < limits.max_records; ++j) {
                    const auto* pending = std::get_if<RegistrationOperationData>(&records[j].data);
                    if (records[j].active && pending != nullptr && binding_id != nullptr
                        && pending->world == item.world && pending->batch_generation == batch_generation
                        && pending->binding == *binding_id) {
                        has_original_operation = true;
                        break;
                    }
                }
                complete = complete && has_original_operation;
            }
        }
        complete = complete && item.native_completed
            && candidate_count == item.observed_count
            && operation_count == item.observed_count;
        for (std::uint32_t ordinal = 1; complete && ordinal <= item.observed_count; ++ordinal) {
            std::uint32_t matches = 0;
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                if (!records[i].active) continue;
                const auto* binding = std::get_if<SystemBindingData>(&records[i].data);
                if (binding != nullptr && binding->world == item.world
                    && binding->world_base == item.world_base
                    && binding->batch_generation == batch_generation
                    && binding->ordinal == ordinal) ++matches;
            }
            complete = matches == 1;
        }
        world = Find<WorldPublicationData>(item.world);
        if (world != nullptr) world->registration_batch_open = false;
        if (!complete) {
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                auto& record = records[i];
                if (!record.active) continue;
                if (const auto* candidate = std::get_if<SystemBindingData>(&record.data);
                    candidate != nullptr && candidate->world == item.world
                    && candidate->batch_generation == batch_generation) {
                    const auto* id = std::get_if<SystemBindingId>(&record.id);
                    if (id != nullptr) RevokeBinding(*id);
                } else if (const auto* pending = std::get_if<RegistrationOperationData>(&record.data);
                           pending != nullptr && pending->world == item.world
                           && pending->batch_generation == batch_generation) {
                    RetireRecord(record);
                }
            }
            return empty(EventStatus::Incomplete);
        }
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            if (!records[i].active) continue;
            auto* binding = std::get_if<SystemBindingData>(&records[i].data);
            if (binding != nullptr && binding->world == item.world
                && binding->world_base == item.world_base
                && binding->batch_generation == batch_generation) {
                binding->batch_committed = true;
            } else if (auto* operation = std::get_if<RegistrationOperationData>(&records[i].data);
                       operation != nullptr && operation->world == item.world
                       && operation->batch_generation == batch_generation) {
                RetireRecord(records[i]);
            }
        }
        return empty(EventStatus::Committed);
    }
    else if constexpr (std::is_same_v<T, RegistrationRetire>) {
        RevokeAddress(item.record_slot);
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, CompileAssociationsRetire>) {
        if (item.schema == 0) return empty(EventStatus::Refused);
        bool found = false;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            if (!record.active) continue;
            if (const auto* compiled = std::get_if<CompiledData>(&record.data);
                compiled != nullptr && compiled->schema == item.schema) {
                const auto* id = std::get_if<CompileId>(&record.id);
                if (id != nullptr) {
                    RevokeCompile(*id);
                    found = true;
                }
            } else if (const auto* pending = std::get_if<CompileOperationData>(&record.data);
                       pending != nullptr && pending->schema == item.schema) {
                RetireEvidence(std::get<CompileOpId>(record.id));
                RetireRecord(record);
                found = true;
            }
        }
        InvalidateObservations();
        return empty(found ? EventStatus::Revoked : EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, CompileBegin>) {
        const auto* world = Find<WorldPublicationData>(item.world);
        const auto* binding = Find<SystemBindingData>(item.binding);
        if (world == nullptr || binding == nullptr || !binding->batch_committed
            || binding->world != item.world || item.schema == 0) return empty(EventStatus::Refused);
        bool scope_valid = false;
        if (const auto* row = std::get_if<RowIterationId>(&item.owner_scope)) {
            const auto* row_data = Find<RowIterationData>(*row);
            scope_valid = row_data != nullptr && row_data->completed && !row_data->ended
                && row_data->context != ContextPublicationId{}
                && Find<ContextPublicationData>(row_data->context) != nullptr;
        } else if (const auto* root_id = std::get_if<RootAttributionId>(&item.owner_scope)) {
            const auto* root = Find<RootData>(*root_id);
            scope_valid = root != nullptr && !root->bracket_closed && root->world == item.world;
        }
        if (!scope_valid) return empty(EventStatus::Refused);
        if ((item.expected_read_entries && *item.expected_read_entries > limits.max_compile_entries)
            || (item.expected_write_entries && *item.expected_write_entries > limits.max_compile_entries)) {
            return empty(EventStatus::CapacityExhausted);
        }

        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            if (!record.active) continue;
            if (const auto* compiled = std::get_if<CompiledData>(&record.data);
                compiled != nullptr && compiled->world == item.world
                && compiled->binding == item.binding && compiled->schema == item.schema) {
                const auto* id = std::get_if<CompileId>(&record.id);
                if (id != nullptr) RevokeCompile(*id);
            } else if (const auto* pending = std::get_if<CompileOperationData>(&record.data);
                       pending != nullptr && pending->world == item.world
                       && pending->binding == item.binding && pending->schema == item.schema) {
                RetireEvidence(std::get<CompileOpId>(record.id));
                RetireRecord(record);
            }
        }
        EventStatus failure{};
        auto* record = Allocate<CompileOpId>(
            ledger,
            CompileOperationData{item.world, item.binding, item.owner_scope, item.schema,
                                 item.expected_read_entries, item.expected_write_entries,
                                 0, 0, 0, 0, false, false},
            failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, CompileHashCaptured>) {
        auto* operation = Find<CompileOperationData>(item.operation);
        const bool direction_valid = item.direction == HashDirection::Read
            || item.direction == HashDirection::Write;
        const std::uint32_t expected_group =
            item.direction == HashDirection::Read ? 0U : 1U;
        if (operation == nullptr || !direction_valid || item.exact_slot == 0 || item.ordinal == 0
            || item.group != expected_group || (item.flags & 0xFFFF0000U) != 0) {
            return empty(EventStatus::Refused);
        }
        SemanticKey semantic_key{};
        bool catalog_match = false;
        for (const auto& descriptor : catalog.entries) {
            if (descriptor.semantic_hash == item.hash) {
                semantic_key = descriptor.semantic_key;
                catalog_match = true;
                break;
            }
        }
        if (!catalog_match) return empty(EventStatus::Refused);
        const auto count = operation->read_entries + operation->write_entries;
        if (count >= limits.max_compile_entries) return empty(EventStatus::CapacityExhausted);
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            const auto& existing = evidence[i];
            if (existing.active && existing.kind == EvidenceKind::CompileQuery
                && SameHandle(existing.owner, item.operation)
                && existing.address == item.exact_slot
                && existing.group == item.group
                && existing.direction == item.direction) {
                return empty(EventStatus::Refused);
            }
        }
        auto* entry = AllocateEvidence(item.operation, EvidenceKind::CompileQuery);
        if (entry == nullptr) return empty(EventStatus::CapacityExhausted);
        entry->address = item.exact_slot;
        entry->hash = item.hash;
        entry->key = semantic_key;
        entry->ordinal = item.ordinal;
        entry->group = item.group;
        entry->flags = item.flags;
        entry->direction = item.direction;
        entry->optional = item.optional;
        if (item.direction == HashDirection::Read) ++operation->read_entries;
        else ++operation->write_entries;
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, CompileCountCaptured>) {
        auto* operation = Find<CompileOperationData>(item.operation);
        if (operation == nullptr || item.count > limits.max_compile_entries) return empty(EventStatus::Refused);
        if (item.direction == HashDirection::Read) {
            if (operation->saw_read_count) return empty(EventStatus::Refused);
            operation->read_count = item.count;
            operation->saw_read_count = true;
        } else {
            if (operation->saw_write_count) return empty(EventStatus::Refused);
            operation->write_count = item.count;
            operation->saw_write_count = true;
        }
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, CompileComplete>) {
        auto* record = FindRecord(item.operation);
        auto* operation = Find<CompileOperationData>(item.operation);
        if (record == nullptr || operation == nullptr || !item.native_completed) {
            return empty(EventStatus::Incomplete);
        }
        if (!operation->saw_read_count || !operation->saw_write_count
            || operation->read_count != item.read_count || operation->write_count != item.write_count
            || operation->read_count != operation->read_entries
            || operation->write_count != operation->write_entries
            || (operation->expected_read && *operation->expected_read != item.read_count)
            || (operation->expected_write && *operation->expected_write != item.write_count)) {
            RetireEvidence(item.operation);
            RetireRecord(*record);
            return empty(EventStatus::Incomplete);
        }
        CompiledData compiled{operation->world, operation->binding, operation->scope,
                              operation->schema, item.read_count, item.write_count};
        if (!Promote<CompileId>(ledger, *record, compiled)) return empty(EventStatus::GenerationExhausted);
        return {EventStatus::Committed, record->id, {}};
    }
    else if constexpr (std::is_same_v<T, RootAttributionBegin>) {
        const auto* pass = Find<RowPassData>(item.row_pass);
        const auto* iteration = Find<RowIterationData>(item.iteration);
        const auto* context = Find<ContextPublicationData>(item.context);
        const auto* world = Find<WorldPublicationData>(item.world);
        const auto* pass_record = FindRecord(item.row_pass);
        if (pass == nullptr || pass_record == nullptr || !pass->open
            || iteration == nullptr || !iteration->completed || iteration->ended
            || iteration->pass != item.row_pass
            || iteration->context != item.context || iteration->world != item.world
            || context == nullptr || world == nullptr || world->context != item.context
            || world->address != item.actual_world_scalar || item.actual_world_scalar == 0
            || item.root == 0 || item.original_frame == 0
            || item.original_frame != receipt.FrameToken()) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* record = Allocate<RootAttributionId>(
            ledger,
            RootData{item.row_pass, item.iteration, item.context, item.world,
                     item.root, item.actual_world_scalar, item.original_frame},
            failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, DispatchGroupBegin>) {
        const auto* root = Find<RootData>(item.root);
        if (root == nullptr || root->bracket_closed || item.dispatch_group == 0 || item.original_frame == 0
            || item.original_frame != root->frame || item.original_frame != receipt.FrameToken()) {
            return empty(EventStatus::Refused);
        }
        EventStatus failure{};
        auto* record = Allocate<DispatchGroupId>(
            ledger, DispatchGroupData{item.root, item.dispatch_group, item.original_frame, {}}, failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, DispatchRecordCaptured>) {
        const auto* root = Find<RootData>(item.root);
        if (root == nullptr || root->bracket_closed || item.record_address == 0
            || !CompileEligible(*root, item.root, item.compile, item.binding)
            || !BindingContainsAlias(item.binding, item.system_alias)) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* record = Allocate<DispatchRecordId>(
            ledger,
            DispatchRecordData{item.root, item.compile, item.binding, item.system_alias,
                               item.record_address, 0, item.store_path, {}, false},
            failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, DispatchRecordAssociated>) {
        auto* group = Find<DispatchGroupData>(item.group);
        auto* dispatch = Find<DispatchRecordData>(item.record);
        const auto* root = dispatch == nullptr ? nullptr : Find<RootData>(dispatch->root);
        if (group == nullptr || dispatch == nullptr || root == nullptr || root->bracket_closed
            || dispatch->associated
            || group->root != dispatch->root || group->record_count >= kMaxDispatchRecords
            || item.manager == 0 || item.manager != root->root
            || item.record_address != dispatch->record_address
            || item.system_alias != dispatch->system_alias) return empty(EventStatus::Refused);
        if (group->record_count != 0) {
            const auto* representative = Find<DispatchRecordData>(group->record);
            if (representative == nullptr || !representative->associated
                || representative->group != item.group
                || representative->root != dispatch->root
                || representative->compile != dispatch->compile
                || representative->binding != dispatch->binding
                || representative->system_alias != dispatch->system_alias
                || representative->manager != item.manager
                || group->compile != dispatch->compile
                || group->binding != dispatch->binding
                || group->system_alias != dispatch->system_alias
                || group->manager != item.manager) return empty(EventStatus::Refused);
        } else if (group->record != DispatchRecordId{}) {
            return empty(EventStatus::Refused);
        }
        dispatch->manager = item.manager;
        dispatch->group = item.group;
        dispatch->associated = true;
        if (group->record_count == 0) {
            group->compile = dispatch->compile;
            group->binding = dispatch->binding;
            group->system_alias = dispatch->system_alias;
            group->manager = item.manager;
        }
        group->record = item.record;
        ++group->record_count;
        return empty(EventStatus::Committed);
    }
    else if constexpr (std::is_same_v<T, ParentCreateBegin>) {
        const auto* root = Find<RootData>(item.root);
        const auto* group = Find<DispatchGroupData>(item.group);
        const auto* dispatch = group == nullptr ? nullptr : Find<DispatchRecordData>(group->record);
        if (root == nullptr || root->bracket_closed || group == nullptr || dispatch == nullptr
            || group->root != item.root || !dispatch->associated || dispatch->group != item.group
            || item.descriptor == 0 || item.callback != kApprovedParentCreator
            || item.descriptor_parent != 0) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* record = Allocate<CreateOpId>(
            ledger,
            CreateOperationData{NodeRole::Parent, item.root, item.group, {}, {},
                                item.descriptor, item.callback, item.descriptor_parent,
                                {}, false, false, false},
            failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, DescriptorTargetAssigned>) {
        const auto* group = Find<DispatchGroupData>(item.group);
        if (group == nullptr || group->record == DispatchRecordId{}
            || !RootIsOpen(group->root)) return empty(EventStatus::Refused);
        NodeBirthId parent_id{};
        ScalarAddress expected_target{};
        if (item.target_parent) {
            const auto* parent = Find<NodeBirthData>(*item.target_parent);
            if (parent == nullptr || parent->role != NodeRole::Parent
                || parent->group != item.group || parent->root != group->root) return empty(EventStatus::Refused);
            parent_id = *item.target_parent;
            expected_target = parent->address;
        }
        if (item.actual_target != expected_target) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* record = Allocate<TargetEdgeId>(
            ledger, TargetEdgeData{item.group, parent_id, item.actual_target, {}, false}, failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, NodeCreateBegin>) {
        const auto* root = Find<RootData>(item.root);
        const auto* group = Find<DispatchGroupData>(item.group);
        const auto* dispatch = group == nullptr ? nullptr : Find<DispatchRecordData>(group->record);
        const auto* parent = Find<NodeBirthData>(item.parent);
        const auto* edge = Find<TargetEdgeData>(item.target_edge);
        if (root == nullptr || root->bracket_closed || group == nullptr
            || dispatch == nullptr || parent == nullptr
            || edge == nullptr || group->root != item.root || parent->root != item.root
            || parent->group != item.group || parent->role != NodeRole::Parent
            || edge->group != item.group || edge->parent != item.parent || edge->target != parent->address
            || !dispatch->associated || dispatch->group != item.group
            || item.creator_callback != kWorkloadCallback || item.descriptor == 0) {
            return empty(EventStatus::Refused);
        }
        EventStatus failure{};
        auto* record = Allocate<CreateOpId>(
            ledger,
            CreateOperationData{NodeRole::Child, item.root, item.group, item.parent,
                                item.target_edge, item.descriptor, item.creator_callback,
                                edge->target, {}, false, false, false},
            failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, NodeBirthCaptured>) {
        auto* create = Find<CreateOperationData>(item.operation);
        if (create == nullptr || create->allocated || !item.allocation_succeeded
            || item.node == 0 || item.owner == 0) return empty(EventStatus::Refused);
        const auto role = create->role;
        const auto root_id = create->root;
        const auto group_id = create->group;
        const auto parent_id = create->parent;
        const auto edge_id = create->edge;
        const auto* root = Find<RootData>(root_id);
        const auto* group = Find<DispatchGroupData>(group_id);
        const auto* dispatch = group == nullptr ? nullptr : Find<DispatchRecordData>(group->record);
        if (root == nullptr || root->bracket_closed || group == nullptr
            || dispatch == nullptr || !dispatch->associated) {
            return empty(EventStatus::Refused);
        }
        RevokeAddress(item.node);
        create = Find<CreateOperationData>(item.operation);
        if (create == nullptr) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* node_record = Allocate<NodeBirthId>(
            ledger,
            NodeBirthData{role, item.node, receipt.FrameToken(), item.owner,
                          root_id, group_id, parent_id, edge_id,
                          root->context, root->world, dispatch->compile, dispatch->binding,
                          group->record, {}, 0, 0, false, false, false, false, false},
            failure);
        if (node_record == nullptr) return empty(failure);
        const auto node_id = std::get<NodeBirthId>(node_record->id);
        OriginalSubmitId submit_id{};
        if (role == NodeRole::Child) {
            auto* submit_record = Allocate<OriginalSubmitId>(
                ledger, OriginalSubmitData{node_id, root_id, group_id, 0, {}, false, false}, failure);
            if (submit_record == nullptr) {
                RetireRecord(*node_record);
                return empty(failure);
            }
            submit_id = std::get<OriginalSubmitId>(submit_record->id);
            std::get<NodeBirthData>(node_record->data).submit = submit_id;
        }
        create = Find<CreateOperationData>(item.operation);
        if (create == nullptr) {
            RevokeNode(node_id);
            return empty(EventStatus::Refused);
        }
        create->birth = node_id;
        create->allocated = true;
        node_record->related = submit_id;
        if (role == NodeRole::Child) return {EventStatus::Accepted, node_id, submit_id};
        return {EventStatus::Accepted, node_id, {}};
    }
    else if constexpr (std::is_same_v<T, ParentCreatorReturned>) {
        auto* create = Find<CreateOperationData>(item.operation);
        auto* parent = create == nullptr ? nullptr : Find<NodeBirthData>(create->birth);
        if (create == nullptr || parent == nullptr || create->role != NodeRole::Parent
            || !create->allocated || !create->payload_complete
            || item.parent != parent->address || item.callback != kApprovedParentCreator
            || create->callback != item.callback || item.descriptor_parent != 0
            || create->descriptor_parent != item.descriptor_parent) return empty(EventStatus::Refused);
        parent->creator_returned = true;
        create->creator_returned = true;
        RetireRecord(*FindRecord(item.operation));
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, ChildCreatorReturned>) {
        auto* create = Find<CreateOperationData>(item.operation);
        auto* child = Find<NodeBirthData>(item.child);
        if (create == nullptr || child == nullptr || create->role != NodeRole::Child
            || create->birth != item.child || !create->allocated || !create->payload_complete
            || item.returned_child != child->address || !child->target_stored
            || !child->payload_complete) return empty(EventStatus::Refused);
        child->creator_returned = true;
        create->creator_returned = true;
        RetireRecord(*FindRecord(item.operation));
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, ChildTargetEdgeCaptured>) {
        auto* create = Find<CreateOperationData>(item.operation);
        auto* edge = Find<TargetEdgeData>(item.edge);
        auto* parent = Find<NodeBirthData>(item.parent);
        auto* child = Find<NodeBirthData>(item.child);
        if (create == nullptr || edge == nullptr || parent == nullptr || child == nullptr
            || create->role != NodeRole::Child || create->edge != item.edge
            || create->parent != item.parent || create->birth != item.child
            || edge->parent != item.parent || edge->group != create->group
            || parent->role != NodeRole::Parent || child->role != NodeRole::Child
            || edge->target != item.descriptor_target || parent->address != item.descriptor_target
            || item.loaded_target != item.descriptor_target
            || item.stored_target != item.descriptor_target) return empty(EventStatus::Refused);
        edge->child = item.child;
        edge->child_stored = true;
        child->target_stored = true;
        return empty(EventStatus::Committed);
    }
    else if constexpr (std::is_same_v<T, ChildPayloadCaptured>) {
        auto* child = Find<NodeBirthData>(item.node);
        const auto* group = child == nullptr ? nullptr : Find<DispatchGroupData>(child->group);
        const auto* root = child == nullptr ? nullptr : Find<RootData>(child->root);
        const auto* representative = child == nullptr
            ? nullptr : Find<DispatchRecordData>(child->dispatch);
        if (child == nullptr || child->role != NodeRole::Child || item.owner != child->owner
            || child->payload_captured || group == nullptr || root == nullptr
            || root->bracket_closed
            || representative == nullptr || !representative->associated
            || representative->group != child->group || group->root != child->root
            || group->record_count == 0 || group->record_count > kMaxDispatchRecords
            || item.dispatch_record_count == 0
            || item.dispatch_record_count > kMaxDispatchRecords
            || item.dispatch_record_count != group->record_count
            || item.payload_low == 0 || item.payload_low != group->manager
            || item.payload_low != root->root || item.payload_high == 0
            || group->compile != child->compile || group->binding != child->binding
            || group->system_alias != representative->system_alias
            || !BindingContainsAlias(child->binding, group->system_alias)) {
            return empty(EventStatus::Refused);
        }
        for (std::uint32_t i = 0; i < item.dispatch_record_count; ++i) {
            const auto record_id = item.dispatch_records[i];
            const auto* dispatch = Find<DispatchRecordData>(record_id);
            ScalarAddress expected_address{};
            if (record_id == DispatchRecordId{} || dispatch == nullptr
                || !dispatch->associated || dispatch->root != child->root
                || dispatch->group != child->group || dispatch->compile != child->compile
                || dispatch->binding != child->binding
                || dispatch->system_alias != group->system_alias
                || dispatch->manager != item.payload_low
                || !MulAddAddress(item.payload_high, i, 0x38, expected_address)
                || dispatch->record_address != expected_address) {
                return empty(EventStatus::Refused);
            }
            for (std::uint32_t previous = 0; previous < i; ++previous) {
                if (item.dispatch_records[previous] == record_id) {
                    return empty(EventStatus::Refused);
                }
            }
        }
        for (std::uint32_t i = item.dispatch_record_count;
             i < item.dispatch_records.size(); ++i) {
            if (item.dispatch_records[i] != DispatchRecordId{}) {
                return empty(EventStatus::Refused);
            }
        }
        child->payload_low = item.payload_low;
        child->payload_high = item.payload_high;
        child->dispatch_record_count = item.dispatch_record_count;
        child->dispatch_records = item.dispatch_records;
        child->payload_captured = true;
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, NodePayloadCompleted>) {
        auto* node = Find<NodeBirthData>(item.node);
        if (node == nullptr || !RootIsOpen(node->root) || !item.native_completed
            || (node->role == NodeRole::Child && (!node->payload_captured || !node->target_stored))) {
            return empty(EventStatus::Incomplete);
        }
        node->payload_complete = true;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto* candidate = std::get_if<CreateOperationData>(&records[i].data);
            if (records[i].active && candidate != nullptr && candidate->birth == item.node) {
                candidate->payload_complete = true;
            }
        }
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, RootKickAssigned>) {
        auto* submit = Find<OriginalSubmitData>(item.submit);
        auto* child = submit == nullptr ? nullptr : Find<NodeBirthData>(submit->child);
        auto* root = Find<RootData>(item.root);
        if (submit == nullptr || child == nullptr || root == nullptr || root->bracket_closed
            || submit->root != item.root
            || submit->group != item.group || child->role != NodeRole::Child
            || child->root != item.root || child->group != item.group
            || child->submit != item.submit || !child->payload_complete
            || !child->target_stored || !child->creator_returned
            || item.child != submit->child || item.stored_child != child->address
            || item.stack_slot == 0 || item.callback != kWorkloadCallback
            || item.r15 != 0 || item.native_pending_count != 1
            || item.slot_index >= kRootKickSlotLimit) return empty(EventStatus::Refused);

        // Replacing the same root stack slot retires the earlier one-shot source.
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& record = records[i];
            auto* previous = std::get_if<OriginalSubmitData>(&record.data);
            if (!record.active || previous == nullptr || SameHandle(record.id, item.submit)
                || previous->root != item.root || !previous->root_kicked
                || previous->stack_slot != item.stack_slot) continue;
            if (previous->attempt != LinkAttemptId{}) {
                if (auto* attempt_record = FindRecord(previous->attempt)) RetireRecord(*attempt_record);
            }
            RetireRecord(record);
        }
        submit = Find<OriginalSubmitData>(item.submit);
        child = submit == nullptr ? nullptr : Find<NodeBirthData>(submit->child);
        if (submit == nullptr || child == nullptr) return empty(EventStatus::Refused);
        submit->stack_slot = item.stack_slot;
        submit->root_kicked = true;
        child->root_kicked = true;
        return empty(EventStatus::Committed);
    }
    else if constexpr (std::is_same_v<T, RootSubmitTailReturned>) {
        auto* submit = Find<OriginalSubmitData>(item.submit);
        if (submit == nullptr || !RootIsOpen(submit->root)
            || !submit->root_kicked || submit->tail_returned) {
            return empty(EventStatus::Refused);
        }
        submit->tail_returned = true;
        if (!item.attempt) return empty(EventStatus::Accepted);
        auto* attempt_record = FindRecord(*item.attempt);
        auto* attempt = Find<LinkAttemptData>(*item.attempt);
        if (attempt_record == nullptr || attempt == nullptr || attempt->submit != item.submit) {
            return empty(EventStatus::Refused);
        }
        if (attempt->publication != PublicationId{} && attempt->normal_post) {
            return empty(EventStatus::Accepted);
        }
        RetireRecord(*attempt_record);
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, LinkTargetRetire>) {
        if (item.actual_target == 0 || item.target_owner == 0) {
            return empty(EventStatus::Refused);
        }
        bool found = false;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            if (!records[i].active) continue;
            const auto* node = std::get_if<NodeBirthData>(&records[i].data);
            if (node == nullptr || node->address != item.actual_target
                || node->owner != item.target_owner) continue;
            const auto* id = std::get_if<NodeBirthId>(&records[i].id);
            if (id != nullptr) {
                RevokeNode(*id);
                found = true;
            }
        }
        SweepClosedLifecycle();
        InvalidateObservations();
        return empty(found ? EventStatus::Revoked : EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, RootLinkAttemptBegin>) {
        auto* submit = Find<OriginalSubmitData>(item.submit);
        auto* child = submit == nullptr ? nullptr : Find<NodeBirthData>(submit->child);
        if (submit == nullptr || child == nullptr || !RootIsOpen(submit->root)
            || !submit->root_kicked || submit->tail_returned
            || submit->attempt != LinkAttemptId{} || item.actual_root_slot != submit->stack_slot
            || item.actual_child != child->address || item.target_owner != child->owner
            || item.mode != LinkMode::AcquiredGate || item.gate_hold
            || item.target_owner > std::numeric_limits<ScalarAddress>::max() - 0x148
            || item.actual_gate != item.target_owner + 0x148) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* attempt_record = Allocate<LinkAttemptId>(
            ledger,
            LinkAttemptData{LinkKind::Root, item.submit, {}, submit->child, {}, {},
                            item.actual_root_slot, item.actual_child, item.actual_child,
                            item.target_owner, item.actual_gate, item.mode, std::nullopt,
                            CompletionOrigin::WorkerReturn, true, false, false, false,
                            false, false, false, {}},
            failure);
        if (attempt_record == nullptr) return empty(failure);
        const auto attempt_id = std::get<LinkAttemptId>(attempt_record->id);
        submit = Find<OriginalSubmitData>(item.submit);
        submit->attempt = attempt_id;
        return one(attempt_record, EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, GateHoldBegin>) {
        if (item.target_owner == 0 || item.target_owner > std::numeric_limits<ScalarAddress>::max() - 0x148
            || item.gate_address != item.target_owner + 0x148) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* record = Allocate<GateHoldId>(
            ledger, GateHoldData{item.target_owner, item.gate_address, 0, {}, false}, failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, CachedSourceReserved>) {
        auto* hold = Find<GateHoldData>(item.gate_hold);
        if (hold == nullptr || hold->cached_reserved || item.actual_cached_node == 0) {
            return empty(EventStatus::Refused);
        }
        PublicationId selected{};
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            const auto* publication = std::get_if<PublicationData>(&records[i].data);
            if (!records[i].active || publication == nullptr
                || publication->target_owner != hold->owner) continue;
            const auto* node = Find<NodeBirthData>(publication->target);
            const auto* id = std::get_if<PublicationId>(&records[i].id);
            if (node == nullptr || node->address != item.actual_cached_node || id == nullptr) continue;
            if (selected != PublicationId{}) return empty(EventStatus::Refused);
            selected = *id;
        }
        if (selected == PublicationId{}) return empty(EventStatus::Refused);
        hold = Find<GateHoldData>(item.gate_hold);
        if (hold == nullptr) return empty(EventStatus::Refused);
        hold->cached_node = item.actual_cached_node;
        hold->cached_publication = selected;
        hold->cached_reserved = true;
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, QueueClaimCaptured>) {
        auto* hold = Find<GateHoldData>(item.gate_hold);
        if (hold == nullptr || item.actual_node == 0 || !item.native_increment_completed) {
            return empty(EventStatus::Refused);
        }
        PublicationId publication_id{};
        if (item.kind == ClaimKind::Cached) {
            if (!hold->cached_reserved || hold->cached_node != item.actual_node) {
                return empty(EventStatus::Refused);
            }
            publication_id = hold->cached_publication;
        } else {
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                const auto* publication = std::get_if<PublicationData>(&records[i].data);
                if (!records[i].active || publication == nullptr
                    || publication->target_owner != hold->owner) continue;
                const auto* node = Find<NodeBirthData>(publication->target);
                const auto* id = std::get_if<PublicationId>(&records[i].id);
                if (node == nullptr || node->address != item.actual_node || id == nullptr) continue;
                if (publication_id != PublicationId{}) return empty(EventStatus::Refused);
                publication_id = *id;
            }
        }
        const auto* publication = Find<PublicationData>(publication_id);
        if (publication == nullptr || publication->target_owner != hold->owner) return empty(EventStatus::Refused);
        const auto node_id = publication->target;
        if (Find<NodeBirthData>(node_id) == nullptr) return empty(EventStatus::Refused);
        EventStatus failure{};
        auto* record = Allocate<ClaimId>(
            ledger,
            ClaimData{publication_id, node_id, item.gate_hold, item.kind,
                      ClaimState::Claimed, {}},
            failure);
        if (record == nullptr) return empty(failure);
        if (item.kind == ClaimKind::Cached) hold->cached_reserved = false;
        return one(record, record == nullptr ? failure : EventStatus::Committed);
    }
    else if constexpr (std::is_same_v<T, QueueClaimTransfer>) {
        auto* claim = Find<ClaimData>(item.claim);
        if (claim == nullptr || claim->node != item.node || claim->state != ClaimState::Claimed
            || claim->early_completion || Find<NodeBirthData>(item.node) == nullptr
            || Find<GateHoldData>(claim->gate_hold) == nullptr) return empty(EventStatus::Refused);
        claim->state = ClaimState::Transferred;
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, CompletionBegin>) {
        auto* claim = Find<ClaimData>(item.claim);
        if (claim == nullptr || claim->node != item.node
            || !((claim->state == ClaimState::Working && claim->gate_released)
                || (claim->kind == ClaimKind::Cached && claim->state == ClaimState::Claimed))
            || Find<NodeBirthData>(item.node) == nullptr) return empty(EventStatus::Refused);
        claim->early_completion = claim->state == ClaimState::Claimed;
        claim->state = ClaimState::Completing;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& pending_record = records[i];
            auto* materialize = std::get_if<MaterializeData>(&pending_record.data);
            if (!pending_record.active || materialize == nullptr) continue;
            const auto* owner = std::get_if<ClaimId>(&materialize->source);
            if (owner != nullptr && *owner == item.claim) RetireRecord(pending_record);
        }
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, WorkCallbackBegin>) {
        auto* claim = Find<ClaimData>(item.claim);
        auto* node = Find<NodeBirthData>(item.node);
        const auto* publication = claim == nullptr ? nullptr : Find<PublicationData>(claim->publication);
        if (claim == nullptr || node == nullptr || publication == nullptr
            || claim->node != item.node || claim->state != ClaimState::Transferred
            || !claim->gate_released
            || node->role != NodeRole::Child || publication->target != item.node
            || publication->child != item.node || item.callback != kWorkloadCallback) {
            return empty(EventStatus::Refused);
        }
        claim->state = ClaimState::Working;
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, DelayedLinkAttemptBegin>) {
        auto* claim = Find<ClaimData>(item.source_claim);
        auto* submit = Find<OriginalSubmitData>(item.submit);
        auto* source = Find<NodeBirthData>(item.source_child);
        auto* target = Find<NodeBirthData>(item.target_parent);
        auto* edge = Find<TargetEdgeData>(item.original_edge);
        auto* hold = item.gate_hold ? Find<GateHoldData>(*item.gate_hold) : nullptr;
        const bool exact_gate_address = item.target_owner != 0
            && item.target_owner <= std::numeric_limits<ScalarAddress>::max() - 0x148
            && item.actual_gate == item.target_owner + 0x148;
        const bool gate_mode_valid =
            (item.mode == LinkMode::CallerHeldGate && hold != nullptr
             && hold->owner == item.target_owner && hold->gate == item.actual_gate)
            || (item.mode == LinkMode::AcquiredGate && !item.gate_hold
                && exact_gate_address);
        if (claim == nullptr || submit == nullptr || source == nullptr || target == nullptr
            || edge == nullptr || !claim->gate_released || !gate_mode_valid
            || claim->state != ClaimState::Completing
            || claim->early_completion || claim->node != item.source_child
            || submit->child != item.source_child || source->submit != item.submit
            || source->role != NodeRole::Child || target->role != NodeRole::Parent
            || edge->child != item.source_child || edge->parent != item.target_parent
            || edge->target != target->address || item.target_address != target->address
            || item.actual_loaded_target != target->address || item.actual_source != source->address
            || item.target_owner != target->owner || !item.actual_last_run
            || (item.origin == CompletionOrigin::CachedReturn && claim->kind != ClaimKind::Cached)
            || (item.origin == CompletionOrigin::WorkerReturn && claim->kind != ClaimKind::Fresh)) {
            return empty(EventStatus::Refused);
        }
        EventStatus failure{};
        auto* record = Allocate<LinkAttemptId>(
            ledger,
            LinkAttemptData{LinkKind::Delayed, item.submit, item.source_claim,
                            item.source_child, item.original_edge, item.target_parent,
                            0, item.actual_source, item.target_address, item.target_owner,
                            item.actual_gate, item.mode, item.gate_hold, item.origin,
                            item.actual_last_run, false, false, false, false, false, false, {}},
            failure);
        if (record == nullptr) return empty(failure);
        const auto attempt_id = std::get<LinkAttemptId>(record->id);
        claim = Find<ClaimData>(item.source_claim);
        claim->delayed_attempt = attempt_id;
        return one(record, EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, DelayedLinkRequestEntered>) {
        auto* attempt = Find<LinkAttemptData>(item.attempt);
        if (attempt == nullptr || attempt->kind != LinkKind::Delayed
            || attempt->request_entered || attempt->source_address != item.actual_source
            || attempt->target_address != item.actual_target) return empty(EventStatus::Refused);
        attempt->request_entered = true;
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, LinkPrevalidated>) {
        auto* attempt = Find<LinkAttemptData>(item.attempt);
        if (attempt == nullptr || attempt->prevalidated
            || attempt->target_address != item.target_address
            || attempt->target_owner != item.target_owner || attempt->gate != item.actual_gate) {
            return empty(EventStatus::Refused);
        }
        if (attempt->kind == LinkKind::Root) {
            const auto* submit = Find<OriginalSubmitData>(attempt->submit);
            if (submit == nullptr || !RootIsOpen(submit->root)) return empty(EventStatus::Refused);
            if (attempt->mode != LinkMode::AcquiredGate || !item.native_gate_acquired
                || attempt->gate_hold) return empty(EventStatus::Refused);
        } else {
            const auto* hold = attempt->gate_hold ? Find<GateHoldData>(*attempt->gate_hold) : nullptr;
            const bool exact_gate_address = attempt->target_owner != 0
                && attempt->target_owner <= std::numeric_limits<ScalarAddress>::max() - 0x148
                && attempt->gate == attempt->target_owner + 0x148
                && item.actual_gate == attempt->gate;
            const bool held_gate_valid = attempt->mode == LinkMode::CallerHeldGate
                && !item.native_gate_acquired && hold != nullptr
                && hold->owner == item.target_owner && hold->gate == item.actual_gate
                && attempt->request_entered;
            const bool acquired_gate_valid = attempt->mode == LinkMode::AcquiredGate
                && !attempt->gate_hold && item.native_gate_acquired && exact_gate_address;
            if (!held_gate_valid && !acquired_gate_valid) {
                return empty(EventStatus::Refused);
            }
        }
        attempt->prevalidated = true;
        attempt->native_gate_acquired = item.native_gate_acquired;
        return empty(EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, LinkCommitted>) {
        auto* attempt_record = FindRecord(item.attempt);
        auto* attempt = Find<LinkAttemptData>(item.attempt);
        if (attempt_record == nullptr || attempt == nullptr || !attempt->prevalidated
            || attempt->publication != PublicationId{}
            || attempt->target_address != item.target_address
            || attempt->target_owner != item.target_owner || attempt->gate != item.actual_gate
            || !item.native_queue_count_increment_completed || !item.native_gate_still_held
            || !item.normal_post) return empty(EventStatus::Refused);
        if (attempt->kind == LinkKind::Root) {
            const auto* submit = Find<OriginalSubmitData>(attempt->submit);
            if (submit == nullptr || !RootIsOpen(submit->root)) return empty(EventStatus::Refused);
        }

        NodeBirthId target_id{};
        NodeBirthId parent_id{};
        NodeBirthId child_id{};
        RootAttributionId root_id{};
        ContextPublicationId context_id{};
        WorldPublicationId world_id{};
        SystemBindingId binding_id{};
        CompileId compile_id{};
        DispatchRecordId dispatch_id{};
        DispatchGroupId group_id{};
        std::uint32_t dispatch_record_count{};
        std::array<DispatchRecordId, kMaxDispatchRecords> dispatch_records{};
        OriginalSubmitId submit_id = attempt->submit;
        BirthStamp<NodeBirthId> parent_stamp{};
        BirthStamp<NodeBirthId> child_stamp{};
        if (attempt->kind == LinkKind::Root) {
            auto* submit = Find<OriginalSubmitData>(attempt->submit);
            auto* child = submit == nullptr ? nullptr : Find<NodeBirthData>(submit->child);
            if (submit == nullptr || child == nullptr || !submit->root_kicked
                || submit->tail_returned || submit->stack_slot != attempt->root_slot
                || child->address != attempt->target_address || child->owner != attempt->target_owner) {
                return empty(EventStatus::Refused);
            }
            target_id = submit->child;
            child_id = submit->child;
            parent_id = child->parent;
            root_id = child->root;
            context_id = child->context;
            world_id = child->world;
            binding_id = child->binding;
            compile_id = child->compile;
            dispatch_id = child->dispatch;
            group_id = child->group;
            dispatch_record_count = child->dispatch_record_count;
            dispatch_records = child->dispatch_records;
            const auto* birth_record = FindRecord(child_id);
            if (birth_record == nullptr) return empty(EventStatus::Refused);
            if (parent_id != NodeBirthId{}) {
                const auto* parent = Find<NodeBirthData>(parent_id);
                if (parent == nullptr || parent->role != NodeRole::Parent
                    || parent->root != root_id || parent->group != child->group) {
                    return empty(EventStatus::Refused);
                }
                parent_stamp = BirthStamp<NodeBirthId>{parent_id, parent->address,
                                                       NativeSite::NodeAllocationReturn,
                                                       parent->birth_frame};
            }
            child_stamp = BirthStamp<NodeBirthId>{child_id, child->address,
                                                  NativeSite::NodeAllocationReturn,
                                                  child->birth_frame};
        } else {
            auto* claim = Find<ClaimData>(attempt->source_claim);
            auto* source_pub = claim == nullptr ? nullptr : Find<PublicationData>(claim->publication);
            auto* source = Find<NodeBirthData>(attempt->source_child);
            auto* parent = Find<NodeBirthData>(attempt->target_parent);
            const auto* parent_record = FindRecord(attempt->target_parent);
            if (claim == nullptr || source_pub == nullptr || source == nullptr || parent == nullptr
                || claim->state != ClaimState::Completing || claim->node != attempt->source_child
                || source->address != attempt->source_address || parent->address != attempt->target_address
                || parent->owner != attempt->target_owner || parent_record == nullptr) {
                return empty(EventStatus::Refused);
            }
            target_id = attempt->target_parent;
            parent_id = attempt->target_parent;
            child_id = attempt->source_child;
            root_id = source_pub->root;
            context_id = source_pub->context;
            world_id = source_pub->world;
            binding_id = source_pub->binding;
            compile_id = source_pub->compile;
            dispatch_id = source_pub->dispatch;
            group_id = source_pub->group;
            dispatch_record_count = source_pub->dispatch_record_count;
            dispatch_records = source_pub->dispatch_records;
            parent_stamp = BirthStamp<NodeBirthId>{parent_id, parent->address,
                                                   NativeSite::NodeAllocationReturn,
                                                   parent->birth_frame};
            child_stamp = source_pub->child_stamp;
        }
        if (Find<ContextPublicationData>(context_id) == nullptr
            || Find<WorldPublicationData>(world_id) == nullptr
            || Find<SystemBindingData>(binding_id) == nullptr
            || Find<CompiledData>(compile_id) == nullptr
            || Find<DispatchRecordData>(dispatch_id) == nullptr
            || Find<DispatchGroupData>(group_id) == nullptr
            || dispatch_record_count == 0
            || dispatch_record_count > kMaxDispatchRecords) return empty(EventStatus::Refused);
        PublicationData publication{target_id, root_id, context_id, world_id, binding_id,
                                    compile_id, dispatch_id, parent_id, child_id,
                                    submit_id, attempt->target_owner, parent_stamp, child_stamp};
        publication.group = group_id;
        publication.dispatch_record_count = dispatch_record_count;
        publication.dispatch_records = dispatch_records;
        EventStatus failure{};
        auto* publication_record = Allocate<PublicationId>(ledger, publication, failure);
        if (publication_record == nullptr) return empty(failure);
        const auto publication_id = std::get<PublicationId>(publication_record->id);
        attempt = Find<LinkAttemptData>(item.attempt);
        if (attempt == nullptr) {
            RetireRecord(*publication_record);
            return empty(EventStatus::Refused);
        }
        attempt->native_queue_increment = true;
        attempt->gate_still_held = true;
        attempt->normal_post = true;
        attempt->publication = publication_id;
        if (auto* submit = Find<OriginalSubmitData>(submit_id)) submit->attempt = item.attempt;
        if (attempt->kind == LinkKind::Delayed) {
            if (auto* claim = Find<ClaimData>(attempt->source_claim)) claim->delayed_attempt = item.attempt;
        }
        return {EventStatus::Committed, publication_id, {}};
    }
    else if constexpr (std::is_same_v<T, CompletionRelease>) {
        auto* record = FindRecord(item.claim);
        auto* claim = Find<ClaimData>(item.claim);
        if (record == nullptr || claim == nullptr || claim->node != item.node
            || claim->state != ClaimState::Completing) return empty(EventStatus::Refused);
        if (claim->delayed_attempt != LinkAttemptId{}) {
            const auto* attempt = Find<LinkAttemptData>(claim->delayed_attempt);
            if (attempt == nullptr || attempt->publication == PublicationId{} || !attempt->normal_post) {
                return empty(EventStatus::Incomplete);
            }
        } else if (!claim->early_completion || claim->kind != ClaimKind::Cached) {
            return empty(EventStatus::Refused);
        }
        RetireRecord(*record);
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, GateHoldRelease>) {
        auto* record = FindRecord(item.hold);
        auto* hold = Find<GateHoldData>(item.hold);
        if (record == nullptr || hold == nullptr || hold->owner != item.actual_owner) {
            return empty(EventStatus::Refused);
        }
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            const auto* attempt = std::get_if<LinkAttemptData>(&records[i].data);
            if (records[i].active && attempt != nullptr && attempt->gate_hold == item.hold
                && attempt->prevalidated && attempt->publication == PublicationId{}) {
                return empty(EventStatus::Incomplete);
            }
        }
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto* claim = std::get_if<ClaimData>(&records[i].data);
            if (records[i].active && claim != nullptr && claim->gate_hold == item.hold
                && claim->state == ClaimState::Transferred) {
                claim->gate_released = true;
            }
        }
        RetireRecord(*record);
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, CachedClaimAbandon>) {
        if (!item.claim) return empty(EventStatus::Accepted);
        auto* claim_record = FindRecord(*item.claim);
        auto* claim = Find<ClaimData>(*item.claim);
        if (claim_record == nullptr || claim == nullptr) return empty(EventStatus::Accepted);
        if (claim->kind != ClaimKind::Cached || claim->state == ClaimState::Completing
            || claim->delayed_attempt != LinkAttemptId{}) return empty(EventStatus::Refused);
        RetireRecord(*claim_record);
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, NodeRetire>) {
        if (item.address == 0) return empty(EventStatus::Refused);
        if (item.node) {
            const auto* node = Find<NodeBirthData>(*item.node);
            if (node == nullptr || node->address != item.address) return empty(EventStatus::Refused);
            RevokeNode(*item.node);
        } else {
            RevokeAddress(item.address);
        }
        SweepClosedLifecycle();
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, RejectedNodeFree>) {
        if (item.address == 0) return empty(EventStatus::Refused);
        RevokeAddress(item.address);
        SweepClosedLifecycle();
        return empty(EventStatus::Revoked);
    }
    else if constexpr (std::is_same_v<T, ManagerRetire>) {
        bool found = false;
        if (item.kind == ManagerRetireKind::DispatchOwner) {
            for (std::uint32_t i = 0; i < limits.max_records; ++i) {
                if (!records[i].active) continue;
                if (const auto* world = std::get_if<WorldPublicationData>(&records[i].data);
                    world != nullptr && world->manager == item.manager) {
                    if (const auto* id = std::get_if<WorldPublicationId>(&records[i].id)) {
                        RevokeWorld(*id);
                        found = true;
                    }
                    continue;
                }
                if (const auto* birth = std::get_if<WorldBirthData>(&records[i].data);
                    birth != nullptr && birth->manager == item.manager) {
                    const auto birth_id = std::get<WorldBirthId>(records[i].id);
                    for (std::uint32_t j = 0; j < limits.max_records; ++j) {
                        auto& pending_record = records[j];
                        const auto* pending = std::get_if<WorldReservationData>(&pending_record.data);
                        if (pending_record.active && pending != nullptr && pending->birth == birth_id) {
                            RetireRecord(pending_record);
                        }
                    }
                    RetireRecord(records[i]);
                    found = true;
                    continue;
                }
                const auto* dispatch = std::get_if<DispatchRecordData>(&records[i].data);
                if (!records[i].active || dispatch == nullptr || dispatch->manager != item.manager) continue;
                RevokeRoot(dispatch->root);
                found = true;
            }
        } else {
            RevokeAddress(item.manager);
            found = true;
        }
        InvalidateObservations();
        return empty(found ? EventStatus::Revoked : EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, RRelocationBegin>) {
        if (item.container == 0) return empty(EventStatus::Refused);
        constexpr ScalarAddress kRelocationOffset = 0x4A4A8;
        bool found = false;
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            if (!records[i].active) continue;
            const auto* dispatch = std::get_if<DispatchRecordData>(&records[i].data);
            if (dispatch == nullptr
                || dispatch->manager > std::numeric_limits<ScalarAddress>::max() - kRelocationOffset
                || dispatch->manager + kRelocationOffset != item.container) continue;
            const auto root_id = dispatch->root;
            RevokeRoot(root_id);
            found = true;
        }
        // Closed-root sweeping may already have reclaimed the dispatch records
        // for a copied direct observation. The still-live World publication
        // retains the original manager scalar, which is sufficient to apply
        // this negative fence without keeping root/R tombstones alive.
        for (std::uint32_t i = 0; i < limits.max_observations; ++i) {
            auto& observation = observations[i];
            if (!observation.active) continue;
            const auto* world = Find<WorldPublicationData>(observation.observation.world);
            if (world == nullptr || world->manager == 0
                || world->manager > std::numeric_limits<ScalarAddress>::max() - kRelocationOffset
                || world->manager + kRelocationOffset != item.container) continue;
            observation.active = false;
            found = true;
        }
        InvalidateObservations();
        return empty(found ? EventStatus::Revoked : EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, MaterializeBegin>) {
        MaterializeData data{item.source, item.publication, item.compile, item.binding,
                             item.dispatch, item.parent, item.child, 0,
                             receipt.ThreadToken(), receipt.FrameToken()};
        if (const auto* direct_root = std::get_if<RootAttributionId>(&item.source);
            direct_root != nullptr && !RootIsOpen(*direct_root)) {
            return empty(EventStatus::Refused);
        }
        RootAttributionId root_id{};
        ContextPublicationId context_id{};
        WorldPublicationId world_id{};
        const PublicationData* publication = nullptr;
        if (!MaterializeOwnerLive(data, root_id, context_id, world_id, publication)) {
            return empty(EventStatus::Refused);
        }

        // A same-frame re-entry cancels its older pending operation before this
        // operation is made visible to the adapter.
        for (std::uint32_t i = 0; i < limits.max_records; ++i) {
            auto& pending_record = records[i];
            auto* pending = std::get_if<MaterializeData>(&pending_record.data);
            if (!pending_record.active || pending == nullptr
                || pending->thread != receipt.ThreadToken()
                || pending->frame != receipt.FrameToken()) continue;
            bool same_source = false;
            if (const auto* new_root = std::get_if<RootAttributionId>(&item.source)) {
                const auto* old_root = std::get_if<RootAttributionId>(&pending->source);
                same_source = old_root != nullptr && *old_root == *new_root;
            } else if (const auto* new_claim = std::get_if<ClaimId>(&item.source)) {
                const auto* old_claim = std::get_if<ClaimId>(&pending->source);
                same_source = old_claim != nullptr && *old_claim == *new_claim;
            }
            if (same_source) RetireRecord(pending_record);
        }
        EventStatus failure{};
        auto* record = Allocate<MaterializeOpId>(ledger, data, failure);
        return one(record, record == nullptr ? failure : EventStatus::Pending);
    }
    else if constexpr (std::is_same_v<T, MaterializeEntryCaptured>) {
        auto* data = Find<MaterializeData>(item.operation);
        if (data == nullptr) return empty(EventStatus::Refused);
        if (data->thread != receipt.ThreadToken() || data->frame != receipt.FrameToken()) {
            if (auto* record = FindRecord(item.operation)) RetireRecord(*record);
            return empty(EventStatus::Refused);
        }
        if (data->entry_count >= kMaxMaterializeEntries
            || item.source_entry == 0 || item.first_ordinal == 0 || item.final_ordinal == 0
            || item.native_output_record_address == 0
            || (item.emitted_component_pointer != 0 && item.component_stride == 0)) {
            return empty(EventStatus::Refused);
        }
        RootAttributionId root_id{};
        ContextPublicationId context_id{};
        WorldPublicationId world_id{};
        const PublicationData* publication = nullptr;
        if (!MaterializeOwnerLive(*data, root_id, context_id, world_id, publication)) {
            RetireRecord(*FindRecord(item.operation));
            return empty(EventStatus::Refused);
        }
        EvidenceEntry query{};
        if (!CompileHasQuery(data->compile, item.source_entry, item.first_ordinal,
                             item.group, item.direction, query)
            || query.ordinal != item.first_ordinal
            || (item.emitted_component_pointer == 0 && !query.optional)) {
            return empty(EventStatus::Refused);
        }
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            const auto& existing = evidence[i];
            if (existing.active && existing.kind == EvidenceKind::MaterializeEntry
                && SameHandle(existing.owner, item.operation)
                && existing.source_entry == item.source_entry
                && existing.group == item.group && existing.direction == item.direction) {
                return empty(EventStatus::Refused);
            }
        }
        auto* entry = AllocateEvidence(item.operation, EvidenceKind::MaterializeEntry);
        if (entry == nullptr) return empty(EventStatus::CapacityExhausted);
        entry->address = item.source_entry;
        entry->source_entry = item.source_entry;
        entry->hash = query.hash;
        entry->key = query.key;
        entry->ordinal = query.ordinal;
        entry->first_ordinal = item.first_ordinal;
        entry->final_ordinal = item.final_ordinal;
        entry->original_row_index = item.original_row_index;
        entry->group = query.group;
        entry->flags = query.flags;
        entry->direction = query.direction;
        entry->optional = query.optional;
        entry->emitted_component_pointer = item.emitted_component_pointer;
        entry->component_stride = item.component_stride;
        entry->native_output_record_address = item.native_output_record_address;
        ++data->entry_count;
        return empty(EventStatus::Accepted);
    }
    else if constexpr (std::is_same_v<T, MaterializeComplete>) {
        auto* record = FindRecord(item.operation);
        auto* data = Find<MaterializeData>(item.operation);
        if (record == nullptr || data == nullptr) return empty(EventStatus::Incomplete);
        if (data->thread != receipt.ThreadToken() || data->frame != receipt.FrameToken()) {
            RetireRecord(*record);
            return empty(EventStatus::Refused);
        }
        const auto captured_count = EvidenceCount(item.operation, EvidenceKind::MaterializeEntry);
        if (item.query_count == 0 && item.completed_count == 0 && captured_count == 0
            && data->entry_count == 0) {
            RetireRecord(*record);
            return empty(EventStatus::Accepted);
        }
        if (item.query_count == 0 || item.completed_count == 0
            || item.query_count > kMaxMaterializeEntries
            || item.query_count != item.completed_count
            || item.query_count != data->entry_count
            || captured_count != data->entry_count) {
            RetireRecord(*record);
            return empty(EventStatus::Incomplete);
        }

        RootAttributionId root_id{};
        ContextPublicationId context_id{};
        WorldPublicationId world_id{};
        const PublicationData* publication = nullptr;
        if (!MaterializeOwnerLive(*data, root_id, context_id, world_id, publication)) {
            RetireRecord(*record);
            return empty(EventStatus::Refused);
        }

        OwnedObservation observation{};
        observation.context = context_id;
        observation.world = world_id;
        observation.binding = data->binding;
        observation.compile = data->compile;
        observation.dispatch = data->dispatch;
        observation.root = root_id;
        observation.publication = data->publication.value_or(PublicationId{});
        if (publication != nullptr) {
            observation.parent = *data->parent;
            observation.child = *data->child;
            observation.parent_birth = publication->parent_stamp;
            observation.child_birth = publication->child_stamp;
        }

        std::uint32_t output_index = 0;
        for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
            const auto& captured = evidence[i];
            if (!captured.active || captured.kind != EvidenceKind::MaterializeEntry
                || !SameHandle(captured.owner, item.operation)) continue;
            EvidenceEntry query{};
            if (output_index >= item.query_count
                || !CompileHasQuery(data->compile, captured.source_entry,
                                    captured.first_ordinal, captured.group,
                                    captured.direction, query)
                || query.ordinal != captured.ordinal || query.hash != captured.hash
                || query.key != captured.key || query.flags != captured.flags
                || query.optional != captured.optional || query.direction != captured.direction) {
                RetireRecord(*record);
                return empty(EventStatus::Refused);
            }
            observation.entries[output_index++] = OwnedSemanticEntry{
                captured.hash,
                captured.key,
                captured.first_ordinal,
                captured.final_ordinal,
                captured.original_row_index,
                captured.group,
                captured.flags,
                captured.optional,
                captured.direction,
                captured.source_entry,
                captured.emitted_component_pointer,
                captured.component_stride,
                captured.native_output_record_address};
        }
        if (output_index != item.query_count) {
            RetireRecord(*record);
            return empty(EventStatus::Incomplete);
        }
        observation.entry_count = output_index;

        ObservationSlot* selected = nullptr;
        std::uint32_t observation_index = 0;
        bool exhausted = false;
        for (std::uint32_t i = 0; i < limits.max_observations; ++i) {
            auto& slot = observations[i];
            if (slot.active) continue;
            if (slot.generation == std::numeric_limits<std::uint64_t>::max()) {
                exhausted = true;
                continue;
            }
            selected = &slot;
            observation_index = i;
            break;
        }
        if (selected == nullptr) {
            RetireRecord(*record);
            if (exhausted) {
                generation_exhausted = true;
                return empty(EventStatus::GenerationExhausted);
            }
            return empty(EventStatus::CapacityExhausted);
        }
        ++selected->generation;
        selected->active = true;
        selected->id = ledger.MintId<ObservationId>(
            incarnation, limits.max_records + observation_index + 1, selected->generation);
        selected->observation = observation;
        const auto observation_id = selected->id;
        RetireRecord(*record);
        return {EventStatus::Committed, observation_id, {}};
    }

    return record_failure(EventStatus::Refused);
}

EventReply Ledger::Impl::Observe(Ledger& ledger,
                                 const NativeReceipt& receipt,
                                 const Event& event) noexcept {
    if (receipt.Site() != SiteFor(event)
        || receipt.AdapterIncarnation() != limits.expected_adapter_incarnation
        || receipt.ProfileIdentity() != limits.expected_profile_identity
        || receipt.ThreadToken() == 0 || receipt.FrameToken() == 0) {
        return {EventStatus::InvalidReceipt, {}, {}};
    }
    auto reply = std::visit([&](const auto& item) noexcept {
        return Apply(ledger, receipt, item);
    }, event);
    if (receipt.Site() == NativeSite::MaterializeOutput
        || receipt.Site() == NativeSite::MaterializeComplete) {
        SweepClosedLifecycle();
    }
    return reply;
}

std::size_t Ledger::RequiredStorage(Limits limits) noexcept {
    if (limits.max_records == 0 || limits.max_evidence_entries == 0
        || limits.max_observations == 0 || limits.max_registration_entries == 0
        || limits.max_compile_entries == 0 || limits.expected_adapter_incarnation == 0
        || limits.expected_profile_identity == 0
        || limits.max_records > std::numeric_limits<std::uint32_t>::max()
            - limits.max_observations - 1U) {
        return 0;
    }
    std::size_t total = 0;
    if (!AddSize<Ledger>(total)
        || !AddSize<Impl>(total)
        || !AddSize<Record>(total, limits.max_records)
        || !AddSize<EvidenceEntry>(total, limits.max_evidence_entries)
        || !AddSize<ObservationSlot>(total, limits.max_observations)) {
        return 0;
    }
    return total;
}

ConstructResult Ledger::Construct(std::span<std::byte> storage,
                                  Limits limits,
                                  DescriptorCatalog catalog) noexcept {
    const auto required = RequiredStorage(limits);
    if (required == 0) return {nullptr, ConstructStatus::InvalidLimits};
    if (catalog.entries.size() != 0 && catalog.entries.data() == nullptr) {
        return {nullptr, ConstructStatus::InvalidCatalog};
    }
    for (std::size_t i = 0; i < catalog.entries.size(); ++i) {
        for (std::size_t j = i + 1; j < catalog.entries.size(); ++j) {
            if (catalog.entries[i].semantic_hash == catalog.entries[j].semantic_hash) {
                return {nullptr, ConstructStatus::InvalidCatalog};
            }
        }
    }
    if (storage.data() == nullptr || storage.size() < required) {
        return {nullptr, ConstructStatus::InsufficientStorage};
    }

    std::size_t offset = 0;
    auto* ledger_memory = Carve<Ledger>(storage, offset);
    auto* impl_memory = Carve<Impl>(storage, offset);
    auto* record_memory = Carve<Record>(storage, offset, limits.max_records);
    auto* evidence_memory = Carve<EvidenceEntry>(storage, offset, limits.max_evidence_entries);
    auto* observation_memory = Carve<ObservationSlot>(storage, offset, limits.max_observations);
    if (ledger_memory == nullptr || impl_memory == nullptr || record_memory == nullptr
        || evidence_memory == nullptr || observation_memory == nullptr) {
        return {nullptr, ConstructStatus::InsufficientStorage};
    }

    std::uint64_t incarnation{};
    if (!NextIncarnation(incarnation)) return {nullptr, ConstructStatus::IncarnationExhausted};
    for (std::uint32_t i = 0; i < limits.max_records; ++i) ::new (&record_memory[i]) Record{};
    for (std::uint32_t i = 0; i < limits.max_evidence_entries; ++i) {
        ::new (&evidence_memory[i]) EvidenceEntry{};
    }
    for (std::uint32_t i = 0; i < limits.max_observations; ++i) {
        ::new (&observation_memory[i]) ObservationSlot{};
    }
    auto* impl = ::new (impl_memory) Impl(
        limits, catalog, incarnation, record_memory, evidence_memory, observation_memory);
    auto* ledger = ::new (ledger_memory) Ledger(impl);
    return {ledger, ConstructStatus::Ready};
}

EventReply Ledger::Observe(const NativeReceipt& receipt, const Event& event) noexcept {
    if (impl_ == nullptr) return {EventStatus::Refused, {}, {}};
    std::lock_guard lock(impl_->gate);
    return impl_->Observe(*this, receipt, event);
}

bool Ledger::TryCopy(ObservationId id, OwnedObservation& output) const noexcept {
    output = OwnedObservation{};
    if (impl_ == nullptr || id == ObservationId{}) return false;
    std::lock_guard lock(impl_->gate);
    if (impl_->closed) return false;
    impl_->InvalidateObservations();
    for (std::uint32_t i = 0; i < impl_->limits.max_observations; ++i) {
        const auto& slot = impl_->observations[i];
        if (!slot.active || slot.id != id) continue;
        output = slot.observation;
        return true;
    }
    return false;
}

void Ledger::CloseAll(CloseReason) noexcept {
    if (impl_ == nullptr) return;
    std::lock_guard lock(impl_->gate);
    impl_->closed = true;
    for (std::uint32_t i = 0; i < impl_->limits.max_records; ++i) {
        impl_->records[i].active = false;
        impl_->records[i].data = std::monostate{};
        impl_->records[i].related = {};
    }
    for (std::uint32_t i = 0; i < impl_->limits.max_evidence_entries; ++i) {
        impl_->evidence[i].active = false;
    }
    for (std::uint32_t i = 0; i < impl_->limits.max_observations; ++i) {
        impl_->observations[i].active = false;
        impl_->observations[i].observation = OwnedObservation{};
    }
}

Ledger::~Ledger() {
    if (impl_ == nullptr) return;
    CloseAll(CloseReason::Shutdown);
    for (std::uint32_t i = 0; i < impl_->limits.max_records; ++i) {
        std::destroy_at(&impl_->records[i]);
    }
    for (std::uint32_t i = 0; i < impl_->limits.max_evidence_entries; ++i) {
        std::destroy_at(&impl_->evidence[i]);
    }
    for (std::uint32_t i = 0; i < impl_->limits.max_observations; ++i) {
        std::destroy_at(&impl_->observations[i]);
    }
    impl_->~Impl();
    impl_ = nullptr;
}

} // namespace SemanticAttribution
