#pragma once
#include "read_actor_stack_identity.h"
#include "resolve_invocation_owner.h"
#include "flight_intents.h"
#include <bit>

namespace PlayerOwnership {
// Internal producer contract, never Lua/ABI input. A concrete producer must own
// the actual original World-call interval and service allocation gates before
// constructing this borrow. Validate cannot create that lifetime from values.
class FlightCallbackBorrow {
public:
    virtual ~FlightCallbackBorrow()=default;
    virtual ActorQueryMemory& Stack() noexcept=0;
    virtual bool Copy(CurrentSnapshot&) noexcept=0;
    virtual bool Validate() noexcept=0;
    virtual bool Release() noexcept=0;
};
// Called at the exact original scalar site. Only the returned XMM0 operand may
// change. No component/ItemInfo/stamina write, Scene reacquire or engine call.
// Always closes the supplied borrow, including Off and every refusal.
inline std::uint32_t select_invocation_flight(FlightCallbackBorrow& borrow,const FlightIntents& intents,
    const ActorScalarFrame& frame,std::uint32_t original_bits,std::uint32_t vertical_bits) noexcept {
    const auto selected=[&]() noexcept -> std::optional<FlightIntents::Read> {
        const auto magnitude=vertical_bits&0x7fffffffU;
        if(original_bits!=0x3db2b8c2 || magnitude>=0x7f800000 ||
           (!(vertical_bits&0x80000000U) && magnitude) || !borrow.Validate())return {};
        ActorQueryIdentity actor_before,actor_after;
        CurrentSnapshot before,after;
        if(!read_actor_stack_identity(borrow.Stack(),frame,actor_before) || !borrow.Copy(before))return {};
        const auto owner=resolve_invocation_owner(before,actor_before.entity_id);
        if(!owner)return {};
        const auto intent=intents.Get(owner->world,owner->platform_uid);
        if(!intent || !intent->enabled || intent->gameplay_key!=owner->gameplay_key ||
           intent->machine_key!=owner->machine_key || !borrow.Copy(after) ||
           !read_actor_stack_identity(borrow.Stack(),frame,actor_after) ||
           actor_before.manager!=actor_after.manager || actor_before.entity_id!=actor_after.entity_id ||
           actor_before.row_index!=actor_after.row_index || !borrow.Validate())return {};
        const auto confirmed=resolve_invocation_owner(after,actor_after.entity_id);
        if(!confirmed || *confirmed!=*owner || !intents.Unchanged(*intent))return {};
        return intent;
    }();
    const bool released=borrow.Release();
    if(!released || !selected || !intents.Unchanged(*selected))return original_bits;
    return std::bit_cast<std::uint32_t>(-1.4835298641951802f);
}
}
