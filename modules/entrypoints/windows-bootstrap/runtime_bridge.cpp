#include "runtime_bridge.h"
#include "startup_admission.h"
#include "../../game-runtime/native/engine-hooks/held_entry_preparation.h"
#include "../../game-runtime/native/abi/runtime_api.h"
#include <windows.h>
#include <filesystem>
#include <utility>

namespace {
HMODULE provider{};
using AbiFunction = decltype(&KfcRuntimeAbi);
using InitializeFunction = decltype(&KfcRuntimeInitialize);
using VoidFunction = decltype(&KfcRuntimeTick);
using StatusFunction = decltype(&KfcRuntimeStatus);
VoidFunction tick{}, shutdown{};
StatusFunction status{};
std::string error{"provider-not-loaded"};
using ActivateSelectedFunction = bool (__cdecl*)(const SelectedStartup::PlanV2*, unsigned*);
ActivateSelectedFunction activate_selected{};
bool load_provider() {
    if (provider) { error = "native-provider-already-loaded"; return false; }
    HMODULE self{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        reinterpret_cast<LPCWSTR>(&load_provider), &self)) return false;
    wchar_t path[32768]{};
    if (!GetModuleFileNameW(self, path, 32768)) return false;
    const auto dll = std::filesystem::path(path).parent_path() / L"shroudforge" / L"kfc-runtime.dll";
    provider = LoadLibraryExW(dll.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!provider) { error = "kfc-runtime.dll-load-failed"; return false; }
    const auto abi = reinterpret_cast<AbiFunction>(GetProcAddress(provider, "KfcRuntimeAbi"));
    tick = reinterpret_cast<VoidFunction>(GetProcAddress(provider, "KfcRuntimeTick"));
    shutdown = reinterpret_cast<VoidFunction>(GetProcAddress(provider, "KfcRuntimeShutdown"));
    status = reinterpret_cast<StatusFunction>(GetProcAddress(provider, "KfcRuntimeStatus"));
    const auto actual_abi = abi ? abi() : 0;
    if (!abi || actual_abi != KFC_RUNTIME_ABI_VERSION || !tick || !shutdown || !status) {
        error = abi
            ? "incompatible-runtime-provider-ABI:expected=" + std::to_string(KFC_RUNTIME_ABI_VERSION) +
                ",actual=" + std::to_string(actual_abi)
            : "missing-runtime-provider-ABI-export";
        tick = nullptr; shutdown = nullptr; status = nullptr;
        FreeLibrary(provider); provider = nullptr; return false;
    }
    error.clear();return true;
}
}
namespace EcsRuntime {
bool Initialize(bool overlay_demand, bool require_startup_ready,
        const EngineHooks::HeldEntryProof* proof, bool first_person) {
    if (!load_provider()) return false;
    const auto initialize = reinterpret_cast<InitializeFunction>(GetProcAddress(provider, "KfcRuntimeInitialize"));
    if (!initialize) { error = "missing-legacy-initialize"; return false; }
    const auto initialized = initialize();
    using HeldPreparation=bool (__cdecl*)(const EngineHooks::HeldEntryProof*,unsigned);
    const auto prepare_held=reinterpret_cast<HeldPreparation>(GetProcAddress(provider,"KfcRuntimePrepareHeldEntry"));
    if(first_person&&(!initialized||!prepare_held||!prepare_held(proof,1u))) {
        error="required-first-person-held-entry-preparation-unavailable";return false;
    }
    const auto startup_ready = reinterpret_cast<bool (__cdecl*)()>(GetProcAddress(provider,"KfcRuntimeStartupReady"));
    const auto prepare_overlay = reinterpret_cast<bool (__cdecl*)(unsigned)>(GetProcAddress(provider, "KfcRuntimePrepareOverlay"));
    bool graphics_ready=false;
    if (prepare_overlay) graphics_ready=prepare_overlay(overlay_demand ? 1u : 0u);
    else if (overlay_demand) OutputDebugStringA("[ESM overlay] provider lacks early broker; restart with matching native provider\n");
    if(require_startup_ready&&!StartupAdmission::RequiredReady(initialized,startup_ready!=nullptr,
            startup_ready&&startup_ready(),overlay_demand,graphics_ready)) {
        error="required-startup-hooks-or-graphics-unavailable";return false;
    }
    return initialized;
}
bool PrepareSelected(const SelectedStartup::PlanV2& plan, const EngineHooks::HeldEntryProof& proof,
        const wchar_t* ini, unsigned units, unsigned& completed) {
    completed = 0;
    if (!SelectedStartup::Valid(plan) || !load_provider()) return false;
    using Prepare = bool (__cdecl*)(const SelectedStartup::PlanV2*,const EngineHooks::HeldEntryProof*,const wchar_t*,unsigned,unsigned*);
    const auto prepare = reinterpret_cast<Prepare>(GetProcAddress(provider,"KfcRuntimePrepareSelectedV1"));
    const auto activate = reinterpret_cast<ActivateSelectedFunction>(GetProcAddress(provider,"KfcRuntimeActivateSelectedV1"));
    if (!prepare || !activate) { error = "required-selected-startup-exports-unavailable"; return false; }
    if (!prepare(&plan,&proof,ini,units,&completed) || completed != SelectedStartup::PreparedServices(plan)) {
        error = "required-selected-native-preparation-failed"; return false;
    }
    activate_selected=activate; // Publish only the successful exact prepared operation.
    return true;
}
bool ActivateSelected(const SelectedStartup::PlanV2& plan, unsigned& completed) {
    completed = 0;
    const auto activate=std::exchange(activate_selected,nullptr); // Consume even a rejected activation.
    if (!SelectedStartup::Valid(plan) || !activate || !activate(&plan,&completed) || completed != plan.services) {
        error = "required-selected-native-activation-failed"; return false;
    }
    return true;
}
void Tick() { if (tick) tick(); }
void Shutdown() { activate_selected=nullptr; if (shutdown) shutdown(); }
std::string Status() {
    if (!status) return error;
    char text[2048]{}; status(text, sizeof(text)); return error.empty()?std::string(text):error+"; "+text;
}
}
