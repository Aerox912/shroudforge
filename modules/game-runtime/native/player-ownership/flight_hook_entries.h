#pragma once
#include "native_flight_callback.h"
namespace PlayerOwnership {
NativeFlightCallback& NativeFlightService() noexcept;
// Scalar-only preparation under the Flight setup writer. The independently
// prepared common World producer must already be ready; CPU failure cannot
// remove its process-resident wrapper.
bool PrepareFlightEntries() noexcept;
}
extern "C" std::uintptr_t SF_FlightWorldCall(std::uintptr_t,std::uintptr_t,std::uintptr_t,std::uintptr_t);
