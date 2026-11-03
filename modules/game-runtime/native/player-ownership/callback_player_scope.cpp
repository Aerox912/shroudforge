#include "player-ownership/callback_player_scope.h"
#include "player-ownership/service_lifetime.h"
#include "player-ownership/session_snapshot_access.h"
#include "ecs/try_read_borrowed_scene_epoch.h"
#include "engine-hooks/allocate_near_code.h"
#include "engine-hooks/write_executable_bytes.h"
#include <intrin.h>
#include <memory>
#include <atomic>
namespace PlayerOwnership {
using namespace EngineContextImplementation;
using namespace NativeDetail;
namespace {
struct PreparedWorld {Layout layout;std::uintptr_t image{};};
std::unique_ptr<PreparedWorld> state;
OriginalWorldInterval world;
std::atomic<bool> prepared{},ready{},fenced{};
std::atomic<OriginalWorldCall> original_world{};
std::mutex setup_mutex;
bool attempted{};
void* world_relay{}; // Process-resident after any possible publication.
bool same_world(const EngineContext::EcsView& a,const EngineContext::EcsView& b) noexcept {
    return a.owners==b.owners && a.role==b.role && a.scene_owner==b.scene_owner &&
        a.scene_entries==b.scene_entries && a.scene_entry==b.scene_entry && a.scene_context==b.scene_context &&
        a.scene_uuid==b.scene_uuid && a.execution_owner==b.execution_owner && a.registry_owner==b.registry_owner &&
        a.manager==b.manager && a.layout==b.layout;
}
bool empty_registry(const Layout& layout,std::uintptr_t image) noexcept {
    const auto lock=image+layout.write.registry_lock;
    if(!try_shared(lock))return false;
    std::uint16_t count{};const bool empty=field(image,layout.write.registry_count,count) && count==0;
    NativeSnapshotMemory memory;
    if(!memory.decrement(lock))fenced.store(true);
    return empty && !fenced.load();
}
}
bool CallbackPlayerWorldReady() noexcept {return ready.load() && ServiceLifetimeReady() && !fenced.load();}
bool CallbackPlayerFenced() noexcept {return fenced.load();}
std::uintptr_t CallbackPlayerImage() noexcept {return CallbackPlayerWorldReady()?state->image:0;}
OriginalWorldInterval::Lease TryReadCallbackWorld() noexcept {
    if(!CallbackPlayerWorldReady())return {};
    auto lease=world.TryRead();
    if(!CallbackPlayerWorldReady())lease.Release();
    return lease;
}
CallbackPlayerScope::CallbackPlayerScope(const OriginalWorldInterval::Lease& lease) noexcept:lease_(lease) {
    if(!lease_.Held() || !CallbackPlayerWorldReady())return;
    const auto lock=state->image+state->layout.write.registry_lock;
    if(!try_shared(lock))return;
    registry_=lock;
    if(!resolve_ecs(state->layout,view_,nullptr,state->image) || !same_world(view_,lease_.View()) ||
       !EcsRuntime::TryReadBorrowedSceneEpoch(view_,epoch_))return;
    valid_=true;
}
bool CallbackPlayerScope::Validate() noexcept {
    if(!valid_ || !registry_ || !lease_.Held() || !CallbackPlayerWorldReady())return false;
    EngineContext::EcsView current;std::uint64_t epoch{};
    return resolve_ecs(state->layout,current,nullptr,state->image) && current==view_ &&
        EcsRuntime::TryReadBorrowedSceneEpoch(current,epoch) && epoch==epoch_;
}
bool CallbackPlayerScope::Copy(CurrentSnapshot& output) noexcept {
    output={};
    if(!Validate() || view_.scene_context>UINTPTR_MAX-(0x1e0+15*0x148+0x1c))return false;
    auto service_lease=TryReadServiceLifetime();
    if(!service_lease)return false;
    SessionIdentity before,after;
    if(!session_identity(state->layout,view_,before))return false;
    NativeSnapshotMemory memory;NetworkSnapshot network;
    const auto result=copy_server_snapshot(memory,before.service,network);
    if(result==SnapshotStatus::ReleaseFailed)fenced.store(true);
    if(result!=SnapshotStatus::Ok)return false;
    CurrentSnapshot copied;copied.world={epoch_,view_.scene_uuid};
    copied.players=network.players;copied.machines=network.machines;
    for(std::size_t i=0;i<player_limit;++i) {
        const auto address=view_.scene_context+0x1e0+i*0x148;
        auto& player=copied.bindings[i];
        if(!field(address,0,player.gameplay_key) || !field(address,4,player.machine_key) ||
           !field(address,0x18,player.entity_id))return false;
    }
    if(!Validate() || !session_identity(state->layout,view_,after) || after!=before)return false;
    output=copied;return true;
}
bool CallbackPlayerScope::Release() noexcept {
    valid_=false;
    if(registry_) {
        NativeSnapshotMemory memory;
        if(!memory.decrement(registry_))fenced.store(true);
        registry_=0;
    }
    return !fenced.load();
}
bool CopyCallbackPlayers(const OriginalWorldInterval::Lease& lease,CurrentSnapshot& output) noexcept {
    CallbackPlayerScope scope(lease);
    const bool copied=scope.Copy(output);
    const bool released=scope.Release();
    if(!copied || !released || !CallbackPlayerWorldReady()){output={};return false;}
    return true;
}
bool BeginCallbackWorld(std::uintptr_t return_pc,std::uintptr_t scene_arg,std::uintptr_t world_arg) noexcept {
    if(!CallbackPlayerWorldReady() || return_pc!=state->image+0x7b41c9)return false;
    const auto& s=*state;const auto lock=s.image+s.layout.write.registry_lock;
    if(!try_shared(lock))return false;
    NativeSnapshotMemory memory;EngineContext::EcsView current;std::uint32_t scene_gate{};
    const bool valid=resolve_ecs(s.layout,current,nullptr,s.image) &&
        current.owners.dedicated_server && current.role==EngineContext::EcsRole::LocalAuthority &&
        current.scene_context==world_arg && current.scene_owner<=UINTPTR_MAX-0x5dc &&
        current.scene_owner+0x5dc==scene_arg &&
        field(current.scene_owner,0x5d8,scene_gate) && scene_gate==0x80000000U;
    if(!memory.decrement(lock))fenced.store(true);
    return valid && CallbackPlayerWorldReady() && world.Begin(current);
}
void EndCallbackWorld() noexcept {world.End();}
bool PrepareCallbackPlayerWorld(const EngineHooks::HeldEntryToken& token) noexcept {
    return NativeDetail::PrepareCallbackWorldAtImage(token,reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr)));
}
bool NativeDetail::PrepareCallbackWorldAtImage(const EngineHooks::HeldEntryToken& token,std::uintptr_t image) noexcept {
    if(!token.ValidForCurrentThread() || !ServiceLifetimeReady())return false;
    try {
        std::scoped_lock setup(setup_mutex);
        if(attempted || !image || image>UINTPTR_MAX-0x1dd1000)return false;
        Layout layout;
        {
            using namespace KfcRuntimeCompatibility::EnshroudedClient;
            std::scoped_lock lock(context_mutex);
            if(!configured || image_target!="enshrouded_server.exe" || image_size!=0x1dd1000 ||
               image_sha256!="d7baa768eb00aa895d9f5a1cd8747ccf66858c7b542f50983092b2e3261588ee" ||
               !active_layout.dedicated_server || !active_layout.ecs || !active_layout.write.enabled ||
               !active_layout.write.capacity || active_layout.write.capacity>256 || active_layout.write.stride!=24 ||
               active_layout.write.scene_lock!=0x5d8)return false;
            layout=active_layout;
        }
        if(!empty_registry(layout,image) || !token.ValidForCurrentThread())return false;
        const auto site=image+0x7b41c4;
        constexpr std::array<unsigned char,5> stock{0xe8,0x77,0xe7,0xff,0xff};
        std::array<unsigned char,5> current{};
        if(!read_span(site,current.data(),current.size()) || current!=stock)return false;
        attempted=true;
        HMODULE self{};
        if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,
            reinterpret_cast<LPCWSTR>(&SF_FlightWorldCall),&self))return false;
        state=std::make_unique<PreparedWorld>();state->layout=layout;state->image=image;
        original_world.store(reinterpret_cast<OriginalWorldCall>(image+0x7b2940));
        world_relay=EngineHooks::AllocateNearCode(site,16);
        if(!world_relay)return false;
        constexpr std::array<unsigned char,6> jump{0xff,0x25,0,0,0,0};
        const auto target=reinterpret_cast<std::uintptr_t>(&SF_FlightWorldCall);
        std::memcpy(world_relay,jump.data(),jump.size());
        std::memcpy(static_cast<unsigned char*>(world_relay)+6,&target,sizeof(target));
        DWORD old{};
        if(!VirtualProtect(world_relay,16,PAGE_EXECUTE_READ,&old) ||
           !FlushInstructionCache(GetCurrentProcess(),world_relay,16))return false;
        const auto distance=static_cast<std::int64_t>(reinterpret_cast<std::uintptr_t>(world_relay))-
            static_cast<std::int64_t>(site+5);
        if(distance<INT32_MIN || distance>INT32_MAX)return false;
        auto patch=stock;const auto relative=static_cast<std::int32_t>(distance);
        std::memcpy(patch.data()+1,&relative,sizeof(relative));
        if(!token.ValidForCurrentThread() || !ServiceLifetimeReady() || !empty_registry(layout,image) ||
           !EngineHooks::WriteExecutableBytes(site,patch.data(),patch.size(),stock.data()))return false;
        // Retain original target, state and relay even when a postcondition fails.
        // Such a wrapper forwards stock exactly once while all effects stay closed.
        if(!token.ValidForCurrentThread() || !ServiceLifetimeReady() || !empty_registry(layout,image) ||
           !read_span(site,current.data(),current.size()) || current!=patch)return false;
        prepared.store(true);return true;
    }catch(...){return false;}
}
bool NativeDetail::CompleteCallbackPlayerWorldPreparation(bool coordinated) noexcept {
    const bool published=prepared.exchange(false);
    if(!published)return false;
    if(!coordinated || !ServiceLifetimeReady() || fenced.load()) {
        fenced.store(true);return false;
    }
    ready.store(true);
    return CallbackPlayerWorldReady();
}
std::uintptr_t NativeDetail::ForwardCallbackWorldCall(OriginalWorldCall original,std::uintptr_t return_pc,
    std::uintptr_t output,std::uintptr_t scene_counter,std::uintptr_t world_arg,std::uintptr_t time) {
    const auto error=GetLastError();
    const bool started=BeginCallbackWorld(return_pc,scene_counter,world_arg);
    SetLastError(error);std::uintptr_t result{};
    // Qualified native signature: four integer/pointer register arguments, no
    // stack arguments. Caller consumes output[0], not an XMM or flags result.
    __try {result=original(output,scene_counter,world_arg,time);}
    __finally {const auto after=GetLastError();if(started)EndCallbackWorld();SetLastError(after);}
    return result;
}
}
extern "C" std::uintptr_t SF_FlightWorldCall(std::uintptr_t output,std::uintptr_t scene_counter,
    std::uintptr_t world,std::uintptr_t time) {
    const auto original=PlayerOwnership::original_world.load();
    if(!original)__fastfail(FAST_FAIL_INVALID_ARG);
    return PlayerOwnership::NativeDetail::ForwardCallbackWorldCall(original,
        reinterpret_cast<std::uintptr_t>(_ReturnAddress()),output,scene_counter,world,time);
}
