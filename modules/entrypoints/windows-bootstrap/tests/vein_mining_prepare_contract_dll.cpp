#include "../../../game-runtime/native/abi/runtime_api.h"
#include "../../../game-runtime/native/engine-hooks/held_entry_preparation.h"

#include <windows.h>

#include <atomic>
#include <iterator>

#ifndef PLAYER_OWNERSHIP_EXPORT
#define PLAYER_OWNERSHIP_EXPORT 1
#endif

namespace {
std::atomic<unsigned> calls[4]{};
std::atomic<unsigned> call_count{};
std::atomic<unsigned> vein_demand{99};

void record(unsigned call) {
    const auto index = call_count.fetch_add(1, std::memory_order_relaxed);
    if (index < 4) calls[index].store(call, std::memory_order_relaxed);
}
}

extern "C" KFC_RUNTIME_API uint32_t KFC_RUNTIME_CALL KfcRuntimeAbi(void) {
    return KFC_RUNTIME_ABI_VERSION;
}

extern "C" KFC_RUNTIME_API bool KFC_RUNTIME_CALL KfcRuntimeInitialize(void) {
    record(1);
    return true;
}

extern "C" KFC_RUNTIME_API bool KFC_RUNTIME_CALL KfcRuntimePrepareOverlay(unsigned) {
    return true;
}

extern "C" KFC_RUNTIME_API void KFC_RUNTIME_CALL KfcRuntimeTick(void) {}
extern "C" KFC_RUNTIME_API void KFC_RUNTIME_CALL KfcRuntimeShutdown(void) {}

extern "C" KFC_RUNTIME_API void KFC_RUNTIME_CALL KfcRuntimeStatus(char* buffer, size_t capacity) {
    if (buffer && capacity) buffer[0] = '\0';
}

#if PLAYER_OWNERSHIP_EXPORT
extern "C" __declspec(dllexport) bool __cdecl KfcRuntimePreparePlayerOwnership(
    const EngineHooks::HeldEntryProof* proof) {
    record(2);
    if (!proof) return false;
    wchar_t value[8]{};
    return GetEnvironmentVariableW(L"SF_PLAYER_OWNERSHIP_TEST_REJECT", value,
        static_cast<DWORD>(std::size(value))) == 0;
}
#endif

#if VEIN_PREPARE_EXPORT
extern "C" __declspec(dllexport) bool __cdecl KfcRuntimePrepareVeinMining(
    const EngineHooks::HeldEntryProof* proof, unsigned demand) {
    record(3);
    vein_demand.store(demand, std::memory_order_relaxed);
    if (!proof || demand > 1u) return false;
    wchar_t value[8]{};
    return GetEnvironmentVariableW(L"SF_VEIN_TEST_REJECT", value,
        static_cast<DWORD>(std::size(value))) == 0;
}
#endif

extern "C" __declspec(dllexport) unsigned __cdecl KfcRuntimeTestCallCount(void) {
    return call_count.load(std::memory_order_relaxed);
}

extern "C" __declspec(dllexport) unsigned __cdecl KfcRuntimeTestCallAt(unsigned index) {
    return index < 4 ? calls[index].load(std::memory_order_relaxed) : 0;
}

extern "C" __declspec(dllexport) unsigned __cdecl KfcRuntimeTestVeinDemand(void) {
    return vein_demand.load(std::memory_order_relaxed);
}
