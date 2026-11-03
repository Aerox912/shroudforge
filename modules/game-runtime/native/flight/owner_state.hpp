// SPDX-License-Identifier: MIT
#pragma once
#include <cstdint>

namespace shroudforge::flight {
// Serialized by the controller gate. Tokens come from trusted Rust mod
// identity/generation allocation, never from script arguments or game IDs.
class OwnerState final {
    std::uint64_t owner_ = 0;
    bool requested_ = false;
public:
    bool owns(std::uint64_t owner) const noexcept { return owner && owner == owner_; }
    bool claim(std::uint64_t owner) noexcept {
        if (!owner || (owner_ && owner_ != owner)) return false;
        owner_ = owner;
        return true;
    }
    bool set(std::uint64_t owner, bool enabled, bool ready) noexcept {
        if (!owns(owner)) return false;
        requested_ = enabled && ready;
        return !enabled || ready;
    }
    bool reset(std::uint64_t owner) noexcept {
        if (!owns(owner)) return false;
        requested_ = false;
        return true;
    }
    bool retire(std::uint64_t owner) noexcept {
        if (!reset(owner)) return false;
        owner_ = 0;
        return true;
    }
    void provider_reset() noexcept { requested_ = false; }
    bool requested() const noexcept { return requested_; }
};
}
