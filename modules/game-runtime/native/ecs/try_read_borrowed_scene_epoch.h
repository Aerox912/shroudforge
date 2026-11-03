#pragma once
#include "engine-context/engine_owners.h"

namespace EcsRuntime {
// Internal observer for a caller that ALREADY owns a qualified original native
// World/Scene borrow. This does not acquire, extend or create that borrow.
// No allocation, native call or Scene/registry gate acquisition is performed.
// A busy host state mutex refuses immediately. Call again before publishing
// an effect, and require the same nonzero session and native borrow identity.
bool TryReadBorrowedSceneEpoch(const EngineContext::EcsView& current,
                               std::uint64_t& session) noexcept;
}
