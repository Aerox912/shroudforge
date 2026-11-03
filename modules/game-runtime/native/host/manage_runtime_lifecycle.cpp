#include "abi/runtime_api.h"
#include "graphics/render_overlay.h"
#include "ecs/entity_session.h"
#include "loaded-resources/loaded_resource_access.h"
#include "main-menu/native_main_menu.h"
#include <algorithm>
#include <cstring>
#include <string>
#include <nlohmann/json.hpp>
#include "host/runtime_lifecycle_state.h"
#include "engine-hooks/install_engine_hooks.h"
#include "engine-hooks/held_entry_preparation.h"
// The separately owned First Person translation unit supplies these real
// functions. No fallback implementation or alternate hook owner is provided.
namespace FirstPersonControl {
bool Prepare(const EngineHooks::HeldEntryToken&) noexcept;
void Shutdown() noexcept;
}

using namespace ProviderImplementation;

namespace ProviderImplementation {
std::atomic<bool> provider_initialized{};
}

extern "C" {
KFC_RUNTIME_API unsigned __cdecl KfcRuntimeAbi() { return KFC_RUNTIME_ABI_VERSION; }
KFC_RUNTIME_API bool __cdecl KfcRuntimeStartupReady() {
    return provider_initialized.load(std::memory_order_acquire)&&EngineHooks::StartupReady();
}
KFC_RUNTIME_API bool __cdecl KfcRuntimePrepareHeldEntry(const EngineHooks::HeldEntryProof* proof,unsigned demand) {
    if(demand==0)return true;
    if(demand!=1||!proof)return false;
    return EngineHooks::RunHeldEntryPreparation(*proof,+[](const EngineHooks::HeldEntryToken& token,void*) noexcept {
        return FirstPersonControl::Prepare(token);
    },nullptr);
}

KFC_RUNTIME_API bool __cdecl KfcRuntimeInitialize() {
    try {
        const auto initialized = EcsRuntime::Initialize();
        ResourceRuntime::Initialize();
        NativeMainMenu::Initialize();
        provider_initialized.store(true, std::memory_order_release);
        return initialized;
    } catch (...) {
        provider_initialized.store(false, std::memory_order_release);
        FirstPersonControl::Shutdown();
        NativeMainMenu::Shutdown();
        ResourceRuntime::Shutdown();
        EcsRuntime::Shutdown();
        return false;
    }
}

KFC_RUNTIME_API bool __cdecl KfcRuntimePrepareOverlay(unsigned demand) {
    try { return demand <= 1 && OverlayRuntime::PrepareGraphicsBoot(demand == 1); } catch (...) { return false; }
}

KFC_RUNTIME_API void __cdecl KfcRuntimeTick() {
    try { NativeMainMenu::Tick(); EcsRuntime::Tick(); OverlayRuntime::TickServices(); }
    catch (...) { KfcRuntimeShutdown(); }
}

KFC_RUNTIME_API void __cdecl KfcRuntimeShutdown() {
    provider_initialized.store(false, std::memory_order_release);
    FirstPersonControl::Shutdown();
    OverlayRuntime::ShutdownServices();
    KfcRuntimeNetworkServiceShutdown();
    NativeMainMenu::Shutdown();
    ResourceRuntime::Shutdown();
    EcsRuntime::Shutdown();
}
}
