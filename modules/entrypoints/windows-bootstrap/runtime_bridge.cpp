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
VoidFunction tick{}, shutdown{};
StatusFunction status{};
std::string error{"provider-not-loaded"};
}
namespace EcsRuntime {
bool Initialize(bool overlay_demand) {
    HMODULE self{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        reinterpret_cast<LPCWSTR>(&Initialize), &self)) return false;
    wchar_t path[32768]{};
    if (!GetModuleFileNameW(self, path, 32768)) return false;
    const auto dll = std::filesystem::path(path).parent_path() / L"shroudforge" / L"kfc-runtime.dll";
    provider = LoadLibraryExW(dll.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!provider) { error = "kfc-runtime.dll-load-failed"; return false; }
    const auto abi = reinterpret_cast<AbiFunction>(GetProcAddress(provider, "KfcRuntimeAbi"));
    const auto initialize = reinterpret_cast<InitializeFunction>(GetProcAddress(provider, "KfcRuntimeInitialize"));
    tick = reinterpret_cast<VoidFunction>(GetProcAddress(provider, "KfcRuntimeTick"));
    shutdown = reinterpret_cast<VoidFunction>(GetProcAddress(provider, "KfcRuntimeShutdown"));
    status = reinterpret_cast<StatusFunction>(GetProcAddress(provider, "KfcRuntimeStatus"));
    const auto actual_abi = abi ? abi() : 0;
    if (!abi || actual_abi != KFC_RUNTIME_ABI_VERSION || !initialize || !tick || !shutdown || !status) {
        error = abi
            ? "incompatible-runtime-provider-ABI:expected=" + std::to_string(KFC_RUNTIME_ABI_VERSION) +
                ",actual=" + std::to_string(actual_abi)
            : "missing-runtime-provider-ABI-export";
        tick = nullptr; shutdown = nullptr; status = nullptr;
        FreeLibrary(provider); provider = nullptr; return false;
    }
    const auto initialized = initialize();
    const auto prepare_overlay = reinterpret_cast<bool (__cdecl*)(unsigned)>(GetProcAddress(provider, "KfcRuntimePrepareOverlay"));
    if (prepare_overlay) prepare_overlay(overlay_demand ? 1u : 0u);
    else if (overlay_demand) OutputDebugStringA("[ESM overlay] provider lacks early broker; restart with matching native provider\n");
    return initialized;
}
void Tick() { if (tick) tick(); }
void Shutdown() { if (shutdown) shutdown(); }
std::string Status() {
    if (!status) return error;
    char text[2048]{}; status(text, sizeof(text)); return text;
}
}
