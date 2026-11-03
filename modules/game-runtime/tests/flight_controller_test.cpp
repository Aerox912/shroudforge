// SPDX-License-Identifier: MIT
#include "abi/runtime_api.h"
#include "flight/client_controller.hpp"
#include "vein/ikea_client/flight_intent_abi.h"
#include <atomic>
#include <cassert>
namespace { std::atomic_bool provider{false},scalar{false},selected{false}; }
namespace VeinFlight { bool provider_ready() noexcept { return provider; } }
namespace shroudforge::flight {
bool prepare_scalar() noexcept {return scalar;}
bool scalar_ready() noexcept {return scalar;}
void set_scalar_enabled(bool value) noexcept {selected=value;}
}
// Only the OS/profile/provider admission boundary is replaced. Controller,
// publisher thread, epoch/reset logic and exported owner checks are real.
extern "C" bool ESM_VEIN_FLIGHT_CALL ESM_VeinGetFlightResetSerialV1(uint64_t* value) {
    if(!provider)return false;*value=7;return true;
}
extern "C" bool ESM_VEIN_FLIGHT_CALL ESM_VeinPublishFlightIntentV2(const EsmVeinFlightIntentV2* value) {
    return provider&&value->observed_reset_serial==7;
}
int main() {
    assert(KfcRuntimeFlightClientSetEnabledV1(0,1)==-1);
    assert(KfcRuntimeFlightClientSetEnabledV1(17,2)==-1);
    assert(KfcRuntimeFlightClientSetEnabledV1(17,1)==0);
    assert(!selected);
    provider=true;
    assert(KfcRuntimeFlightClientSetEnabledV1(17,1)==0); // missing scalar
    assert(!selected);
    scalar=true;
    assert(KfcRuntimeFlightClientSetEnabledV1(17,1)==1);
    assert(selected);
    assert(KfcRuntimeFlightClientSetEnabledV1(18,0)==-1);
    assert(KfcRuntimeFlightClientResetV1(18)==-1);
    assert(selected);
    KfcFlightClientStatusV1 status{32,1,0,0,0,0,0};
    assert(KfcRuntimeFlightClientStatusV1(17,&status)==1);
    assert(status.flags==7&&status.mode==1&&status.local_reset_serial==7);
    assert(KfcRuntimeFlightClientRetireV1(17)==1);
    assert(!selected);
    assert(KfcRuntimeFlightClientRetireV1(17)==1);
    assert(KfcRuntimeFlightClientSetEnabledV1(18,1)==1);
    assert(selected);
    assert(KfcRuntimeFlightClientRetireV1(17)==1);
    assert(selected); // stale retirement must not clear a newer owner
    assert(KfcRuntimeFlightClientSetEnabledV1(18,0)==1);
    assert(!selected);
    assert(KfcRuntimeFlightClientSetEnabledV1(18,1)==1);
    shroudforge::flight::stop_client();
    assert(!selected);
    assert(KfcRuntimeFlightClientRetireV1(18)==1);
}
