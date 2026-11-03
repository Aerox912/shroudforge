#pragma once
#include "engine-hooks/held_entry_preparation.h"
#include <cstdint>

namespace PlayerOwnership::NativeDetail {
// Private dependency seam, not an exported API or runtime image override.
// The production entry supplies its actual executable base. Owned fixtures
// supply an inert mapping; policy, token, profile and publication checks remain
// inside this function, including the final Unprepared -> Ready transition.
bool PrepareServiceRetirementAtImage(const EngineHooks::HeldEntryToken&,
                                    std::uintptr_t image) noexcept;
}
