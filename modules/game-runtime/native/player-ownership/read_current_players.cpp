#include "player-ownership/read_current_players.h"
#include "player-ownership/service_lifetime_admission.h"
#include "player-ownership/service_lifetime.h"
#include "player-ownership/session_snapshot_access.h"
#include "engine-context/engine_context_state.h"

using namespace EngineContextImplementation;
using namespace PlayerOwnership::NativeDetail;

namespace PlayerOwnership {
struct PreparedPlayerReader::State { Layout layout;std::uintptr_t image{}; };
PreparedPlayerReader::PreparedPlayerReader()=default;
PreparedPlayerReader::~PreparedPlayerReader()=default;
bool PreparedPlayerReader::Prepare() {
    state_.reset();
    if(!service_lifetime_admitted())return false;
    using namespace KfcRuntimeCompatibility::EnshroudedClient;
    std::scoped_lock lock(context_mutex);
    // Configure already admitted the exact owner profile. Dedicated servers
    // have no client execution-hook profile and leave exact_build_match false.
    if(!configured || image_target!="enshrouded_server.exe" ||
       image_sha256!="d7baa768eb00aa895d9f5a1cd8747ccf66858c7b542f50983092b2e3261588ee" ||
       image_size!=0x1dd1000 || !active_layout.dedicated_server || !active_layout.write.enabled)return false;
    const auto& w=active_layout.write;
    if(!w.capacity || w.capacity>256 || w.stride!=24)return false;
    auto prepared=std::make_unique<State>();
    prepared->layout=active_layout;
    prepared->image=reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
    if(!prepared->image)return false;
    state_=std::move(prepared);
    return true;
}
PlayerReadStatus PreparedPlayerReader::Copy(const EngineContext::EcsWriteGuard& guard,
    const EngineContext::EcsView& expected,std::uint64_t session,CurrentSnapshot& output) const noexcept {
    output={};
    if(!state_)return PlayerReadStatus::Unavailable;
    const auto& prepared=*state_;
    if(!session || expected.owners.image!=prepared.image || !expected.owners.dedicated_server ||
       expected.role!=EngineContext::EcsRole::LocalAuthority || !expected.scene_context ||
       guard.registry_lock_!=prepared.image+prepared.layout.write.registry_lock ||
       guard.manager_!=expected.manager || guard.registry_owner_!=expected.registry_owner ||
       !guard.Validate())return PlayerReadStatus::NoScope;
    EngineContext::EcsView current;
    if(!resolve_ecs(prepared.layout,current,nullptr,prepared.image) || current!=expected)return PlayerReadStatus::NoScope;
    auto service_lease=TryReadServiceLifetime();
    if(!service_lease)return PlayerReadStatus::Unavailable;
    SessionIdentity before;
    if(!session_identity(prepared.layout,expected,before))return PlayerReadStatus::Unavailable;
    NativeSnapshotMemory memory;
    NetworkSnapshot network;
    const auto status=copy_server_snapshot(memory,before.service,network);
    if(status!=SnapshotStatus::Ok) {
        switch(status) {
        case SnapshotStatus::Busy:return PlayerReadStatus::Busy;
        case SnapshotStatus::Invalid:return PlayerReadStatus::Invalid;
        case SnapshotStatus::ReleaseFailed:return PlayerReadStatus::ReleaseFailed;
        default:return PlayerReadStatus::Unreadable;
        }
    }
    CurrentSnapshot copied;
    copied.world={session,expected.scene_uuid};copied.players=network.players;copied.machines=network.machines;
    for(std::size_t i=0;i<player_limit;++i) {
        auto& binding=copied.bindings[i];
        std::uintptr_t address{};
        if(!add(expected.scene_context,0x1e0+i*0x148,address) ||
           !field(address,0,binding.gameplay_key) || !field(address,4,binding.machine_key) ||
           !field(address,0x18,binding.entity_id))return PlayerReadStatus::Unreadable;
    }
    SessionIdentity after;
    std::uintptr_t service_again{};
    if(!guard.Validate() || !resolve_ecs(prepared.layout,current,nullptr,prepared.image) || current!=expected ||
       !field(expected.scene_context,0x10,service_again) || service_again!=before.service ||
       !session_identity(prepared.layout,expected,after) || after!=before)return PlayerReadStatus::Unavailable;
    output=copied;
    return PlayerReadStatus::Ok;
}
}
