#pragma once

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include <atomic>
#include <cstdint>
#include <functional>

namespace shroudweave::flight {

inline constexpr LONG kVanillaPitchBits = static_cast<LONG>(0x3DB2B8C2);
inline constexpr LONG kEnabledPitchBits = static_cast<LONG>(0xBFC8F5C3);
static_assert(sizeof(LONG) == sizeof(std::uint32_t));

enum class HookLifetimeState : std::uint8_t {
    not_attempted,
    installing,
    resident,
    failed,
};

class HookLifetime final {
public:
    bool BeginAttempt() noexcept;
    bool FinishAttempt(bool service_succeeded, void* trampoline) noexcept;
    HookLifetimeState state() const noexcept;
    bool resident() const noexcept;
    void* trampoline() const noexcept;

private:
    std::atomic<HookLifetimeState> state_{HookLifetimeState::not_attempted};
    std::atomic<void*> trampoline_{};
};

using HostCreateDetour = std::function<bool(LPVOID, LPVOID, void**)>;

// The host callback creates and enables the hook in one operation. Establish
// its data first, and never retry or tear down an uncertain or resident hook.
bool InstallResidentDetourOnce(
    HookLifetime& lifetime,
    const HostCreateDetour& create_detour,
    LPVOID target,
    LPVOID detour,
    volatile LONG* pitch_value,
    PVOID volatile* continuation_slot,
    PVOID continuation,
    void** trampoline_out = nullptr) noexcept;

void SetFlightPitchEnabled(bool enabled) noexcept;
HookLifetime& ProcessHookLifetime() noexcept;

} // namespace shroudweave::flight

extern "C" {
extern __declspec(align(4)) volatile LONG flight_pitch_bits;
extern __declspec(align(8)) PVOID volatile flight_continue_address;
void flight_detour_thunk();
}
