#include "player-ownership/vein_control_api.h"
#include "player-ownership/vein_intents.h"
#include "player-ownership/player_binding_api.h"
#include "player-ownership/service_lifetime.h"
#include "abi/runtime_api.h"
#include "impact/mining_host_entries.h"
#include <bcrypt.h>
#include <cstring>

namespace PlayerOwnership {
VeinIntents& NativeVeinIntents() noexcept {static VeinIntents intents;return intents;}
}
namespace {
using namespace PlayerOwnership;
bool output_header(const SFVeinLeaseV1* p) noexcept {
    if(!p)return false;
    __try{return p->size==sizeof(*p) && p->version==1;}
    __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
bool publish(SFVeinLeaseV1* p,const SFVeinLeaseV1& value) noexcept {
    __try{std::memcpy(p,&value,sizeof(value));return true;}
    __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
bool input_copy(const SFVeinUpdateV1* p,SFVeinUpdateV1& value) noexcept {
    if(!p)return false;
    __try{std::memcpy(&value,p,sizeof(value));return true;}
    __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
bool token_copy(const std::uint64_t* p,VeinToken& value) noexcept {
    if(!p)return false;
    __try{std::memcpy(value.data(),p,sizeof(value));return true;}
    __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
int binding(std::uint64_t uid,std::uint64_t session,VeinControlBinding& out) {
    out={};
    if(!session || !ImpactRuntime::MiningHost::MiningAdmissionOpen() || !ServiceLifetimeReady() || KfcRuntimeWorldSessionId()!=session)return 3;
    SFPlayerBindingV1 current{};current.size=sizeof(current);current.version=1;
    const auto status=KfcRuntimeReadPlayerBindingV1(uid,session,&current);
    if(status)return status;
    if(current.size!=sizeof(current) || current.version!=1 || current.flags || current.reserved ||
       current.peer_uid!=uid || current.session!=session || !current.entity_id ||
       !ImpactRuntime::MiningHost::MiningAdmissionOpen() || !ServiceLifetimeReady() || KfcRuntimeWorldSessionId()!=session)return 3;
    out={{current.session,{current.world_uuid[0],current.world_uuid[1]}},uid,current.gameplay_key,current.machine_key};
    return 0;
}
std::uint64_t monotonic_ms() noexcept {return GetTickCount64();}
bool new_token(VeinToken& token) noexcept {
    return BCryptGenRandom(nullptr,reinterpret_cast<PUCHAR>(token.data()),static_cast<ULONG>(sizeof(token)),
                           BCRYPT_USE_SYSTEM_PREFERRED_RNG)>=0;
}
int finish(std::uint64_t uid,int status,const VeinControlLease& lease,SFVeinLeaseV1* out) noexcept {
    SFVeinLeaseV1 value{};value.size=sizeof(value);value.version=1;
    if(!status){value.token[0]=lease.token[0];value.token[1]=lease.token[1];value.ttl_ms=lease.ttl_ms;value.renew_after_ms=lease.renew_after_ms;}
    if(publish(out,value))return status;
    if(lease.token!=VeinToken{})NativeVeinIntents().Revoke(uid,lease.token);
    return 7;
}
}
extern "C" std::int32_t KfcRuntimeBeginPeerVeinV1(std::uint64_t uid,std::uint64_t session,SFVeinLeaseV1* out) {
    if(!output_header(out))return 1;
    VeinControlLease lease;
    const auto status=NativeVeinIntents().Begin(uid,session,lease,binding,monotonic_ms,new_token);
    return finish(uid,status,lease,out);
}
extern "C" std::int32_t KfcRuntimeSetPeerVeinV1(std::uint64_t uid,std::uint64_t session,
                                               const SFVeinUpdateV1* update,SFVeinLeaseV1* out) {
    if(!output_header(out))return 1;
    SFVeinUpdateV1 input{};VeinControlLease lease;
    if(!input_copy(update,input) || input.size!=sizeof(input) || input.version!=1 || input.reserved || input.active>1)
        return finish(uid,1,lease,out);
    const auto status=NativeVeinIntents().Set(uid,session,{input.token[0],input.token[1]},input.sequence,
                                             input.active!=0,input.tier,lease,binding,monotonic_ms);
    return finish(uid,status,lease,out);
}
extern "C" std::int32_t KfcRuntimeRevokePeerVeinV1(std::uint64_t uid,const std::uint64_t* input) {
    VeinToken token{};if(!token_copy(input,token))return 1;
    return NativeVeinIntents().Revoke(uid,token);
}
