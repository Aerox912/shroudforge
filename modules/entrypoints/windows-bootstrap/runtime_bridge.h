#pragma once

#include <string>
namespace EngineHooks {struct HeldEntryProof;}

namespace EcsRuntime {
bool Initialize(bool overlay_demand = false, bool require_startup_ready = false,
    const EngineHooks::HeldEntryProof* proof = nullptr, bool first_person = false);
void Tick();
std::string Status();
void Shutdown();
}
