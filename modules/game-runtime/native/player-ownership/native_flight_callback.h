#pragma once
#include "flight_callback.h"
#include "callback_player_scope.h"
#include "flight_hook_thunk.h"

namespace PlayerOwnership {
// Host-owned exact-server adapter. Prepare once before publishing hooks. The
// object and prepared state must outlive every hook and World/callback reader.
// No hook is installed or engine code invoked by this class.
class NativeFlightCallback {
public:
    NativeFlightCallback();
    ~NativeFlightCallback();
    NativeFlightCallback(const NativeFlightCallback&)=delete;
    NativeFlightCallback& operator=(const NativeFlightCallback&)=delete;
    bool Prepare();
    // Only the exact native CALL wrapper may supply its actual return address
    // and register args here. A caller-supplied integer is not admission proof.
    bool BeginWorldCall(std::uintptr_t actual_return_pc,std::uintptr_t scene_argument,
                        std::uintptr_t world_argument) noexcept;
    void EndWorldCall() noexcept;
    std::uint32_t Invoke(const ActorScalarFrame&,std::uint32_t original_bits,
                         std::uint32_t native_vertical_bits) noexcept;
    // The scalar thunk supplies raw machine state. Construct the expected
    // manager inside this same World lease, with no gap between separate reads.
    std::uint32_t InvokeRaw(const ScalarCall&,std::uintptr_t stack_low,std::uintptr_t stack_high) noexcept;
    FlightIntents& Intents() noexcept {return intents_;}
    bool Fenced() const noexcept {return CallbackPlayerFenced();}
private:
    FlightIntents intents_;
};
}
