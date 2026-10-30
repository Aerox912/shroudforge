#pragma once

#include <string>

namespace EcsRuntime {
bool Initialize(bool overlay_demand = false);
void Tick();
std::string Status();
void Shutdown();
}
