#pragma once

#include <string>
#include "../../game-runtime/native/host/selected_startup.h"
namespace EngineHooks {struct HeldEntryProof;}

namespace EcsRuntime {
bool Initialize(bool overlay_demand = false, bool require_startup_ready = false,
    const EngineHooks::HeldEntryProof* proof = nullptr, bool first_person = false);
bool PrepareSelected(const SelectedStartup::PlanV2&, const EngineHooks::HeldEntryProof&,
    const wchar_t* admitted_vein_ini, unsigned path_units, unsigned& completed);
bool ActivateSelected(const SelectedStartup::PlanV2&, unsigned& completed);
void Tick();
std::string Status();
void Shutdown();
}
