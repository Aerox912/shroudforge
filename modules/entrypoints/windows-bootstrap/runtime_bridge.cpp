#include "runtime_bridge.h"
#include "../../game-runtime/native/abi/runtime_api.h"
#include <windows.h>
#include <filesystem>

namespace {
HMODULE provider{};
using AbiFunction = decltype(&KfcRuntimeAbi);
using InitializeFunction = decltype(&KfcRuntimeInitialize);
using VoidFunction = decltype(&KfcRuntimeTick);
using StatusFunction = decltype(&KfcRuntimeStatus);
using ReplaceHeldEntryBytesFunction = bool (__cdecl*)(
    const EngineHooks::HeldEntryProof*, std::uintptr_t, const void*, const void*, std::size_t);
VoidFunction tick{}, shutdown{};
StatusFunction status{};
ReplaceHeldEntryBytesFunction replace_held_entry_bytes{};
std::string error{"provider-not-loaded"};
std::string first_person_preparation_error;
std::string vein_mining_preparation_error;
std::string gifting_range_preparation_error;
std::uintptr_t early_probe_runtime_image{};
using EarlyProbeRevoke=void (__cdecl*)();
EarlyProbeRevoke early_probe_revoke{};
unsigned __cdecl publish_early_probe(const EngineHooks::HeldEntryProof* proof,
    const EarlyOwnerProbe::OwnerProofV1* owner,const SFALProbeAttachV1* attach,
    EarlyOwnerProbe::PublicationResultV1* result) noexcept {
    if(!provider || !proof || !owner || !attach || !result ||
       owner->runtime_image!=early_probe_runtime_image)return static_cast<unsigned>(EarlyOwnerProbe::PublicationStatus::Refused);
    using Prepare=EarlyOwnerProbe::Publish;
    const auto prepare=reinterpret_cast<Prepare>(GetProcAddress(provider,"KfcRuntimePrepareEarlyOwnerProbe"));
    if(!prepare)return static_cast<unsigned>(EarlyOwnerProbe::PublicationStatus::Refused);
    return prepare(proof,owner,attach,result);
}

void record_vein_mining_preparation_error(bool demanded, const std::string& detail) {
    if (!demanded) return;
    vein_mining_preparation_error = "vein_mining_preparation=unavailable-" + detail;
    OutputDebugStringA(("[ESM Vein Mining] " + vein_mining_preparation_error + "\n").c_str());
}

void record_gifting_range_preparation_error(bool demanded, const std::string& detail) {
    if (!demanded) return;
    gifting_range_preparation_error = "gifting_range_preparation=unavailable-" + detail;
    OutputDebugStringA(("[Shroudweave Unlimited Gifting Range] " + gifting_range_preparation_error + "\n").c_str());
}
}
namespace EcsRuntime {
EarlyOwnerProbe::BootstrapHostV1 EarlyProbeBootstrapHost(std::uintptr_t runtime_image,
    const EngineHooks::HeldEntryProof& proof) {
    EarlyOwnerProbe::BootstrapHostV1 host;
    if(!runtime_image || (early_probe_runtime_image && early_probe_runtime_image!=runtime_image))return host;
    early_probe_runtime_image=runtime_image;
    early_probe_revoke=reinterpret_cast<EarlyProbeRevoke>(GetProcAddress(
        reinterpret_cast<HMODULE>(runtime_image),"shroudforge_revoke_early_owner_probe"));
    host.runtime_image=runtime_image;
    host.executable_image=reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
    host.held_entry=&proof;
    if(provider && GetProcAddress(provider,"KfcRuntimePrepareEarlyOwnerProbe"))host.publish=&publish_early_probe;
    return host;
}
bool Initialize(bool overlay_demand,
                const EngineHooks::HeldEntryProof* held_entry_proof,
                bool first_person_demand,
                std::optional<unsigned> vein_mining_demand,
                std::optional<unsigned> gifting_range_demand) {
    first_person_preparation_error.clear();
    vein_mining_preparation_error.clear();
    gifting_range_preparation_error.clear();
    replace_held_entry_bytes = nullptr;
    const bool vein_mining_demanded = vein_mining_demand && *vein_mining_demand == 1u;
    const bool has_supported_vein_mining_demand = vein_mining_demand && *vein_mining_demand <= 1u;
    const bool gifting_range_demanded = gifting_range_demand && *gifting_range_demand == 1u;
    const bool has_supported_gifting_range_demand = gifting_range_demand && *gifting_range_demand <= 1u;
    HMODULE self{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        reinterpret_cast<LPCWSTR>(&Initialize), &self)) {
        record_vein_mining_preparation_error(vein_mining_demanded, "bootstrap-module-unavailable");
        record_gifting_range_preparation_error(gifting_range_demanded, "bootstrap-module-unavailable");
        return false;
    }
    wchar_t path[32768]{};
    if (!GetModuleFileNameW(self, path, 32768)) {
        record_vein_mining_preparation_error(vein_mining_demanded, "bootstrap-path-unavailable");
        record_gifting_range_preparation_error(gifting_range_demanded, "bootstrap-path-unavailable");
        return false;
    }
    const auto dll = std::filesystem::path(path).parent_path() / L"shroudforge" / L"kfc-runtime.dll";
    provider = LoadLibraryExW(dll.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!provider) {
        error = "kfc-runtime.dll-load-failed";
        record_vein_mining_preparation_error(vein_mining_demanded, "native-runtime-provider-unavailable");
        record_gifting_range_preparation_error(gifting_range_demanded, "native-runtime-provider-unavailable");
        return false;
    }
    const auto abi = reinterpret_cast<AbiFunction>(GetProcAddress(provider, "KfcRuntimeAbi"));
    const auto initialize = reinterpret_cast<InitializeFunction>(GetProcAddress(provider, "KfcRuntimeInitialize"));
    tick = reinterpret_cast<VoidFunction>(GetProcAddress(provider, "KfcRuntimeTick"));
    shutdown = reinterpret_cast<VoidFunction>(GetProcAddress(provider, "KfcRuntimeShutdown"));
    status = reinterpret_cast<StatusFunction>(GetProcAddress(provider, "KfcRuntimeStatus"));
    replace_held_entry_bytes = reinterpret_cast<ReplaceHeldEntryBytesFunction>(
        GetProcAddress(provider, "KfcRuntimeReplaceHeldEntryBytes"));
    const auto actual_abi = abi ? abi() : 0;
    if (!abi || actual_abi != KFC_RUNTIME_ABI_VERSION || !initialize || !tick || !shutdown || !status) {
        error = abi
            ? "incompatible-runtime-provider-ABI:expected=" + std::to_string(KFC_RUNTIME_ABI_VERSION) +
                ",actual=" + std::to_string(actual_abi)
            : "missing-runtime-provider-ABI-export";
        tick = nullptr; shutdown = nullptr; status = nullptr;
        replace_held_entry_bytes = nullptr;
        record_vein_mining_preparation_error(vein_mining_demanded, "native-runtime-provider-ABI-unavailable");
        record_gifting_range_preparation_error(gifting_range_demanded, "native-runtime-provider-ABI-unavailable");
        FreeLibrary(provider); provider = nullptr; return false;
    }
    if (first_person_demand) {
        using PrepareHeldEntryFunction = bool (__cdecl*)(
            const EngineHooks::HeldEntryProof*, unsigned);
        const auto prepare_held_entry = reinterpret_cast<PrepareHeldEntryFunction>(
            GetProcAddress(provider, "KfcRuntimePrepareHeldEntry"));
        bool prepared{};
        if (held_entry_proof && prepare_held_entry) {
            try { prepared = prepare_held_entry(held_entry_proof, 1u); }
            catch (...) { prepared = false; }
        }
        if (!prepared) {
            first_person_preparation_error = !prepare_held_entry
                ? "first_person_preparation=restart-required-provider-lacks-held-entry-export"
                : "first_person_preparation=restart-required-held-entry-preparation-failed";
            OutputDebugStringA(("[ESM first person] " + first_person_preparation_error + "\n").c_str());
        }
    }
    const auto initialized = initialize();
    // Initialize first configures the exact engine owner profile. The genuine
    // PE entry gate is still held; UI/package-native startup has not begun.
    bool player_ownership_export_available{};
    bool player_ownership_prepared{};
    using PreparePlayerOwnershipFunction = bool (__cdecl*)(const EngineHooks::HeldEntryProof*);
    const auto prepare_player_ownership = reinterpret_cast<PreparePlayerOwnershipFunction>(
        GetProcAddress(provider, "KfcRuntimePreparePlayerOwnership"));
    player_ownership_export_available = prepare_player_ownership != nullptr;
    if (initialized && held_entry_proof && player_ownership_export_available) {
        try { player_ownership_prepared = prepare_player_ownership(held_entry_proof); }
        catch (...) { player_ownership_prepared = false; }
        if (!player_ownership_prepared)
            OutputDebugStringA("[ESM player ownership] optional native peer adapter remains unavailable\n");
    }
    if (has_supported_vein_mining_demand) {
        if (vein_mining_demanded && !initialized) {
            record_vein_mining_preparation_error(true, "native-runtime-initialization-failed");
        } else if (vein_mining_demanded && !held_entry_proof) {
            record_vein_mining_preparation_error(vein_mining_demanded, "held-entry-proof-unavailable");
        } else if (vein_mining_demanded && !player_ownership_export_available) {
            record_vein_mining_preparation_error(true, "player-ownership-prepare-export-unavailable");
        } else if (vein_mining_demanded && !player_ownership_prepared) {
            record_vein_mining_preparation_error(true, "player-ownership-preparation-failed");
        } else {
            using PrepareVeinMiningFunction = bool (__cdecl*)(
                const EngineHooks::HeldEntryProof*, unsigned);
            const auto prepare_vein_mining = reinterpret_cast<PrepareVeinMiningFunction>(
                GetProcAddress(provider, "KfcRuntimePrepareVeinMining"));
            if (initialized && !held_entry_proof) {
                record_vein_mining_preparation_error(vein_mining_demanded, "held-entry-proof-unavailable");
            } else if (initialized && held_entry_proof && !prepare_vein_mining) {
                record_vein_mining_preparation_error(vein_mining_demanded, "provider-lacks-private-prepare-export");
            } else if (initialized && held_entry_proof && prepare_vein_mining) {
                bool prepared{};
                try { prepared = prepare_vein_mining(held_entry_proof, *vein_mining_demand); }
                catch (...) { prepared = false; }
                if (vein_mining_demanded && !prepared)
                    record_vein_mining_preparation_error(true, "provider-refused-qualified-prepare");
            }
        }
    }
    // Unlimited Gifting is an independent transfer-distance callsite adapter.
    // Its dedicated path must not be conditioned on optional player ownership.
    if (has_supported_gifting_range_demand && gifting_range_demanded) {
        if (!initialized) {
            record_gifting_range_preparation_error(true, "native-runtime-initialization-failed");
        } else if (!held_entry_proof) {
            record_gifting_range_preparation_error(true, "held-entry-proof-unavailable");
        } else {
            using PrepareGiftingRangeFunction = bool (__cdecl*)(
                const EngineHooks::HeldEntryProof*, unsigned);
            const auto prepare_gifting_range = reinterpret_cast<PrepareGiftingRangeFunction>(
                GetProcAddress(provider, "KfcRuntimePrepareGiftingRange"));
            if (!prepare_gifting_range) {
                record_gifting_range_preparation_error(true, "provider-lacks-private-prepare-export");
            } else {
                bool prepared{};
                try { prepared = prepare_gifting_range(held_entry_proof, *gifting_range_demand); }
                catch (...) { prepared = false; }
                if (!prepared)
                    record_gifting_range_preparation_error(true, "provider-refused-qualified-prepare");
            }
        }
    }
    const auto prepare_overlay = reinterpret_cast<bool (__cdecl*)(unsigned)>(GetProcAddress(provider, "KfcRuntimePrepareOverlay"));
    if (prepare_overlay) prepare_overlay(overlay_demand ? 1u : 0u);
    else if (overlay_demand) OutputDebugStringA("[ESM overlay] provider lacks early broker; restart with matching native provider\n");
    return initialized;
}
void Tick() { if (tick) tick(); }
void Shutdown() { if(early_probe_revoke)early_probe_revoke();if (shutdown) shutdown(); }
std::string VeinMiningPreparationError() { return vein_mining_preparation_error; }
std::string GiftingRangePreparationError() { return gifting_range_preparation_error; }
bool ReplaceHeldEntryBytes(const EngineHooks::HeldEntryProof& proof,
                           std::uintptr_t address, const void* expected,
                           const void* replacement, std::size_t size) {
    if (!provider || !replace_held_entry_bytes || !address || !expected ||
        !replacement || expected == replacement || size != sizeof(std::uint32_t))
        return false;
    try {
        return replace_held_entry_bytes(
            &proof, address, expected, replacement, size);
    } catch (...) {
        return false;
    }
}
std::string Status() {
    if (!status) return error;
    char text[2048]{}; status(text, sizeof(text));
    std::string result{text};
    if (!first_person_preparation_error.empty()) {
        if (!result.empty()) result.push_back(' ');
        result += first_person_preparation_error;
    }
    if (!vein_mining_preparation_error.empty()) {
        if (!result.empty()) result.push_back(' ');
        result += vein_mining_preparation_error;
    }
    if (!gifting_range_preparation_error.empty()) {
        if (!result.empty()) result.push_back(' ');
        result += gifting_range_preparation_error;
    }
    return result;
}
}
