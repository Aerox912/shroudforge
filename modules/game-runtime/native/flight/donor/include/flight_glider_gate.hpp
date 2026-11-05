#pragma once

#include "flight_client_frame.hpp"

#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
#include "local_subject_context.hpp"
#endif

#include <atomic>
#include <cstddef>
#include <cstdint>

namespace shroudweave::flight {

inline constexpr std::uint64_t kClientFlightProfileId = 0x5D9EF2F140B8ED49ull;
inline constexpr std::uint32_t kHangGlidingState = 4;
inline constexpr std::uint32_t kEnabledPitchScalarBits = 0xBFC8F5C3u;
inline constexpr std::uint32_t kStockPitchScalarBits = 0x3DB2B8C2u;

struct NativeProfileAdmission final {
    std::uint64_t imageBase{};
    std::uint64_t imageSize{};
    std::uint64_t generation{};
    bool admittedBeforePatch{};
};

struct TypedGliderBinding final {
    std::uintptr_t query{};
    std::uintptr_t queryContext{};
    std::uintptr_t component{};
    std::uintptr_t componentChunk{};
    std::uintptr_t currentRow{};
    std::uintptr_t entityLocation{};
    std::uintptr_t entityPresenceBitmap{};
    std::uintptr_t entityChunk{};
    std::uint32_t queryIndex{};
    std::uint32_t entityId{};
    std::uint32_t entityRow{};
    std::uint16_t nativeIndex{};
    std::uint16_t stride{};
    std::uint16_t entityRowStride{};
    std::uint16_t componentOffset{};
    std::uint8_t state{};
};

enum class PitchDecisionPath : std::uint8_t {
    disabled,
    invalid_profile,
    invalid_frame,
    non_gliding,
    typed_binding_failed,
    local_subject_failed,
    typed_world_failed,
    typed_index_failed,
    typed_entity_failed,
    typed_component_failed,
    typed_world_query_read_failed,
    typed_world_pointer_invalid,
    typed_world_registry_read_failed,
    typed_world_anchor_read_failed,
    typed_world_root_underflow,
    typed_world_root_overflow,
    typed_world_identity_mismatch,
    typed_world_registry_mismatch,
    enabled_local_glider,
};

struct PitchDecision final {
    std::uint32_t scalarBits{kStockPitchScalarBits};
    std::uint32_t requestedPitchBits{};
    bool overrideRequestedPitch{};
    std::size_t readCalls{};
    std::size_t bytesRead{};
    PitchDecisionPath path{PitchDecisionPath::disabled};
#if defined(SHROUDWEAVE_FLIGHT_DIAGNOSTICS) && SHROUDWEAVE_FLIGHT_DIAGNOSTICS
    shroudweave::client_context::Reason localSubjectReason{
        shroudweave::client_context::Reason::none};
    std::uint8_t canonicalIndexMismatchMask{};
    shroudweave::client_context::Reason diagnosticReadError{
        shroudweave::client_context::Reason::none};
    std::uint8_t locomotionState{};
    bool hasLocomotionState{};
    bool frameRspRelationKnown{};
    bool frameRspRelationMatches{};
#endif
};

// Runs the exact selector used by the SDK callback with injected bounded
// reads and stack bounds. The test seam never executes native game code.
PitchDecision EvaluateCurrentPitch(
    const NativeScalarFrame& frame,
    const NativeProfileAdmission& profile,
    const std::atomic_bool& enabled,
    const std::atomic<std::uint64_t>& enableEpoch,
    BoundedNativeRead read,
    void* readContext,
    std::uintptr_t stackLow,
    std::uintptr_t stackHigh) noexcept;

bool AdmitCurrentClientProfile(
    std::uintptr_t imageBase,
    std::size_t imageSize,
    std::uint64_t generation) noexcept;
void DisableCurrentPitchSelection() noexcept;
void EnableCurrentPitchSelection() noexcept;
bool CurrentPitchSelectionEnabled() noexcept;
const std::atomic_bool& CurrentPitchSelectionEnabledAtomic() noexcept;
const std::atomic<std::uint64_t>& CurrentPitchSelectionEpochAtomic() noexcept;
const NativeProfileAdmission& CurrentNativeProfile() noexcept;

// Resolves the reflected DynamicLocomotion index through the current query's
// world registry, then uses the native entity map's type offset and stride to
// join the exact current entity row to the callback's B0 component pointer.
// State value 4 is only returned after that join; no declaration-order column
// mapping or cached actor is used.
bool ResolveCurrentTypedGlider(
    const NativeQueryFrame& frame,
    BoundedNativeRead read,
    void* readContext,
    TypedGliderBinding& binding,
    PitchDecisionPath* rejectedPath = nullptr) noexcept;

bool SameTypedGliderBinding(
    const TypedGliderBinding& left,
    const TypedGliderBinding& right) noexcept;

} // namespace shroudweave::flight
