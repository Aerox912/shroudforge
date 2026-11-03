#pragma once

#include "owned_event_types.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <span>

namespace SemanticAttribution {

struct Limits final {
    std::uint32_t max_records{};
    std::uint32_t max_evidence_entries{};
    std::uint32_t max_observations{};
    std::uint32_t max_registration_entries{};
    std::uint32_t max_compile_entries{};
    std::uint64_t expected_adapter_incarnation{};
    std::uint64_t expected_profile_identity{};
};

struct DescriptorDefinition final {
    SemanticHash semantic_hash{};
    SemanticKey semantic_key{};
};

// The caller-owned catalog is immutable and must outlive the constructed ledger.
struct DescriptorCatalog final {
    std::span<const DescriptorDefinition> entries;
};

struct OwnedSemanticEntry final {
    SemanticHash semantic_hash{};
    SemanticKey semantic_key{};
    // Pairing identity is the compiled query entry occurrence, not ordinal alone.
    std::uint32_t first_ordinal{};
    std::uint32_t final_ordinal{};
    std::uint32_t original_row_index{};
    // Native branch category only: 0 for READ and 1 for WRITE.
    std::uint32_t group{};
    // Native flag0 in bits 0-7 and flag1 in bits 8-15; upper 16 bits are zero.
    std::uint32_t flags{};
    bool optional{};
    HashDirection direction{};
    ScalarAddress source_entry{};
    // This scalar is already row-indexed by the native query.
    ScalarAddress emitted_component_pointer{};
    std::uint32_t component_stride{};
    // Kept distinct from the component pointer; this is the native stack record.
    ScalarAddress native_output_record_address{};
};

struct OwnedObservation final {
    ContextPublicationId context;
    WorldPublicationId world;
    SystemBindingId binding;
    CompileId compile;
    DispatchRecordId dispatch;
    RootAttributionId root;
    PublicationId publication;
    NodeBirthId parent;
    NodeBirthId child;
    BirthStamp<NodeBirthId> parent_birth;
    BirthStamp<NodeBirthId> child_birth;
    std::uint32_t entry_count{};
    std::array<OwnedSemanticEntry, kMaxMaterializeEntries> entries{};
};

enum class CloseReason : std::uint8_t {
    Explicit,
    ContextRetired,
    WorldRetired,
    SchemaChanged,
    RegistrationReplaced,
    CompileReplaced,
    ParentRetired,
    ChildRetired,
    CapacityEviction,
    Shutdown,
};

enum class ConstructStatus : std::uint8_t {
    Ready,
    InvalidLimits,
    InvalidCatalog,
    InsufficientStorage,
    IncarnationExhausted,
};

class Ledger;

struct ConstructResult final {
    Ledger* ledger{};
    ConstructStatus status{ConstructStatus::InvalidLimits};
    explicit operator bool() const noexcept { return ledger != nullptr; }
};

class Ledger final {
public:
    Ledger(const Ledger&) = delete;
    Ledger& operator=(const Ledger&) = delete;

    static std::size_t RequiredStorage(Limits limits) noexcept;
    static ConstructResult Construct(std::span<std::byte> storage,
                                     Limits limits,
                                     DescriptorCatalog catalog) noexcept;

    EventReply Observe(const NativeReceipt& receipt, const Event& event) noexcept;
    bool TryCopy(ObservationId id, OwnedObservation& output) const noexcept;
    void CloseAll(CloseReason reason) noexcept;

    ~Ledger();

private:
    struct Impl;
    explicit Ledger(Impl* impl) noexcept : impl_(impl) {}

    template <class Id>
    static Id MintId(std::uint64_t incarnation,
                     std::uint32_t slot,
                     std::uint64_t generation) noexcept;

    Impl* impl_{};
    friend class NativeEventAdapter;
};

} // namespace SemanticAttribution
