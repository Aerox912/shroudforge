#pragma once

#include <cstddef>
#include <cstdint>

namespace shroudweave::flight {

struct NativeScalarFrame final {
    std::uint64_t site{};
    std::uint64_t nativeRbp{};
    std::uint64_t nativeRsp{};
    std::uint32_t originalScalarBits{};
    std::uint32_t requestedPitchBits{};
    std::uint32_t facingYBits{};
    bool hasPitchOperands{};
};

struct NativeQueryFrame final {
    std::uintptr_t query{};
    std::uintptr_t output{};
    std::uintptr_t componentPointerSlot{};
};

using BoundedNativeRead = bool (*)(
    std::uintptr_t address, void* destination, std::size_t size, void* context) noexcept;

// Establishes only the current callback frame and Q/O identity. It does not
// admit component type, row identity, or local ownership.
bool ValidateNativeQueryFrame(
    const NativeScalarFrame& frame,
    std::uintptr_t expectedSite,
    std::uintptr_t currentStackLow,
    std::uintptr_t currentStackHigh,
    BoundedNativeRead read,
    void* readContext,
    NativeQueryFrame& frameOut) noexcept;

// Reads one candidate state byte after frame validation. This is a cheap
// negative filter only; callers must perform typed and local joins before use.
bool ReadCheapLocomotionState(
    const NativeQueryFrame& frame,
    BoundedNativeRead read,
    void* readContext,
    std::uint8_t& stateOut) noexcept;

// Production entry point: derives bounds from the actual current thread and
// uses a bounded SEH-guarded memcpy for the two native-frame identity reads.
bool CaptureCurrentNativeQueryFrame(
    const NativeScalarFrame& frame,
    std::uintptr_t expectedSite,
    NativeQueryFrame& frameOut) noexcept;

// Production bounded read for the remaining query/type/local resolver joins.
bool ReadCurrentNativeMemory(
    std::uintptr_t address,
    void* destination,
    std::size_t size,
    void* unusedContext) noexcept;

} // namespace shroudweave::flight
