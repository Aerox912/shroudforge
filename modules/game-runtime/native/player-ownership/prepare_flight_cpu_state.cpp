#include "player-ownership/flight_hook_thunk.h"
#include <intrin.h>
extern "C" {
std::uint64_t SF_FlightXsaveMask{};
std::uintptr_t SF_FlightScalarAddress{};
}
namespace PlayerOwnership {
bool PrepareScalarCpuState() noexcept {
    if(SF_FlightXsaveMask)return true;
    int leaf[4]{};__cpuid(leaf,0);
    if(leaf[0]<0xd)return false;
    __cpuid(leaf,1);
    if((leaf[2]&(1<<26))==0 || (leaf[2]&(1<<27))==0)return false;
    const auto mask=_xgetbv(0);
    if((mask&3)!=3)return false;
    __cpuidex(leaf,0xd,0);
    if(leaf[1]<576 || leaf[1]>0x4000)return false;
    SF_FlightXsaveMask=mask;
    return true;
}
}
