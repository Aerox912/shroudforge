#include "player-ownership/flight_control_api.h"
#include "player-ownership/player_binding_api.h"
#include "player-ownership/flight_hook_entries.h"
#include "player-ownership/service_lifetime.h"
#include "engine-context/engine_context_state.h"
#include "engine-hooks/allocate_near_code.h"
#include "engine-hooks/write_executable_bytes.h"
#include <array>
#include <cstring>
#include <limits>

namespace {
std::mutex setup_mutex;
bool setup_attempted{},installed{};
// Every relay is retained after a possible publication. Failed executable
// transactions must never leave a return target pointing into freed memory.
void* scalar_relay{};
void* relay(std::uintptr_t site,std::uintptr_t target) {
    void* p=EngineHooks::AllocateNearCode(site,16);
    if(!p)return nullptr;
    const std::array<unsigned char,6> jump{0xff,0x25,0,0,0,0};
    std::memcpy(p,jump.data(),jump.size());
    std::memcpy(static_cast<unsigned char*>(p)+6,&target,sizeof(target));
    DWORD previous{};
    if(!VirtualProtect(p,16,PAGE_EXECUTE_READ,&previous) || !FlushInstructionCache(GetCurrentProcess(),p,16)) {
        VirtualFree(p,0,MEM_RELEASE);return nullptr; // not yet published
    }
    return p;
}
template<std::size_t N>bool call_bytes(std::uintptr_t site,void* target,std::array<unsigned char,N>& out) {
    static_assert(N>=5);
    const auto distance=static_cast<std::int64_t>(reinterpret_cast<std::uintptr_t>(target))-
        static_cast<std::int64_t>(site+5);
    if(distance<INT32_MIN || distance>INT32_MAX)return false;
    out.fill(0x90);out[0]=0xe8;const auto rel=static_cast<std::int32_t>(distance);
    std::memcpy(out.data()+1,&rel,sizeof(rel));return true;
}
std::int32_t install_hooks() {
    if(installed)return 0;
    if(setup_attempted)return 7;
    using namespace EngineContextImplementation;
    if(!PlayerOwnership::PrepareFlightEntries())return 3;
    const auto image=PlayerOwnership::CallbackPlayerImage();
    const auto scalar_site=image+0x7de0e;
    const std::array<unsigned char,8> scalar_original{0xf3,0x0f,0x10,0x05,0xae,0x37,0xb5,0};
    std::array<unsigned char,8> scalar_current;
    std::uint32_t stock{};
    if(!PlayerOwnership::CallbackPlayerWorldReady() ||
       !read_span(scalar_site,scalar_current.data(),scalar_current.size()) || scalar_current!=scalar_original ||
       !read(SF_FlightScalarAddress,stock) || stock!=0x3db2b8c2)return 3;
    HMODULE self{};
    if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,
        reinterpret_cast<LPCWSTR>(&install_hooks),&self))return 3;
    setup_attempted=true;
    scalar_relay=relay(scalar_site,reinterpret_cast<std::uintptr_t>(&SF_FlightScalarThunk));
    if(!scalar_relay)return 7;
    std::array<unsigned char,8> scalar_patch;
    if(!call_bytes(scalar_site,scalar_relay,scalar_patch))return 7;
    // Common World publication belongs to held-entry preparation. A failed
    // scalar write never removes/restores that shared original-forwarding hook.
    if(!EngineHooks::WriteExecutableBytes(scalar_site,scalar_patch.data(),scalar_patch.size(),scalar_original.data()))return 7;
    installed=true;return 0;
}
}
extern "C" std::int32_t KfcRuntimeFlightSetPeerIntentV1(std::uint64_t uid,std::uint64_t epoch,std::uint32_t enabled) {
    if(!uid || enabled>1 || (enabled && !epoch))return 1;
    try {
        std::scoped_lock lock(setup_mutex);
        auto& flight=PlayerOwnership::NativeFlightService();
        if(!enabled)return flight.Intents().Disable(uid)?0:7;
        if(!PlayerOwnership::ServiceLifetimeReady())return 3;
        if(flight.Fenced())return 7;
        // Qualify the supported image without publishing code. A player may
        // still be spawning after its transport connects; retain a retryable
        // result there instead of declaring the supported provider absent.
        if(!PlayerOwnership::PrepareFlightEntries())return 3;
        SFPlayerBindingV1 binding{};binding.size=sizeof(binding);binding.version=1;
        const auto status=KfcRuntimeReadPlayerBindingV1(uid,epoch,&binding);
        if(status)return status==3?4:status;
        if(binding.session!=epoch || binding.peer_uid!=uid || (!binding.world_uuid[0] && !binding.world_uuid[1]))return 3;
        if(!PlayerOwnership::ServiceLifetimeReady())return 3;
        const auto installed_status=install_hooks();
        if(installed_status)return installed_status;
        return flight.Intents().Set({epoch,{binding.world_uuid[0],binding.world_uuid[1]}},uid,true,
            binding.gameplay_key,binding.machine_key)?0:4;
    }catch(...){return 7;}
}
