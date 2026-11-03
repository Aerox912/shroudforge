#pragma once
#include <cstdint>

namespace SemanticAttribution::Native {
struct CpuStateSupport final {
    std::uint64_t enabled_mask{};
    std::uint32_t enabled_size{};
};
// This queries the host CPU/OS only. A failed result forbids publishing relays.
bool QueryCpuStateSupport(CpuStateSupport&) noexcept;
}
