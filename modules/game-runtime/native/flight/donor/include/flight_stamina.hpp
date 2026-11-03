#pragma once
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include "flight_hook_state.hpp"
#include <cstddef>
#include <cstdint>

namespace shroudweave::flight {
inline constexpr std::uint32_t kStaminaSiteRva = 0x2EBD7E;
inline constexpr std::uint32_t kStaminaContinuationRva = 0x2EBD86;
HookLifetime& ProcessStaminaHookLifetime() noexcept;
// Called only by the validated native glider-depletion callback. The query and
// Actor must be the same current row; no pointer is retained after this call.
bool ShouldSuppressGliderDrain(std::uintptr_t query, std::uintptr_t actor) noexcept;
bool StaminaProfileMatches(const std::uint8_t* image, std::size_t size,
                           const std::uint8_t* installed_site = nullptr) noexcept;
}
extern "C" {
bool flight_should_suppress_stamina(std::uintptr_t query, std::uintptr_t actor) noexcept;
extern __declspec(align(8)) PVOID volatile flight_stamina_continue_address;
void flight_stamina_detour_thunk();
}
