#include "flight_client_frame.hpp"

#define WIN32_LEAN_AND_MEAN
#include <windows.h>

#include <cstring>
#include <limits>

namespace shroudweave::flight {
namespace {

constexpr std::uintptr_t kRspFromRbp = 0x100;
constexpr std::uintptr_t kOutputFromRbp = 0x40;
constexpr std::uintptr_t kOutputSize = 0x140;
constexpr std::uintptr_t kSavedQueryFromRbp = 0x1670;
constexpr std::uintptr_t kComponentSlotFromRbp = 0xB0;
constexpr std::uintptr_t kStateFromComponent = 0x59;
constexpr std::size_t kMaximumNativeRead = 0x1000;

bool AddFits(std::uintptr_t base, std::uintptr_t amount) noexcept {
    return base <= (std::numeric_limits<std::uintptr_t>::max)() - amount;
}

bool ReadCurrentNativeMemorySeh(
    std::uintptr_t address,
    void* destination,
    std::size_t size,
    void*) noexcept {
    if (address == 0 || destination == nullptr || size == 0 || size > kMaximumNativeRead ||
        address > (std::numeric_limits<std::uintptr_t>::max)() - size) {
        return false;
    }
    __try {
        std::memcpy(destination, reinterpret_cast<const void*>(address), size);
        return true;
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        return false;
    }
}

} // namespace

bool ValidateNativeQueryFrame(
    const NativeScalarFrame& frame,
    std::uintptr_t expectedSite,
    std::uintptr_t currentStackLow,
    std::uintptr_t currentStackHigh,
    BoundedNativeRead read,
    void* readContext,
    NativeQueryFrame& frameOut) noexcept {
    frameOut = {};
    if (read == nullptr || frame.site != expectedSite || expectedSite == 0 ||
        frame.nativeRbp == 0 || frame.nativeRsp == 0 ||
        frame.nativeRbp < kRspFromRbp || currentStackLow >= currentStackHigh ||
        frame.nativeRsp != frame.nativeRbp - kRspFromRbp ||
        (frame.nativeRsp & 0xF) != 0 ||
        !AddFits(frame.nativeRbp, kSavedQueryFromRbp + sizeof(std::uintptr_t)) ||
        frame.nativeRsp < currentStackLow ||
        frame.nativeRbp > currentStackHigh) {
        return false;
    }

    const auto output = frame.nativeRbp - kOutputFromRbp;
    const auto outputEnd = output + kOutputSize;
    const auto savedQueryAddress = frame.nativeRbp + kSavedQueryFromRbp;
    const auto savedQueryEnd = savedQueryAddress + sizeof(std::uintptr_t);
    const auto componentSlot = frame.nativeRbp + kComponentSlotFromRbp;
    if ((output & 0xF) != 0 || output < currentStackLow || outputEnd > currentStackHigh ||
        savedQueryAddress < currentStackLow || savedQueryEnd > currentStackHigh ||
        componentSlot < output || componentSlot > outputEnd - sizeof(std::uintptr_t)) {
        return false;
    }

    std::uintptr_t outputQuery{};
    std::uintptr_t savedQuery{};
    if (!read(output, &outputQuery, sizeof(outputQuery), readContext) ||
        !read(savedQueryAddress, &savedQuery, sizeof(savedQuery), readContext) ||
        outputQuery == 0 || savedQuery == 0 || outputQuery != savedQuery) {
        return false;
    }

    frameOut.query = savedQuery;
    frameOut.output = output;
    frameOut.componentPointerSlot = componentSlot;
    return true;
}

bool ReadCheapLocomotionState(
    const NativeQueryFrame& frame,
    BoundedNativeRead read,
    void* readContext,
    std::uint8_t& stateOut) noexcept {
    stateOut = 0;
    if (read == nullptr || frame.query == 0 || frame.output == 0 ||
        frame.output > (std::numeric_limits<std::uintptr_t>::max)() - 0xF0 ||
        frame.componentPointerSlot != frame.output + 0xF0) {
        return false;
    }
    std::uintptr_t component{};
    if (!read(frame.componentPointerSlot, &component, sizeof(component), readContext) ||
        component == 0 || !AddFits(component, kStateFromComponent)) {
        return false;
    }
    return read(component + kStateFromComponent, &stateOut, sizeof(stateOut), readContext);
}

bool CaptureCurrentNativeQueryFrame(
    const NativeScalarFrame& frame,
    std::uintptr_t expectedSite,
    NativeQueryFrame& frameOut) noexcept {
    ULONG_PTR stackLow{};
    ULONG_PTR stackHigh{};
    GetCurrentThreadStackLimits(&stackLow, &stackHigh);
    return ValidateNativeQueryFrame(
        frame, expectedSite, static_cast<std::uintptr_t>(stackLow),
        static_cast<std::uintptr_t>(stackHigh), ReadCurrentNativeMemorySeh, nullptr, frameOut);
}

bool ReadCurrentNativeMemory(
    std::uintptr_t address,
    void* destination,
    std::size_t size,
    void*) noexcept {
    return ReadCurrentNativeMemorySeh(address, destination, size, nullptr);
}

} // namespace shroudweave::flight

