#include "player-ownership/service_lifetime.h"
#include "player-ownership/service_lifetime_admission.h"
#include <atomic>

namespace PlayerOwnership {
namespace {
SRWLOCK service_readers=SRWLOCK_INIT;
enum class Phase { Unprepared, Ready, Retired };
#if defined(SF_PLAYER_OWNERSHIP_OWNED_FIXTURE) && !defined(SF_PLAYER_OWNERSHIP_COLD_FIXTURE)
// Private fixtures supply owned service memory; no engine hook is installed.
std::atomic<Phase> phase{Phase::Ready};
#else
// Production and cold preparation fixtures must earn Ready through successful
// held-entry publication; the cold switch never enables production policy.
std::atomic<Phase> phase{Phase::Unprepared};
#endif
std::atomic<NativeDetail::OriginalSessionDestroy> original_destroy{};
}
bool ServiceLifetimeReady() noexcept {
    return service_lifetime_reviewed() && phase.load(std::memory_order_acquire)==Phase::Ready;
}
ServiceReadLease TryReadServiceLifetime() noexcept {
    if(!ServiceLifetimeReady() || !TryAcquireSRWLockShared(&service_readers))return ServiceReadLease(nullptr);
    if(!ServiceLifetimeReady()) {
        ReleaseSRWLockShared(&service_readers);return ServiceReadLease(nullptr);
    }
    return ServiceReadLease(&service_readers);
}
namespace NativeDetail {
void SetOriginalSessionDestroy(OriginalSessionDestroy original) noexcept {
    original_destroy.store(original,std::memory_order_release);
}
bool PublishServiceLifetimeReady() noexcept {
    if(!service_lifetime_reviewed() || !original_destroy.load(std::memory_order_acquire))return false;
    auto expected=Phase::Unprepared;
    return phase.compare_exchange_strong(expected,Phase::Ready,std::memory_order_acq_rel);
}
void __fastcall DestroySessionAfterReaderDrain(std::uintptr_t context) {
    const auto error=GetLastError();
    // Never reopen after any observed alternate retirement. A new native
    // service requires a separately qualified birth event or process restart.
    phase.store(Phase::Retired,std::memory_order_release);
    AcquireSRWLockExclusive(&service_readers);
    ReleaseSRWLockExclusive(&service_readers);
    // The preparation worker binds this before any possible code publication.
    // No host gate is held while original game code or a nested destructor runs.
    const auto original=original_destroy.load(std::memory_order_acquire);
    SetLastError(error);
    original(context);
}
}
}
