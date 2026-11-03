#pragma once
#include <cstdint>
#include <cstddef>
namespace PlayerOwnership {
struct ScalarCall {
    std::uintptr_t actor_rbp,native_rsp,site;
    std::uint32_t vertical_bits,original_bits;
};
static_assert(sizeof(ScalarCall)==32 && offsetof(ScalarCall,vertical_bits)==24);
// One-time setup before any hook publication. Refuse CPUs/OS state larger than
// the fixed thunk buffer; callback code never probes CPUID or resizes storage.
bool PrepareScalarCpuState() noexcept;
}
extern "C" {
extern std::uint64_t SF_FlightXsaveMask;
extern std::uintptr_t SF_FlightScalarAddress;
void SF_FlightScalarThunk();
std::uint32_t SF_FlightSelectScalar(const PlayerOwnership::ScalarCall*) noexcept;
}
