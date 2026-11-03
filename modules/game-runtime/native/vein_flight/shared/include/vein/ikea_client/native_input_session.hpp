// SPDX-License-Identifier: MIT
#pragma once

#include <cstdint>

namespace vein::ikea_client {

struct VeinInputTicket {
    std::uintptr_t context = 0;
    std::uint32_t channel = 0;
    std::uint64_t generation = 0;
};

enum class VeinInputUpdate { invalid, same_session, changed_session };

// Owned scalar identity for the current synchronous native input producer.
// It never dereferences or retains a game pointer beyond its opaque value.
class VeinInputSession final {
public:
    static constexpr std::uint64_t worker_input_max_age_ms = 32;

    [[nodiscard]] VeinInputUpdate observe_input(
        std::uintptr_t context, std::uint32_t channel, std::uint64_t completed_tick_ms,
        VeinInputTicket& ticket) noexcept;
    [[nodiscard]] bool is_current(const VeinInputTicket& ticket) const noexcept;
    [[nodiscard]] bool current_for_worker(std::uint64_t now_tick_ms,
                                          VeinInputTicket& ticket) const noexcept;

    void begin_transition() noexcept;
    void end_transition() noexcept;
    void invalidate() noexcept;

private:
    void clear_identity() noexcept;
    [[nodiscard]] bool advance_generation() noexcept;

    std::uintptr_t context_ = 0;
    std::uint32_t channel_ = 0;
    std::uint64_t generation_ = 0;
    std::uint64_t last_input_tick_ms_ = 0;
    std::uint32_t transition_depth_ = 0;
    bool identity_known_ = false;
    bool exhausted_ = false;
};

[[nodiscard]] constexpr bool needs_native_input_lifecycle_hooks(
    bool vein_policy_allowed, bool flight_transport_enabled) noexcept {
    return vein_policy_allowed || flight_transport_enabled;
}

} // namespace vein::ikea_client
