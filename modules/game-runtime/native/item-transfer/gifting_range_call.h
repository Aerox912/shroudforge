#pragma once

#include "../engine-hooks/held_entry_preparation.h"

#include <atomic>
#include <array>
#include <cstdint>
#include <cstring>
#include <limits>

namespace GiftingRange {

// This one-way gate keeps the stock getter active until the held-entry
// coordinator itself confirms success. Failure can never reopen the adapter.
class PublicationGate final {
public:
    bool MarkPrepared() noexcept {
        auto expected = State::Stock;
        return state_.compare_exchange_strong(
            expected, State::Preparing, std::memory_order_acq_rel);
    }

    bool Complete(bool coordinated) noexcept {
        auto expected = State::Preparing;
        const auto next = coordinated ? State::Active : State::Failed;
        return state_.compare_exchange_strong(
            expected, next, std::memory_order_acq_rel);
    }

    bool Active() const noexcept {
        return state_.load(std::memory_order_acquire) == State::Active;
    }

private:
    enum class State : std::uint8_t { Stock, Preparing, Active, Failed };
    std::atomic<State> state_{State::Stock};
};

using DistanceGetter = float (__cdecl*)() noexcept;

inline bool SelectsDedicatedServerInstall(unsigned demand, bool dedicated_server,
                                          bool provider_ready) noexcept {
    return demand == 1 && dedicated_server && provider_ready;
}

inline bool BuildDirectCallPatch(std::uintptr_t site,
                                 const std::array<std::uint8_t, 5>& expected,
                                 std::uintptr_t expected_target,
                                 std::uintptr_t relay,
                                 std::array<std::uint8_t, 5>& replacement) noexcept {
    if (!site || site > std::numeric_limits<std::uintptr_t>::max() - 5 ||
        expected[0] != 0xe8 || !relay) return false;
    std::int32_t original_displacement{};
    std::memcpy(&original_displacement, expected.data() + 1, sizeof(original_displacement));
    const auto original_target = static_cast<std::int64_t>(site + 5) + original_displacement;
    if (original_target < 0 || static_cast<std::uintptr_t>(original_target) != expected_target)
        return false;
    const auto delta = static_cast<std::int64_t>(relay) - static_cast<std::int64_t>(site + 5);
    if (delta < (std::numeric_limits<std::int32_t>::min)() ||
        delta > (std::numeric_limits<std::int32_t>::max)()) return false;
    replacement = expected;
    const auto displacement = static_cast<std::int32_t>(delta);
    std::memcpy(replacement.data() + 1, &displacement, sizeof(displacement));
    return true;
}

inline float TransferDistance(const PublicationGate& gate, DistanceGetter stock) noexcept {
    if (gate.Active()) return std::numeric_limits<float>::max();
    return stock ? stock() : 0.0F;
}

// Startup-only adapter. Demand 0 is a strict no-op; demand 1 requires the
// dedicated-server target, its validated mapped PE and exact gifting code
// guards, plus a genuine held-entry proof.
bool PrepareDedicatedServer(const EngineHooks::HeldEntryProof* proof, unsigned demand) noexcept;

}
