#pragma once
#include "current_player_binding.h"
#include "read_actor_query_identity.h"
#include <bit>

namespace PlayerOwnership {
struct FlightIntent {
    Binding player;
    bool enabled{};
};
// Pure effect selection, not a native borrow or an authorization API. The
// adapter may apply the returned bits only at the validated glider clamp while
// owning the original World/actor lifetime. It must obtain current bindings
// on both sides of current-actor decoding within that same lifetime scope.
// Off and every refusal preserve original bits, including unusual float bits.
// No shared ItemInfo, stamina attribute or persistent engine field is changed.
inline std::uint32_t select_glider_pitch_lower(const FlightIntent& intent,
    const Result& before, const Result& after, const ActorQueryIdentity& actor,
    std::uintptr_t expected_manager, std::uint32_t original_bits,
    std::uint32_t native_vertical_bits) noexcept {
    constexpr std::uint32_t stock_lower_bits=0x3db2b8c2;
    // The original preceding COMISS tests XMM1 lane1 against zero (XMM12).
    // Preserve the native recovery branch when that lane is positive; never
    // turn its forced stock limit into an immediate -85-degree pitch.
    const auto vertical_magnitude=native_vertical_bits&0x7fffffffU;
    if (!intent.enabled || original_bits!=stock_lower_bits ||
        vertical_magnitude>=0x7f800000U ||
        (!(native_vertical_bits&0x80000000U) && vertical_magnitude!=0) || !expected_manager ||
        actor.manager!=expected_manager || !actor.entity_id || !intent.player.platform_uid ||
        !intent.player.world.session || intent.player.world.uuid==std::array<std::uint64_t,2>{} ||
        actor.entity_id!=intent.player.entity_id || !still_matches(intent.player,before) ||
        !still_matches(intent.player,after)) return original_bits;
    // Match the existing upward-glide limit. This removes the positive lower
    // pitch restriction for only this actor; it does not implement level hold.
    return std::bit_cast<std::uint32_t>(-1.4835298641951802f);
}
}
