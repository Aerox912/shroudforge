// Compile the production entrypoint, replacing only thread creation for the
// deterministic post-publication failure fixture. No game/runtime is loaded.
#include "../bootstrap_state.h"
#include <atomic>

namespace {
std::atomic<unsigned> thread_requests{}, workers{};
bool held_verified{};
HANDLE TestCreateThread(LPSECURITY_ATTRIBUTES attributes, SIZE_T stack,
        LPTHREAD_START_ROUTINE start, LPVOID argument, DWORD flags, LPDWORD id) {
    ++thread_requests;
    wchar_t fail[2]{};
    if (GetEnvironmentVariableW(L"SF_ADMISSION_TEST_THREAD_FAIL", fail, 2)) return nullptr;
    return ::CreateThread(attributes, stack, start, argument, flags, id);
}
}
#define CreateThread TestCreateThread
#include "../startup.cpp"
#undef CreateThread

namespace BootstrapImplementation {
void log(char, const std::string&) {}
void show_startup_error(const std::filesystem::path&, const std::string&) {}
DWORD WINAPI run(void*) {
    ++workers;
    if (WaitForSingleObject(StartupGate::entered, 5000) != WAIT_OBJECT_0) {
        StartupGate::Complete(false); return 1;
    }
    const auto proof = StartupGate::CaptureHeldEntryProof();
    held_verified = proof.is_held && proof.is_held(proof.parked_thread_id) &&
        proof.parked_thread_id != GetCurrentThreadId();
    StartupGate::Complete(held_verified);
    return held_verified ? 0 : 1;
}
}

extern "C" __declspec(dllexport) unsigned FixtureState() {
    if (!thread_requests && !workers && !StartupGate::entrypoint) return 0;
    return thread_requests == 1 && workers == 1 && held_verified &&
        WaitForSingleObject(StartupGate::completed, 0) == WAIT_OBJECT_0 &&
        std::memcmp(StartupGate::entrypoint, StartupGate::original, 14) == 0 ? 1 : 2;
}
