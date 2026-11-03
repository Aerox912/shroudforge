#pragma once
#include <windows.h>
#include <cstring>
#include <atomic>
#include "../../game-runtime/native/engine-hooks/held_entry_preparation.h"

// Installed by the startup proxy before the executable entrypoint runs. The
// wait happens at that entrypoint, outside DllMain and the Windows loader lock,
// so the preparation worker can load DLLs and use parser worker threads.
namespace StartupGate {
inline HANDLE entered{};
inline HANDLE completed{};
inline unsigned char* entrypoint{};
inline unsigned char original[14]{};
inline volatile LONG succeeded{};
inline std::atomic<DWORD> parked_thread_id{};
// Set only from this module's actual process-attach callback, before the worker.
inline bool direct_static_attachment{};

inline void Complete(bool success) {
    InterlockedExchange(&succeeded, success ? 1 : 0);
    SetEvent(completed);
}

inline int __cdecl Run();
using PostRestore = bool (__cdecl*)(void*,unsigned char*,const unsigned char*) noexcept;
inline PostRestore post_restore{};
inline void* post_restore_context{};
inline HANDLE post_restore_completed{};
// 0 absent, 5 reserving, 1 armed, 2 executing, 3 success, 4 failed.
inline std::atomic<unsigned> post_restore_state{};
inline std::atomic<bool> run_started{};
inline bool ExactJump() noexcept {
    if (!entrypoint) return false;
    unsigned char jump[14]{0xff,0x25,0,0,0,0};
    auto destination=&Run;
    std::memcpy(jump+6,&destination,sizeof(destination));
    return std::memcmp(entrypoint,jump,sizeof(jump))==0;
}
// The entrypoint thread alone restores and performs the one-shot continuation.
// No callback runs under the held-entry assertion: it is intentionally false
// once completed is signaled and the original bytes have been restored.
inline bool RestoreAndContinue() noexcept {
    if (!ExactJump()) return false;
    DWORD protection{};
    if (!VirtualProtect(entrypoint,sizeof(original),PAGE_EXECUTE_READWRITE,&protection)) return false;
    std::memcpy(entrypoint,original,sizeof(original));
    DWORD ignored{};
    if (!VirtualProtect(entrypoint,sizeof(original),protection,&ignored) ||
        !FlushInstructionCache(GetCurrentProcess(),entrypoint,sizeof(original))) return false;
    unsigned expected=1;
    if (post_restore_state.compare_exchange_strong(expected,2,std::memory_order_acq_rel)) {
        const bool ok=post_restore && post_restore(post_restore_context,entrypoint,original);
        post_restore_state.store(ok?3u:4u,std::memory_order_release);
        SetEvent(post_restore_completed);
        return ok;
    }
    return expected==0; // Reserved/repeated/failed callbacks cannot enter EP.
}
inline int __cdecl Run() {
    if (run_started.exchange(true,std::memory_order_acq_rel)) ExitProcess(ERROR_DLL_INIT_FAILED);
    parked_thread_id.store(GetCurrentThreadId(),std::memory_order_release);
    SetEvent(entered);
    if (WaitForSingleObject(completed,INFINITE)!=WAIT_OBJECT_0 ||
        InterlockedCompareExchange(&succeeded,0,0)!=1 || !RestoreAndContinue()) {
        ExitProcess(ERROR_DLL_INIT_FAILED);
    }
    return reinterpret_cast<int (__cdecl*)()>(entrypoint)();
}

// Only the preparation worker can capture this private assertion. The observed
// thread is parked in Run, the completion event is unsignaled, and the actual
// PE entrypoint still contains this gate's exact jump. This does not authenticate
// arbitrary privileged native plugins or exclude external process injection.
inline bool __cdecl IsHeld(std::uint32_t thread) noexcept {
    if (!thread || thread == GetCurrentThreadId() ||
        parked_thread_id.load(std::memory_order_acquire) != thread || !entered || !completed || !entrypoint ||
        WaitForSingleObject(entered, 0) != WAIT_OBJECT_0 ||
        WaitForSingleObject(completed, 0) != WAIT_TIMEOUT) return false;
    unsigned char jump[14]{0xff, 0x25, 0, 0, 0, 0};
    auto destination = &Run;
    std::memcpy(jump + 6, &destination, sizeof(destination));
    return std::memcmp(entrypoint, jump, sizeof(jump)) == 0;
}
// Trusted worker arms retained data before releasing this sole gate.
inline bool ArmPostRestore(std::uint32_t thread,PostRestore callback,void* context) noexcept {
    if (!callback || !context || !IsHeld(thread)) return false;
    HANDLE event=CreateEventW(nullptr,TRUE,FALSE,nullptr);
    if (!event) return false;
    unsigned expected=0;
    if (!post_restore_state.compare_exchange_strong(expected,5,std::memory_order_acq_rel)) {CloseHandle(event);return false;}
    post_restore=callback;post_restore_context=context;post_restore_completed=event;
    post_restore_state.store(1,std::memory_order_release);
    return true;
}
inline bool AwaitPostRestore() noexcept {
    return post_restore_completed && WaitForSingleObject(post_restore_completed,INFINITE)==WAIT_OBJECT_0 &&
        post_restore_state.load(std::memory_order_acquire)==3;
}
inline EngineHooks::HeldEntryProof CaptureHeldEntryProof() noexcept {
    EngineHooks::HeldEntryProof proof;
    const auto thread = parked_thread_id.load(std::memory_order_acquire);
    if (IsHeld(thread)) { proof.parked_thread_id = thread; proof.is_held = &IsHeld; }
    return proof;
}

inline bool Install() {
    auto* image = reinterpret_cast<unsigned char*>(GetModuleHandleW(nullptr));
    if (!image) return false;
    const auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(image);
    if (dos->e_magic != IMAGE_DOS_SIGNATURE || dos->e_lfanew <= 0) return false;
    const auto* nt = reinterpret_cast<const IMAGE_NT_HEADERS64*>(image + dos->e_lfanew);
    if (nt->Signature != IMAGE_NT_SIGNATURE ||
        nt->FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 ||
        nt->OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC ||
        nt->OptionalHeader.SizeOfImage < sizeof(original) ||
        !nt->OptionalHeader.AddressOfEntryPoint ||
        nt->OptionalHeader.AddressOfEntryPoint > nt->OptionalHeader.SizeOfImage - sizeof(original))
        return false;
    entered = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    completed = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!entered || !completed) return false;
    entrypoint = image + nt->OptionalHeader.AddressOfEntryPoint;
    // RIP-relative indirect jump preserves registers. No instruction relocation
    // is needed: restore the entire entrypoint before executing any of its code.
    unsigned char jump[14]{0xff, 0x25, 0, 0, 0, 0};
    auto destination = &Run;
    static_assert(sizeof(destination) == 8);
    std::memcpy(jump + 6, &destination, sizeof(destination));
    DWORD protection{};
    if (!VirtualProtect(entrypoint, sizeof(jump), PAGE_EXECUTE_READWRITE, &protection)) return false;
    std::memcpy(original, entrypoint, sizeof(original));
    std::memcpy(entrypoint, jump, sizeof(jump));
    DWORD ignored{};
    if (!VirtualProtect(entrypoint, sizeof(jump), protection, &ignored) ||
        !FlushInstructionCache(GetCurrentProcess(), entrypoint, sizeof(jump)))
        ExitProcess(ERROR_DLL_INIT_FAILED); // Never unload a possibly published gate.
    return true;
}
}
