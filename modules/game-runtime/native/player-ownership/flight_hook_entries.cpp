#include "player-ownership/flight_hook_entries.h"
#include <intrin.h>
namespace {
PlayerOwnership::NativeFlightCallback flight;
std::atomic<bool> entries_ready{};
}
namespace PlayerOwnership {
NativeFlightCallback& NativeFlightService() noexcept {return flight;}
bool PrepareFlightEntries() noexcept {
    if(entries_ready.load())return true;
    try {
        if(!flight.Prepare() || !PrepareScalarCpuState())return false;
        const auto image=CallbackPlayerImage();
        if(!image)return false;
        SF_FlightScalarAddress=image+0xbd15c4;
        entries_ready.store(true);
        return true;
    }catch(...){return false;}
}
}
extern "C" std::uint32_t SF_FlightSelectScalar(const PlayerOwnership::ScalarCall* raw) noexcept {
    // Internal assembly producer; never callable via Lua aggregate fields.
    if(!entries_ready.load())return raw->original_bits;
    const auto* tib=reinterpret_cast<const NT_TIB*>(NtCurrentTeb());
    return flight.InvokeRaw(*raw,reinterpret_cast<std::uintptr_t>(tib->StackLimit),
        reinterpret_cast<std::uintptr_t>(tib->StackBase));
}
