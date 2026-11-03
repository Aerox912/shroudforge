#include "../startup_gate.h"

#include <atomic>

namespace {
using PreparePlayerOwnership = bool (__cdecl*)(
    const EngineHooks::HeldEntryProof*);

HANDLE late_attempt{};
HANDLE late_finished{};
std::atomic<bool> prepared{};
std::atomic<bool> late_refused{};

DWORD WINAPI Prepare(void*) {
    if (WaitForSingleObject(StartupGate::entered, 5000) != WAIT_OBJECT_0) {
        StartupGate::Complete(false);
        return 1;
    }

    wchar_t provider_path[32768]{};
    const auto path_length = GetEnvironmentVariableW(
        L"SF_PLAYER_OWNERSHIP_PROVIDER", provider_path,
        static_cast<DWORD>(sizeof(provider_path) / sizeof(provider_path[0])));
    if (!path_length || path_length >= sizeof(provider_path) / sizeof(provider_path[0])) {
        StartupGate::Complete(false);
        return 1;
    }

    const auto provider = LoadLibraryW(provider_path);
    const auto prepare = provider ? reinterpret_cast<PreparePlayerOwnership>(
        GetProcAddress(provider, "KfcRuntimePreparePlayerOwnership")) : nullptr;
    if (!prepare) {
        StartupGate::Complete(false);
        return 1;
    }

    const auto proof = StartupGate::CaptureHeldEntryProof();
    if (!proof.is_held || !proof.is_held(proof.parked_thread_id)) {
        StartupGate::Complete(false);
        return 1;
    }

    auto invalid_proof = proof;
    invalid_proof.parked_thread_id = GetCurrentThreadId();
    if (prepare(&invalid_proof)) {
        StartupGate::Complete(false);
        return 1;
    }

    // The reviewed policy is enabled, but this non-game fixture has no prepared
    // exact-server profile. A genuine held proof must leave the optional adapter off.
    if (prepare(&proof)) {
        StartupGate::Complete(false);
        return 1;
    }

    prepared.store(true, std::memory_order_release);
    StartupGate::Complete(true);
    if (WaitForSingleObject(late_attempt, 5000) != WAIT_OBJECT_0) return 1;

    const auto late_result = prepare(&proof);
    late_refused.store(!late_result, std::memory_order_release);
    SetEvent(late_finished);
    return late_refused.load(std::memory_order_acquire) ? 0 : 1;
}
}

extern "C" __declspec(dllexport) bool GateVerified() {
    if (!prepared.load(std::memory_order_acquire) || !StartupGate::completed ||
        WaitForSingleObject(StartupGate::completed, 0) != WAIT_OBJECT_0 ||
        !late_attempt || !late_finished) return false;
    SetEvent(late_attempt);
    if (WaitForSingleObject(late_finished, 5000) != WAIT_OBJECT_0) return false;
    return late_refused.load(std::memory_order_acquire);
}

BOOL WINAPI DllMain(HINSTANCE, DWORD reason, LPVOID) {
    if (reason != DLL_PROCESS_ATTACH) return TRUE;
    late_attempt = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    late_finished = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!late_attempt || !late_finished || !StartupGate::Install()) return FALSE;
    const auto worker = CreateThread(nullptr, 0, Prepare, nullptr, 0, nullptr);
    if (!worker) return FALSE;
    CloseHandle(worker);
    return TRUE;
}
