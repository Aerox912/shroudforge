#include "../startup_gate.h"
#include <atomic>
#include <cstring>
#include <iterator>

namespace {
using ReplaceHeldEntryBytes = bool (__cdecl*)(
    const EngineHooks::HeldEntryProof*, std::uintptr_t,
    const void*, const void*, std::size_t);

HANDLE late_attempt{};
HANDLE late_finished{};
unsigned char* target{};
std::atomic<bool> prepared{};
std::atomic<bool> late_refused{};
constexpr unsigned char Original[4]{0x49, 0x83, 0xf8, 0x04};
constexpr unsigned char Replacement[4]{0x49, 0x83, 0xf8, 0x05};

DWORD WINAPI Prepare(void*) {
    if (WaitForSingleObject(StartupGate::entered, 5000) != WAIT_OBJECT_0) {
        StartupGate::Complete(false);
        return 1;
    }

    wchar_t provider_path[32768]{};
    const auto path_length = GetEnvironmentVariableW(
        L"SF_HELD_ENTRY_PROVIDER", provider_path,
        static_cast<DWORD>(std::size(provider_path)));
    if (!path_length || path_length >= std::size(provider_path)) {
        StartupGate::Complete(false);
        return 1;
    }
    const auto provider = LoadLibraryW(provider_path);
    const auto replace = provider ? reinterpret_cast<ReplaceHeldEntryBytes>(
        GetProcAddress(provider, "KfcRuntimeReplaceHeldEntryBytes")) : nullptr;
    if (!replace) {
        StartupGate::Complete(false);
        return 1;
    }

    const auto proof = StartupGate::CaptureHeldEntryProof();
    if (!proof.is_held || !proof.is_held(proof.parked_thread_id)) {
        StartupGate::Complete(false);
        return 1;
    }

    SYSTEM_INFO system{};
    GetSystemInfo(&system);
    target = static_cast<unsigned char*>(VirtualAlloc(
        nullptr, system.dwPageSize, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE));
    if (!target) {
        StartupGate::Complete(false);
        return 1;
    }
    std::memcpy(target, Original, sizeof(Original));
    DWORD previous_protection{};
    if (!VirtualProtect(target, system.dwPageSize, PAGE_EXECUTE_READ, &previous_protection)) {
        StartupGate::Complete(false);
        return 1;
    }

    const auto address = reinterpret_cast<std::uintptr_t>(target);
    auto invalid_proof = proof;
    invalid_proof.parked_thread_id = GetCurrentThreadId();
    if (replace(&invalid_proof, address, Original, Replacement, sizeof(Original)) ||
        std::memcmp(target, Original, sizeof(Original)) != 0) {
        StartupGate::Complete(false);
        return 1;
    }

    if (!replace(&proof, address, Original, Replacement, sizeof(Original)) ||
        std::memcmp(target, Replacement, sizeof(Replacement)) != 0) {
        StartupGate::Complete(false);
        return 1;
    }
    if (replace(&proof, address, Original, Replacement, sizeof(Original)) ||
        std::memcmp(target, Replacement, sizeof(Replacement)) != 0) {
        StartupGate::Complete(false);
        return 1;
    }
    if (!replace(&proof, address, Replacement, Original, sizeof(Replacement)) ||
        std::memcmp(target, Original, sizeof(Original)) != 0 ||
        !StartupGate::IsHeld(proof.parked_thread_id)) {
        StartupGate::Complete(false);
        return 1;
    }

    prepared.store(true, std::memory_order_release);
    StartupGate::Complete(true);
    if (WaitForSingleObject(late_attempt, 5000) != WAIT_OBJECT_0) return 1;

    const bool late_write = replace(
        &proof, address, Original, Replacement, sizeof(Original));
    late_refused.store(
        !late_write && std::memcmp(target, Original, sizeof(Original)) == 0,
        std::memory_order_release);
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
    return late_refused.load(std::memory_order_acquire) &&
        std::memcmp(target, Original, sizeof(Original)) == 0;
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
