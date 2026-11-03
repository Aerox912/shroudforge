#pragma once

#include <cstddef>
#include <cstdint>

namespace shroudweave::client_context {

using Address = std::uintptr_t;

inline constexpr std::size_t kMaxReadCallbacks = 64;
inline constexpr std::size_t kMaxObservedRanges = 32;
inline constexpr std::size_t kMaxObservedBytes = 512;
inline constexpr std::size_t kMaxTotalReadBytes = 1024;
inline constexpr std::size_t kMaxRangeBytes = 64;
inline constexpr std::uint32_t kMaxHashProbes = 8;

enum class Predicate : std::uint8_t {
    unknown,
    remote,
    local,
};

enum class Reason : std::uint8_t {
    none,
    invalid_borrow,
    snapshot_unavailable,
    profile_not_admitted,
    profile_or_lifetime_changed,
    guarded_read_failed,
    read_budget_exceeded,
    observation_budget_exceeded,
    malformed_query,
    query_index_out_of_range,
    invalid_world_root,
    non_main_world,
    invalid_entity_map,
    entity_probe_exhausted,
    entity_not_indexed,
    invalid_location,
    current_row_mismatch,
    invalid_local_map,
    local_probe_exhausted,
    canonical_resource_missing,
    invalid_canonical_resource,
    invalid_selected_value,
    memory_changed,
    local_value_matches,
    local_value_differs,
    world_registry_mismatch,
    canonical_index_witness_failed,
    canonical_index_mismatch,
};

struct NativeProfileStamp {
    std::uint64_t profileId{};
    std::uint64_t generation{};
    Address imageBase{};
    std::uint64_t imageSize{};
    bool admittedBeforePatch{};
};

struct CurrentEngineStamp {
    Address query{};
    std::uint64_t lifetimeGeneration{};
    NativeProfileStamp profile{};
};

using GuardedRead = bool (*)(void* context, Address address, void* output,
                             std::size_t bytes) noexcept;
using SnapshotCurrent = bool (*)(void* context, CurrentEngineStamp& output) noexcept;

// The caller supplies the actual current query Q from its admitted callback.
// Integration must establish the initializer's O0 query and saved entry query,
// including read(RBP-0x40) == read(RBP+0x1670), before constructing this borrow.
// The admitted actor-dispatch context is C=read(Q), with C[0]=owning world,
// rows at C+0x830 and count at C+0x838. The caller must pin that native layout;
// this contract does not accept arbitrary query-context object layouts.
struct CurrentQueryBorrow {
    Address query{};
    NativeProfileStamp admittedProfile{};
    void* context{};
    GuardedRead guardedRead{};
    SnapshotCurrent snapshotCurrent{};
};

// Owned values only. No engine address, actor pointer, or later-dereference token
// is returned by this conditional predicate component.
struct Result {
    Predicate predicate{Predicate::unknown};
    Reason reason{Reason::none};
    std::uint32_t entity{};
    std::uint32_t selectedEntity{};
    std::uint32_t readCalls{};
    std::uint32_t bytesRead{};
    std::uint16_t observations{};
    std::uint8_t entityProbes{};
    std::uint8_t localProbes{};
    std::uint8_t snapshotCalls{};
    // Observational refusal bits only: bitmap/capacity/keys/locations/livecount.
    std::uint8_t canonicalIndexMismatchMask{};
    Reason diagnosticReadError{Reason::none};
};

// Conditional canonical LocalPlayerData current-query predicate. This is not
// an SDK GetLocalActor equivalent or an accepted NativeLocalActor service.
Result evaluate_current_query(const CurrentQueryBorrow& borrow) noexcept;

} // namespace shroudweave::client_context
