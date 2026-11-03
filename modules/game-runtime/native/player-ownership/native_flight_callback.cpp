#include "player-ownership/native_flight_callback.h"
#include "engine-context/engine_context_state.h"
namespace PlayerOwnership {
namespace {
class StackMemory final:public ActorQueryMemory {
    bool read(std::uintptr_t p,void* out,std::size_t size) noexcept override {
        return EngineContextImplementation::read_span(p,out,size);
    }
};
class Borrow final:public FlightCallbackBorrow {
public:
    Borrow() noexcept:lease_(TryReadCallbackWorld()),copy_(lease_){}
    ~Borrow(){Release();}
    bool ValidFrame(const ActorScalarFrame& frame) const noexcept {
        return copy_.Manager()!=0 && frame.actor.image==CallbackPlayerImage() && frame.actor.expected_manager==copy_.Manager();
    }
    std::uintptr_t Manager() const noexcept {return copy_.Manager();}
    ActorQueryMemory& Stack() noexcept override {return stack_;}
    bool Validate() noexcept override {return copy_.Validate();}
    bool Copy(CurrentSnapshot& output) noexcept override {return copy_.Copy(output);}
    bool Release() noexcept override {const bool released=copy_.Release();lease_.Release();return released && !CallbackPlayerFenced();}
private:
    OriginalWorldInterval::Lease lease_;
    CallbackPlayerScope copy_;
    StackMemory stack_;
};
}
NativeFlightCallback::NativeFlightCallback()=default;
NativeFlightCallback::~NativeFlightCallback()=default;
bool NativeFlightCallback::Prepare(){return CallbackPlayerWorldReady();}
bool NativeFlightCallback::BeginWorldCall(std::uintptr_t pc,std::uintptr_t scene,std::uintptr_t world) noexcept {
    return BeginCallbackWorld(pc,scene,world);
}
void NativeFlightCallback::EndWorldCall() noexcept {EndCallbackWorld();}
std::uint32_t NativeFlightCallback::Invoke(const ActorScalarFrame& frame,std::uint32_t original_bits,std::uint32_t vertical_bits) noexcept {
    if(!CallbackPlayerWorldReady())return original_bits;
    Borrow borrow;if(!borrow.ValidFrame(frame))return original_bits;
    return select_invocation_flight(borrow,intents_,frame,original_bits,vertical_bits);
}
std::uint32_t NativeFlightCallback::InvokeRaw(const ScalarCall& raw,std::uintptr_t low,std::uintptr_t high) noexcept {
    if(!CallbackPlayerWorldReady())return raw.original_bits;
    Borrow borrow;ActorScalarFrame frame{{CallbackPlayerImage(),raw.actor_rbp,low,high,borrow.Manager()},raw.native_rsp,raw.site};
    if(!borrow.ValidFrame(frame))return raw.original_bits;
    return select_invocation_flight(borrow,intents_,frame,raw.original_bits,raw.vertical_bits);
}
}
