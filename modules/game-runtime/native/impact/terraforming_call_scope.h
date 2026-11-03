#pragma once
#include "../voxels/augment_terraforming_region.h"

namespace ImpactRuntime::TerraformingEffects {
using namespace NativeTerraforming;
using RevalidateAction = bool (*)(const void* action_context) noexcept;
struct Admission {
    // Explicit trusted HOST assertion: real current profile/program,
    // actor/world/control/lifetime proof and the natural scope are held.
    // This component does not authenticate or manufacture that proof.
    bool supported_natural_invocation{},enabled{}; std::uint8_t radius_tier{};
    // Caller retains the original-action context throughout this scope. Reader
    // is bounded/nonblocking/noexcept and validates that original lease only.
    RevalidateAction revalidate{};
    const void* action_context{};
};
using StockInvocation=void* (__fastcall*)(void*,const void*,const void*);
using NativeQuery=std::uint8_t (__fastcall*)(void*,void*,void*,NativeQuantity*,const NativeFloatSpan*,const float*,const void*,float,std::uint8_t,std::uint8_t);
struct QueryArguments {
    void* source{}; void* optional_output{}; void* allocator{};
    NativeQuantity* quantity{}; const NativeFloatSpan* coefficients{};
    const float* center{}; const void* orientation{}; float range{};
    std::uint8_t mode{},phase{};
    // Actual known222990 terrain-query site, never inferred from readable data.
    bool natural_terrain_query{};
};
struct RegionBinding {
    bool admitted{}; std::uint8_t material{}; float radius{},normal_range{};
    std::array<float,3> center{}; NativeQuantity* quantity{};
    NativeFloatSpan coefficients{};
    const void* invocation{};
};
void* WithCallScope(const Admission&,StockInvocation,void* result,const void* arguments,const void* frame);
// original is the owner's exact native bypass. Call only at its natural seam.
std::uint8_t InvokeQuery(NativeQuery original,const QueryArguments&);
// Borrowed publication binding, only on current thread during its original call.
RegionBinding CurrentRegionBinding() noexcept;
// A refused action stays inactive. Wrong TLS tokens never validate another scope.
bool RevalidateCurrentInvocation(const void* expected_invocation) noexcept;
}
