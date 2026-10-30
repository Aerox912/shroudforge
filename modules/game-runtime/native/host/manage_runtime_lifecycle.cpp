#include "abi/runtime_api.h"
#include "graphics/render_overlay.h"
#include "ecs/entity_session.h"
#include "loaded-resources/loaded_resource_access.h"
#include <algorithm>
#include <cstring>
#include <string>
#include <nlohmann/json.hpp>
#include "host/runtime_lifecycle_state.h"

using namespace ProviderImplementation;

namespace ProviderImplementation {
std::atomic<bool> provider_initialized{};
}

extern "C" {
KFC_RUNTIME_API unsigned __cdecl KfcRuntimeAbi() { return KFC_RUNTIME_ABI_VERSION; }

KFC_RUNTIME_API bool __cdecl KfcRuntimeInitialize() {
    try {
        const auto initialized = EcsRuntime::Initialize();
        ResourceRuntime::Initialize();
        provider_initialized.store(true, std::memory_order_release);
        return initialized;
    } catch (...) {
        provider_initialized.store(false, std::memory_order_release);
        ResourceRuntime::Shutdown();
        EcsRuntime::Shutdown();
        return false;
    }
}

KFC_RUNTIME_API bool __cdecl KfcRuntimePrepareOverlay(unsigned demand) {
    try { return demand <= 1 && OverlayRuntime::PrepareGraphicsBoot(demand == 1); } catch (...) { return false; }
}

KFC_RUNTIME_API void __cdecl KfcRuntimeTick() {
    try { EcsRuntime::Tick(); OverlayRuntime::TickServices(); } catch (...) { EcsRuntime::Shutdown(); }
}

KFC_RUNTIME_API void __cdecl KfcRuntimeShutdown() {
    provider_initialized.store(false, std::memory_order_release);
    OverlayRuntime::ShutdownServices();
    KfcRuntimeNetworkServiceShutdown();
    ResourceRuntime::Shutdown();
    EcsRuntime::Shutdown();
}
}
