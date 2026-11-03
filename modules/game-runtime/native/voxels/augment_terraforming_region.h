#pragma once
#include <array>
#include <cstddef>
#include <cstdint>

// Private d7baa native layouts. Host must prove logical storage and live borrow.
namespace NativeTerraforming {
struct Int3 { std::int32_t x{},y{},z{}; };
struct UInt3 { std::uint32_t x{},y{},z{}; };
struct NativeFloatSpan { const float* data{}; std::uint64_t count{}; };
struct NativeQuantity {
    float* data{}; std::uint64_t count{}; std::uint8_t hint{};
    std::array<std::byte,3> opaque{}; float other{},aggregate{};
};
struct NativeCellBuffer { std::uint16_t* data{}; std::uint64_t count{}; UInt3 dimensions{}; };
using NativeTransform = std::uint8_t (__fastcall*)(const void*,NativeCellBuffer*,void*,const Int3*,const UInt3*);
struct RegionArguments {
    const void* visitor{}; NativeCellBuffer* buffer{}; void* auxiliary{};
    const Int3* offset{}; const UInt3* extent{};
};
struct Enumeration {
    alignas(16) std::array<std::byte,144> visitor{};
    Int3 low{},high{};
};
// Caller owns output throughout a27db0. False means forward original bounds/V.
bool PrepareEnumeration(const void* visitor,Int3 low,Int3 high,Enumeration& output) noexcept;
// Every native argument and the original AL/LastError are retained. No replay.
std::uint8_t InvokeRegion(NativeTransform original,const RegionArguments& arguments);
}
