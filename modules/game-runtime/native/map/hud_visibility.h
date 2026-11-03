#pragma once
#include <cstdint>
#include <cstddef>
namespace MapRuntime {
bool PrepareHudVisibility(std::uintptr_t image,std::size_t image_size);
bool EnableHudVisibility(std::uintptr_t image,std::size_t image_size);
void DisableHudVisibility();
bool HudInputAllowed(std::uint64_t now);
bool HudGameplay(std::uint64_t now);
bool HudKnown(std::uint64_t now);
}
