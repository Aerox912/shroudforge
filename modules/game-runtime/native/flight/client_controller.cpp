// SPDX-License-Identifier: MIT
#include "client_controller.hpp"
#include "client_scalar_bridge.hpp"
#include "flight_publisher.hpp"
#include "owner_state.hpp"
#include "abi/runtime_api.h"
#include "vein_flight/provider_adapter.hpp"
#include <atomic>
#include <mutex>

namespace {
struct Controller {
    std::mutex gate;
    shroudforge::flight::OwnerState owner;
    shroudweave::flight::FlightPublisher publisher;
    std::atomic_bool reset_pending{false};
    bool started=false;
};
Controller& controller() { static auto* value=new Controller; return *value; }
void on_reset(void* context) noexcept {
    auto& c=*static_cast<Controller*>(context);
    c.reset_pending.store(true,std::memory_order_release);
    shroudforge::flight::set_scalar_enabled(false);
}
void reconcile(Controller& c) noexcept {
    if(c.reset_pending.exchange(false,std::memory_order_acq_rel) ||
       (c.owner.requested()&&!c.publisher.Requested())) {
        c.owner.provider_reset();
        shroudforge::flight::set_scalar_enabled(false);
    }
}
void off(Controller& c) noexcept {
    shroudforge::flight::set_scalar_enabled(false);
    c.publisher.SetEnabled(false);
    c.owner.provider_reset();
}
}
extern "C" int32_t KFC_RUNTIME_CALL KfcRuntimeFlightClientSetEnabledV1(uint64_t token,uint32_t enabled) {
    if(!token||enabled>1)return -1;
    Controller* current=nullptr;
    try {
        auto& c=controller();current=&c;std::lock_guard lock(c.gate);
        if(!c.owner.claim(token))return -1;
        reconcile(c);
        if(!enabled){off(c);return 1;}
        // No silent local-only path if shared native admission is unavailable.
        if(!VeinFlight::provider_ready()||!shroudforge::flight::prepare_scalar()){off(c);return 0;}
        if(!c.started){c.started=c.publisher.Start(on_reset,&c);if(!c.started){off(c);return 0;}}
        if(!c.publisher.PrepareEnable()){off(c);return 0;}
        c.publisher.SetEnabled(true);
        if(!c.publisher.Requested()){off(c);return 0;}
        c.owner.set(token,true,true);
        shroudforge::flight::set_scalar_enabled(true);
        // A concurrent native reset disables selection via the callback and is
        // reconciled before status. This is local intent, never a server ACK.
        if(c.reset_pending.load(std::memory_order_acquire)){off(c);return 0;}
        return 1;
    }catch(...){if(current)current->publisher.EmergencyOff();shroudforge::flight::set_scalar_enabled(false);return 0;}
}
extern "C" int32_t KFC_RUNTIME_CALL KfcRuntimeFlightClientResetV1(uint64_t token) {
    if(!token)return -1;
    Controller* current=nullptr;
    try {auto& c=controller();current=&c;std::lock_guard lock(c.gate);
        if(!c.owner.owns(token))return -1;
        off(c);return 1;
    }catch(...){if(current)current->publisher.EmergencyOff();shroudforge::flight::set_scalar_enabled(false);return 0;}
}
extern "C" int32_t KFC_RUNTIME_CALL KfcRuntimeFlightClientRetireV1(uint64_t token) {
    if(!token)return -1;
    Controller* current=nullptr;
    try {auto& c=controller();current=&c;std::lock_guard lock(c.gate);
        // Idempotent stale retirement must never clear a newer owner's intent.
        if(!c.owner.owns(token))return 1;
        off(c);c.publisher.Stop();c.started=false;
        c.owner.retire(token);return 1;
    }catch(...){if(current)current->publisher.EmergencyOff();shroudforge::flight::set_scalar_enabled(false);return 0;}
}
extern "C" int32_t KFC_RUNTIME_CALL KfcRuntimeFlightClientStatusV1(uint64_t token,KfcFlightClientStatusV1* status) {
    if(!token||!status||status->struct_bytes!=sizeof(*status)||status->version!=1||status->reserved0||status->reserved1)return -1;
    try {auto& c=controller();std::lock_guard lock(c.gate);reconcile(c);
        if(!c.owner.owns(token))return 0;
        uint64_t serial=0;const bool provider=ESM_VeinGetFlightResetSerialV1(&serial);
        *status={sizeof(*status),1,(shroudforge::flight::scalar_ready()?1u:0u)|(provider?2u:0u)|(c.owner.requested()?4u:0u),provider?1u:0u,serial,0,0};
        return 1;
    }catch(...){return 0;}
}
namespace shroudforge::flight {
void stop_client() noexcept {
    Controller* current=nullptr;
    try {auto& c=controller();current=&c;std::lock_guard lock(c.gate);off(c);c.publisher.Stop();c.started=false;}
    catch(...){if(current)current->publisher.EmergencyOff();set_scalar_enabled(false);}
}
}
