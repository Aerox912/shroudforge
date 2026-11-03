#pragma once

#include "../../game-runtime/native/engine-hooks/held_entry_preparation.h"
#include "../../game-runtime/native/early-probe/host_api.h"

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>

namespace EcsRuntime {
EarlyOwnerProbe::BootstrapHostV1 EarlyProbeBootstrapHost(std::uintptr_t runtime_image,
    const EngineHooks::HeldEntryProof& proof);
bool Initialize(bool overlay_demand = false,
                const EngineHooks::HeldEntryProof* held_entry_proof = nullptr,
                bool first_person_demand = false,
                std::optional<unsigned> vein_mining_demand = std::nullopt,
                std::optional<unsigned> gifting_range_demand = std::nullopt);
void Tick();
std::string Status();
std::string VeinMiningPreparationError();
std::string GiftingRangePreparationError();
void Shutdown();
bool ReplaceHeldEntryBytes(const EngineHooks::HeldEntryProof& proof,
                          std::uintptr_t address, const void* expected,
                          const void* replacement, std::size_t size);
}
