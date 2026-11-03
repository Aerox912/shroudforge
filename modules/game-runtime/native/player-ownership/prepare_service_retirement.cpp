#include "player-ownership/service_lifetime.h"
#include "player-ownership/prepare_service_retirement.h"
#include "player-ownership/service_lifetime_admission.h"
#include "engine-context/engine_context_state.h"
#include "engine-hooks/allocate_near_code.h"
#include "engine-hooks/write_executable_bytes.h"
#include <array>
#include <atomic>
#include <cstring>

namespace PlayerOwnership {
namespace {
std::atomic<bool> attempted{};
void* retirement_relay{}; // Retain after any possible code publication.
bool empty_registry(const EngineContextImplementation::Layout& layout,std::uintptr_t image) noexcept {
    using namespace EngineContextImplementation;
    const auto lock=image+layout.write.registry_lock;
    if(!try_shared(lock))return false;
    std::uint16_t count{};
    const bool empty=field(image,layout.write.registry_count,count) && count==0;
    release_shared(lock);
    return empty;
}
}
bool PrepareServiceRetirement(const EngineHooks::HeldEntryToken& token) noexcept {
    return NativeDetail::PrepareServiceRetirementAtImage(token,
        reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr)));
}
bool NativeDetail::PrepareServiceRetirementAtImage(const EngineHooks::HeldEntryToken& token,
                                                  std::uintptr_t image) noexcept {
    using namespace EngineContextImplementation;
    using namespace KfcRuntimeCompatibility::EnshroudedClient;
    // Bounded lifetime and positive-preparation reviews are accepted. Runtime
    // readiness still requires every held-entry/profile/publication check below.
    // This is not a runtime setting or permission for late installation.
    if(!service_lifetime_reviewed() || !token.ValidForCurrentThread())return false;
    try {
        Layout layout;
        {
            std::scoped_lock lock(context_mutex);
            if(!configured || image_target!="enshrouded_server.exe" || image_size!=0x1dd1000 ||
               image_sha256!="d7baa768eb00aa895d9f5a1cd8747ccf66858c7b542f50983092b2e3261588ee" ||
               !active_layout.dedicated_server || !active_layout.write.enabled ||
               active_layout.write.registry_lock!=0x14cbb80 || active_layout.write.registry_count!=0x14cd388)
                return false;
            layout=active_layout;
        }
        if(!image || image>UINTPTR_MAX-0x1dd1000 || !empty_registry(layout,image) ||
           !token.ValidForCurrentThread())return false;
        const auto site=image+0x9e9fd0;
        const std::array<unsigned char,5> original{0xe8,0x0b,0x2a,0,0};
        std::array<unsigned char,5> current{};
        if(!read_span(site,current.data(),current.size()) || current!=original)return false;
        bool expected=false;
        if(!attempted.compare_exchange_strong(expected,true))return false;
        HMODULE self{};
        if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,
            reinterpret_cast<LPCWSTR>(&NativeDetail::DestroySessionAfterReaderDrain),&self))return false;
        retirement_relay=EngineHooks::AllocateNearCode(site,16);
        if(!retirement_relay)return false;
        const std::array<unsigned char,6> absolute_jump{0xff,0x25,0,0,0,0};
        const auto target=reinterpret_cast<std::uintptr_t>(&NativeDetail::DestroySessionAfterReaderDrain);
        std::memcpy(retirement_relay,absolute_jump.data(),absolute_jump.size());
        std::memcpy(static_cast<unsigned char*>(retirement_relay)+6,&target,sizeof(target));
        DWORD old{};
        if(!VirtualProtect(retirement_relay,16,PAGE_EXECUTE_READ,&old) ||
           !FlushInstructionCache(GetCurrentProcess(),retirement_relay,16))return false;
        const auto distance=static_cast<std::int64_t>(reinterpret_cast<std::uintptr_t>(retirement_relay))-
            static_cast<std::int64_t>(site+5);
        if(distance<INT32_MIN || distance>INT32_MAX)return false;
        auto patch=original;const auto relative=static_cast<std::int32_t>(distance);
        std::memcpy(patch.data()+1,&relative,sizeof(relative));
        NativeDetail::SetOriginalSessionDestroy(reinterpret_cast<NativeDetail::OriginalSessionDestroy>(image+0x9ec9e0));
        if(!token.ValidForCurrentThread() || !empty_registry(layout,image) ||
           !EngineHooks::WriteExecutableBytes(site,patch.data(),patch.size(),original.data()))return false;
        // A failed postcondition leaves readers closed. A possibly published
        // pinned wrapper still forwards exactly once and cannot expose memory.
        if(!token.ValidForCurrentThread() || !empty_registry(layout,image))return false;
        return NativeDetail::PublishServiceLifetimeReady();
    }catch(...){return false;}
}
}
