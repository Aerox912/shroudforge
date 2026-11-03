#pragma once
#include "current_player_binding.h"
#include "original_world_interval.h"
#include "engine-hooks/held_entry_preparation.h"
namespace PlayerOwnership {
// Internal host producer only, prepared at the genuine held game entry. Ready
// proves this World wrapper, not a feature's scalar/mining hooks or gameplay.
bool PrepareCallbackPlayerWorld(const EngineHooks::HeldEntryToken&) noexcept;
bool CallbackPlayerWorldReady() noexcept;
bool CallbackPlayerFenced() noexcept;
std::uintptr_t CallbackPlayerImage() noexcept;
bool BeginCallbackWorld(std::uintptr_t actual_return_pc,std::uintptr_t scene_argument,std::uintptr_t world_argument) noexcept;
void EndCallbackWorld() noexcept;
// Acquire and release on this callback worker. The original World interval is
// process-shared; a lease never transfers stack/TLS authority from its producer.
OriginalWorldInterval::Lease TryReadCallbackWorld() noexcept;
// Short registry/session snapshot scope. Never hold this across original game
// calls. The caller retains its qualified World lease for the scope's lifetime.
class CallbackPlayerScope {
public:
    explicit CallbackPlayerScope(const OriginalWorldInterval::Lease&) noexcept;
    ~CallbackPlayerScope(){Release();}
    CallbackPlayerScope(const CallbackPlayerScope&)=delete;
    CallbackPlayerScope& operator=(const CallbackPlayerScope&)=delete;
    bool Validate() noexcept;
    bool Copy(CurrentSnapshot&) noexcept;
    bool Release() noexcept;
    std::uintptr_t Manager() const noexcept {return valid_?view_.manager:0;}
private:
    const OriginalWorldInterval::Lease& lease_;
    EngineContext::EcsView view_;
    std::uintptr_t registry_{};
    std::uint64_t epoch_{};
    bool valid_{};
};
// All registry/service/snapshot readers end before this helper returns. Only
// the caller's World lease and copied identity values survive into game calls.
bool CopyCallbackPlayers(const OriginalWorldInterval::Lease&,CurrentSnapshot&) noexcept;
namespace NativeDetail {
using OriginalWorldCall=std::uintptr_t(__fastcall*)(std::uintptr_t,std::uintptr_t,std::uintptr_t,std::uintptr_t);
bool PrepareCallbackWorldAtImage(const EngineHooks::HeldEntryToken&,std::uintptr_t) noexcept;
// Host-only completion with the actual RunHeldEntryPreparation result. Consume
// the pending publication once; never expose Ready inside the held callback.
bool CompleteCallbackPlayerWorldPreparation(bool coordinated) noexcept;
std::uintptr_t ForwardCallbackWorldCall(OriginalWorldCall,std::uintptr_t actual_return_pc,
    std::uintptr_t,std::uintptr_t,std::uintptr_t,std::uintptr_t);
}
}
// Retain the existing internal wrapper symbol; common ownership is independent
// of Flight's scalar hook and its CPU-state preparation.
extern "C" std::uintptr_t SF_FlightWorldCall(std::uintptr_t,std::uintptr_t,std::uintptr_t,std::uintptr_t);
