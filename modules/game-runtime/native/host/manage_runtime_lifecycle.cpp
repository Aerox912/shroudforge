#include "abi/runtime_api.h"
#include "graphics/render_overlay.h"
#include "first-person/first_person_control.h"
#include "item-transfer/gifting_range_call.h"
#include "impact/prepare_mining_hooks.h"
#include "impact/mining_host_entries.h"
#include "engine-hooks/held_entry_preparation.h"
#include "engine-hooks/write_executable_bytes.h"
#include "ecs/entity_session.h"
#include "loaded-resources/loaded_resource_access.h"
#include "main-menu/native_main_menu.h"
#include <algorithm>
#include <cstring>
#include <string>
#include <nlohmann/json.hpp>
#include "host/runtime_lifecycle_state.h"
#include "player-ownership/service_lifetime.h"
#include "player-ownership/callback_player_scope.h"
#include "host/prepare_early_owner_probe.h"

using namespace ProviderImplementation;

namespace ProviderImplementation {
std::atomic<bool> provider_initialized{};
}

namespace {
bool prepare_player_ownership(const EngineHooks::HeldEntryToken& token, void*) noexcept {
    return PlayerOwnership::PrepareServiceRetirement(token) &&
        PlayerOwnership::PrepareCallbackPlayerWorld(token);
}

bool prepare_first_person(const EngineHooks::HeldEntryToken& token, void*) noexcept {
    return FirstPersonControl::Prepare(token);
}

struct HeldEntryExecutableReplacement {
    std::uintptr_t address{};
    const void* expected{};
    const void* replacement{};
    std::size_t size{};
};

bool replace_held_entry_executable_bytes(
    const EngineHooks::HeldEntryToken& token, void* context) noexcept {
    if (!token.ValidForCurrentThread() || !context) return false;
    const auto& replacement = *static_cast<const HeldEntryExecutableReplacement*>(context);
    try {
        return EngineHooks::ReplaceExecutableBytes(
            replacement.address, replacement.expected, replacement.replacement,
            replacement.size);
    } catch (...) {
        return false;
    }
}
}

extern "C" {
// Selected Stage1 diagnostic only. The Rust loader retains the actual image
// and generation before calling, and the provider requires a fresh held token.
KFC_RUNTIME_API unsigned __cdecl KfcRuntimePrepareEarlyOwnerProbe(
    const EngineHooks::HeldEntryProof* proof,const EarlyOwnerProbe::OwnerProofV1* owner,
    const SFALProbeAttachV1* attach,EarlyOwnerProbe::PublicationResultV1* result) noexcept {
    return EarlyOwnerProbe::Prepare(proof,owner,attach,result);
}
// Private bootstrap-only preparation; no public ABI table or Lua entrypoint.
KFC_RUNTIME_API bool __cdecl KfcRuntimePreparePlayerOwnership(
    const EngineHooks::HeldEntryProof* proof) {
    if (!proof) return false;
    return PlayerOwnership::NativeDetail::CompleteCallbackPlayerWorldPreparation(
        EngineHooks::RunHeldEntryPreparation(*proof, &prepare_player_ownership, nullptr));
}

// Private bootstrap-only preparation for the selected server-side gifting
// adapter. The feature completes its one-way publication gate only after the
// held-entry coordinator returns its final proof result.
KFC_RUNTIME_API bool __cdecl KfcRuntimePrepareGiftingRange(
    const EngineHooks::HeldEntryProof* proof, unsigned demand) {
    return GiftingRange::PrepareDedicatedServer(proof, demand);
}

// Private bootstrap-only mining preparation. Completion opens admission only
// after the real held-entry coordinator has returned its final result.
KFC_RUNTIME_API bool __cdecl KfcRuntimePrepareVeinMining(
    const EngineHooks::HeldEntryProof* proof, unsigned demand) {
    return ImpactRuntime::MiningHost::PrepareVeinMiningHooks(proof, demand);
}

KFC_RUNTIME_API unsigned __cdecl KfcRuntimeAbi() { return KFC_RUNTIME_ABI_VERSION; }

KFC_RUNTIME_API bool __cdecl KfcRuntimeInitialize() {
    try {
        const auto initialized = EcsRuntime::Initialize();
        ResourceRuntime::Initialize();
        NativeMainMenu::Initialize();
        provider_initialized.store(true, std::memory_order_release);
        return initialized;
    } catch (...) {
        provider_initialized.store(false, std::memory_order_release);
        NativeMainMenu::Shutdown();
        ResourceRuntime::Shutdown();
        EcsRuntime::Shutdown();
        return false;
    }
}

KFC_RUNTIME_API bool __cdecl KfcRuntimePrepareOverlay(unsigned demand) {
    try { return demand <= 1 && OverlayRuntime::PrepareGraphicsBoot(demand == 1); } catch (...) { return false; }
}

// Private bootstrap-only capability. A false demand returns without touching
// any First Person hooks, while a true demand must carry the held-entry proof.
KFC_RUNTIME_API bool __cdecl KfcRuntimePrepareHeldEntry(
    const EngineHooks::HeldEntryProof* proof, unsigned first_person_demand) {
    if (first_person_demand == 0) return true;
    if (first_person_demand != 1 || !proof) return false;
    return EngineHooks::RunHeldEntryPreparation(
        *proof, &prepare_first_person, nullptr);
}

// Private bootstrap-only route to the provider's one process-wide executable
// writer. The coordinator validates the same captured proof used by startup
// preparation before the callback can invoke ReplaceExecutableBytes.
KFC_RUNTIME_API bool __cdecl KfcRuntimeReplaceHeldEntryBytes(
    const EngineHooks::HeldEntryProof* proof, std::uintptr_t address,
    const void* expected, const void* replacement, std::size_t size) {
    if (!proof || !address || !expected || !replacement ||
        expected == replacement || size != sizeof(std::uint32_t)) return false;
    HeldEntryExecutableReplacement request{address, expected, replacement, size};
    return EngineHooks::RunHeldEntryPreparation(
        *proof, &replace_held_entry_executable_bytes, &request);
}

KFC_RUNTIME_API void __cdecl KfcRuntimeTick() {
    NativeMainMenu::Tick();
    try { EcsRuntime::Tick(); OverlayRuntime::TickServices(); } catch (...) { EcsRuntime::Shutdown(); }
}

KFC_RUNTIME_API void __cdecl KfcRuntimeShutdown() {
    ImpactRuntime::MiningHost::CloseMiningAdmission();
    ImpactRuntime::MiningHost::DrainMiningActions();
    provider_initialized.store(false, std::memory_order_release);
    FirstPersonControl::Shutdown();
    OverlayRuntime::ShutdownServices();
    KfcRuntimeNetworkServiceShutdown();
    NativeMainMenu::Shutdown();
    ResourceRuntime::Shutdown();
    EcsRuntime::Shutdown();
}
}
