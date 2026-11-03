#pragma once
#include "semantic-attribution/owned_ledger.h"
#include "engine-hooks/write_executable_bytes.h"
#include <cstddef>

namespace SemanticAttribution::Native {
enum class PreparationStatus : unsigned {
    Rejected,Published,PartialPublished
};
enum class PreparationFailure : unsigned {
    None,Prerequisite,Profile,CpuState,Capacity,Allocation,Relay,Unwind,Protection,Reservation,Publication
};
struct PreparationResult final {
    PreparationStatus status{PreparationStatus::Rejected};
    PreparationFailure failure{PreparationFailure::Prerequisite};
    std::size_t published_spans{};
};
// Private explicit preparation entry. Current startup does not call it.
// The genuine token/owner must already have been issued by the existing host.
// Every potentially reachable allocation, function table, adapter and provider
// remains process-retained after full OR partial publication. No teardown API.
PreparationResult PrepareAttribution(const EngineHooks::HeldEntryToken&,
    const EngineHooks::RetainedModuleOwnerToken&,Limits) noexcept;
}
