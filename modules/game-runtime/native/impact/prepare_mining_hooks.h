#pragma once
#include "engine-hooks/held_entry_preparation.h"
#include <cstdint>
namespace ImpactRuntime::MiningHost {
// Private bootstrap coordinator. demand0 is a no-op; demand1 requires the real
// held entry and accepted dedicated-server/B profile. Never calls game code.
bool PrepareVeinMiningHooks(const EngineHooks::HeldEntryProof*,unsigned demand) noexcept;
namespace NativeDetail {
// Shared by the real coordinator and inert mapped-PE tests. Does not open.
bool PrepareMiningHooksAtImage(const EngineHooks::HeldEntryToken&,std::uintptr_t) noexcept;
bool ValidateMiningImage(std::uintptr_t image) noexcept;
}
}
