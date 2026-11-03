// SPDX-License-Identifier: MIT
// Real provider translation unit, but no prepare success/hook/worker is invoked.
// Native reset input is an opaque owned fixture value, never dereferenced.
#include "vein_flight/provider_adapter.cpp"
#include <cassert>
int main() {
    auto& r=state();
    assert(!VeinFlight::provider_ready());
    VeinFlight::ProviderConfig config{};
    assert(!VeinFlight::prepare_provider(config));
    assert(r.attempted&&!r.ready);
    r.ready=true;r.flight_transport_enabled=true;r.vein_policy_allowed=true;
    assert(r.native_reset.open());
    const auto now=GetTickCount64();
    const auto serial=r.native_reset.observe_input(123,1,now);
    assert(serial);
    fault_vein_worker();
    assert(r.vein_fault&&!r.fault);
    assert(VeinFlight::provider_ready());
    uint64_t observed=0;
    assert(ESM_VeinGetFlightResetSerialV1(&observed)&&observed==serial);
    EsmVeinFlightIntentV2 intent{32,2,1,1,0,serial};
    assert(ESM_VeinPublishFlightIntentV2(&intent));
    assert(management_wait_timeout()==INFINITE);
    assert(!signal_worker()); // missing A6 event is local failure, not Flight
    assert(!r.fault&&VeinFlight::provider_ready());
    r.vein_policy_allowed=false;
    assert(signal_worker()); // Flight-only configuration needs no A6 event
    VeinFlight::stop_provider();
    assert(!VeinFlight::provider_ready());
    assert(!ESM_VeinGetFlightResetSerialV1(&observed));
    assert(!ESM_VeinPublishFlightIntentV2(&intent));
}
