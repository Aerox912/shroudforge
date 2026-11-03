#include "semantic-attribution/native_cpu_state.h"
#include <intrin.h>

namespace SemanticAttribution::Native {
bool QueryCpuStateSupport(CpuStateSupport& result) noexcept {
    result={};int r[4]{};__cpuid(r,0);
    if(r[0]<0xd) return false;
    __cpuidex(r,1,0);
    if((r[2]&(1<<26))==0 || (r[2]&(1<<27))==0) return false;
    const auto mask=_xgetbv(0);
    if((mask&3)!=3) return false;
    __cpuidex(r,0xd,0);
    const auto supported=std::uint64_t(std::uint32_t(r[0]))|(std::uint64_t(std::uint32_t(r[3]))<<32);
    if((mask&~supported)!=0 || std::uint32_t(r[1])<576 || std::uint32_t(r[1])>0x4000) return false;
    result={mask,static_cast<std::uint32_t>(r[1])};return true;
}
}
