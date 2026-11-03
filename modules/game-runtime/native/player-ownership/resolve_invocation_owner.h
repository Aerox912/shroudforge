#pragma once
#include "current_player_binding.h"
#include <optional>

namespace PlayerOwnership {
// Private, invocation-only ownership value. Unlike Binding, this claims no
// canonical entity-table membership or native generation and cannot be used as
// an ECS handle. The original actor invocation supplies the stack-owned full ID.
struct InvocationOwner {
    WorldIdentity world;
    std::uint64_t platform_uid{};
    std::uint32_t gameplay_key{},machine_key{},entity_id{};
    bool operator==(const InvocationOwner&) const=default;
};
inline std::optional<InvocationOwner> resolve_invocation_owner(const CurrentSnapshot& snapshot,
                                                               std::uint32_t actor_id) noexcept {
    if(!actor_id || !snapshot.world.session || snapshot.world.uuid==std::array<std::uint64_t,2>{})return {};
    const WorldPlayer* world{};
    for(const auto& row:snapshot.bindings) {
        if(!row.gameplay_key || row.entity_id!=actor_id)continue;
        if(world)return {};
        world=&row;
    }
    if(!world || !world->machine_key)return {};
    for(const auto& row:snapshot.bindings)
        if(&row!=world && row.gameplay_key==world->gameplay_key)return {};
    const PlayerRow* player{};
    for(const auto& row:snapshot.players) {
        if(row.gameplay_key!=world->gameplay_key)continue;
        if(player)return {};
        player=&row;
    }
    if(!player || player->platform_tag!=3 || !player->platform_uid || player->machine_key!=world->machine_key)
        return {};
    for(const auto& row:snapshot.players)
        if(&row!=player && row.gameplay_key && row.platform_uid==player->platform_uid)return {};
    const auto index=player->machine_key&0x7fU;
    if(index>=snapshot.machines.size())return {};
    const auto& machine=snapshot.machines[index];
    if(machine.key!=player->machine_key || machine.player_count>machine.players.size())return {};
    unsigned matches{};
    for(std::size_t i=0;i<machine.player_count;++i)if(machine.players[i]==player->gameplay_key)++matches;
    if(matches!=1)return {};
    return InvocationOwner{snapshot.world,player->platform_uid,player->gameplay_key,player->machine_key,actor_id};
}
}
