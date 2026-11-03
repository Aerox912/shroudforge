#include "player-ownership/player_binding_api.h"
#include "player-ownership/read_current_players.h"
#include "ecs/ecs_session_state.h"

namespace {
bool header(const SFPlayerBindingV1* p) noexcept {
    if(!p)return false;
    __try{return p->size==sizeof(*p) && p->version==1;}
    __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
bool publish(SFPlayerBindingV1* p,const SFPlayerBindingV1& value) noexcept {
    __try{std::memcpy(p,&value,sizeof(value));return true;}
    __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
int32_t resolve(std::uint64_t uid,std::uint64_t epoch,SFPlayerBindingV1& output) {
    using namespace PlayerOwnership;
    using namespace EcsImplementation;
    PreparedPlayerReader reader;
    if(!reader.Prepare())return 3;
    EcsRuntime::SessionGuard session;
    if(!session.TryAcquire(epoch) || !session.WritesAllowed())return 3;
    const auto view=session.View();
    EngineContext::EcsWriteGuard guard;
    if(!guard.TryAcquire(view))return 4;
    CurrentSnapshot snapshot;
    const auto status=reader.Copy(guard,view,epoch,snapshot);
    if(status!=PlayerReadStatus::Ok) {
        if(status==PlayerReadStatus::ReleaseFailed) {
            session.MarkWriteUncertain("player-snapshot-reader-release-failed");
            return 7;
        }
        return status==PlayerReadStatus::Busy || status==PlayerReadStatus::NoScope?4:3;
    }
    const PlayerRow* player{};
    for(const auto& row:snapshot.players) {
        if(!row.gameplay_key || row.platform_tag!=3 || row.platform_uid!=uid)continue;
        if(player)return 6;
        player=&row;
    }
    if(!player)return 3;
    for(auto& row:snapshot.bindings) {
        if(row.gameplay_key!=player->gameplay_key || !row.entity_id)continue;
        EntityView entity;
        std::size_t slot{};
        if(entity_for_id(row.entity_id,live_layout,entity,slot)!=EntityMembership::Present)continue;
        row.entity_generation=entity.generation;
        row.entity_current=entity.id==row.entity_id;
    }
    const auto resolved=PlayerOwnership::resolve(snapshot,uid);
    if(resolved.status==Status::Ambiguous)return 6;
    if(resolved.status!=Status::Resolved || !guard.Validate() || !session.WritesAllowed())return 3;
    const auto& value=resolved.binding;
    output={sizeof(output),1,value.world.session,{value.world.uuid[0],value.world.uuid[1]},
        value.platform_uid,value.gameplay_key,value.machine_key,value.entity_id,value.entity_generation,0,0};
    return 0;
}
}
int32_t KfcRuntimeReadPlayerBindingV1(uint64_t uid,uint64_t epoch,SFPlayerBindingV1* output) {
    if(!header(output))return 1;
    SFPlayerBindingV1 value{};value.size=sizeof(value);value.version=1;
    int32_t status=1;
    try {if(uid && epoch)status=resolve(uid,epoch,value);}catch(...){status=7;value={};value.size=sizeof(value);value.version=1;}
    // Alle Originalgates sind vor der fremden Ausgabekopie bereits freigegeben.
    return publish(output,value)?status:7;
}
