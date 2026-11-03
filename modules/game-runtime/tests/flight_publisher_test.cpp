// SPDX-License-Identifier: MIT
#include "flight/flight_publisher.hpp"
#include <atomic>
#include <cassert>
namespace {
std::atomic_bool available{false};
std::atomic_bool admit_on{true};
std::atomic_uint64_t serial{7};
std::atomic_uint32_t last_on{0};
std::atomic_uint32_t resets{0};
void reset(void*) noexcept { ++resets; }
}
// Deterministic native-provider boundary: no game hooks or server are touched.
extern "C" bool ESM_VEIN_FLIGHT_CALL ESM_VeinGetFlightResetSerialV1(uint64_t* out) {
    if (!available) return false;
    *out=serial;
    return true;
}
extern "C" bool ESM_VEIN_FLIGHT_CALL ESM_VeinPublishFlightIntentV2(const EsmVeinFlightIntentV2* intent) {
    assert(intent->struct_bytes==32 && intent->abi_version==2 && !intent->reserved0);
    if (!available || intent->observed_reset_serial!=serial || (intent->requested_on&&!admit_on)) return false;
    last_on=intent->requested_on;
    return true;
}
int main() {
    shroudweave::flight::FlightPublisher publisher;
    assert(!publisher.PrepareEnable()); // Catches former optional-provider fallback.
    assert(publisher.Start(reset,nullptr));
    assert(!publisher.PrepareEnable());
    publisher.SetEnabled(true);
    assert(!publisher.Requested());
    available=true;
    // Binding after a failed On first resets, so it requires another press.
    (void)publisher.PrepareEnable();
    assert(publisher.PrepareEnable());
    publisher.SetEnabled(true);
    assert(publisher.Requested());
    assert(last_on==1);
    publisher.SetEnabled(false);
    admit_on=false;
    assert(publisher.PrepareEnable());
    publisher.SetEnabled(true);
    assert(!publisher.Requested()); // missing fresh input cannot be shown as On
    admit_on=true;
    assert(publisher.PrepareEnable());
    publisher.SetEnabled(true);
    assert(publisher.Requested());
    serial=8;
    assert(!publisher.PrepareEnable());
    assert(!publisher.Requested());
    assert(resets>0);
    assert(last_on==0);
    assert(publisher.PrepareEnable());
    publisher.SetEnabled(true);
    assert(publisher.Requested());
    publisher.EmergencyOff();
    assert(!publisher.Requested());
    assert(!publisher.PrepareEnable());
    publisher.SetEnabled(true);
    assert(!publisher.Requested());
    publisher.Stop();
    assert(!publisher.Requested());
    assert(last_on==0);
}
