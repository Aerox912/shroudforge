#include "terraforming_call_scope.h"
#include <algorithm>
#include <bit>
#include <cmath>
#include <cstring>
#include <windows.h>
namespace ImpactRuntime::TerraformingEffects {
namespace {
struct Scope {
    Admission admission{};
    bool admitted{},first_seen{},selector{};
    std::uint8_t material{};
    std::array<float,3> center{};
    float range{}; void* source{}; void* allocator{};
    RegionBinding publication{};
};
thread_local Scope* current{};
constexpr std::array<std::uint8_t,34> eligible{
    6,7,14,17,18,19,24,25,26,27,29,30,31,32,33,38,48,50,
    60,63,65,68,71,72,76,82,83,84,85,89,93,96,97,98};
bool member(unsigned id) noexcept {
    for(auto value:eligible)if(value==id)return true;
    return false;
}
bool range(const void* p,std::size_t size) noexcept {
    const auto start=reinterpret_cast<std::uintptr_t>(p);
    return p&&size<=UINTPTR_MAX-start;
}
bool query_inputs(const QueryArguments& a) noexcept {
    if(!a.natural_terrain_query||!a.source||!a.allocator||a.optional_output||
       a.mode||!range(a.center,12)||!std::isfinite(a.range)||a.range<=0||a.range>50)
        return false;
    for(unsigned i=0;i<3;++i){
        if(!std::isfinite(a.center[i]))return false;
        const double low=std::floor((std::max)(0.0,static_cast<double>(a.center[i])-a.range)*2);
        const double high=std::ceil((std::max)(0.0,static_cast<double>(a.center[i])+a.range)*2);
        if(high<=low||high>INT32_MAX-16||high-low>256)return false;
    }
    return true;
}
std::uint8_t invoke(NativeQuery f,const QueryArguments& a) {
    return f(a.source,a.optional_output,a.allocator,a.quantity,a.coefficients,
        a.center,a.orientation,a.range,a.mode,a.phase);
}
void select(Scope& scope,NativeQuery f,const QueryArguments& a) {
    alignas(16) std::array<float,256> values{},coefficients{};
    alignas(16) std::array<std::byte,16> rotation{};
    const auto point=std::array<float,3>{a.center[0],a.center[1],a.center[2]};
    for(unsigned i=1;i<128;++i)coefficients[i]=std::bit_cast<float>(0x38d1b717u);
    NativeQuantity q{values.data(),256,0,{},0,0};
    NativeFloatSpan span{coefficients.data(),256};
    QueryArguments query{a.source,nullptr,a.allocator,&q,&span,point.data(),
        rotation.data(),a.range,0,1,false};
    const bool previous=scope.selector;
    scope.selector=true;
    std::uint8_t returned{};
    // POD locals only: __finally also restores TLS state for Windows SEH.
    __try { returned=invoke(f,query); }
    __finally { scope.selector=previous; }
    if(!returned||q.data!=values.data()||q.count!=256||
       span.data!=coefficients.data()||span.count!=256)return;
    float maximum=0;
    std::uint8_t winner=0;
    for(unsigned i=0;i<256;++i){
        if(!std::isfinite(values[i])||values[i]<0)return;
        const auto bits=std::bit_cast<std::uint32_t>(coefficients[i]);
        if(bits!=(i&&i<128?0x38d1b717u:0u))return;
        if(i<128&&member(i)&&values[i]>maximum){
            winner=static_cast<std::uint8_t>(i);maximum=values[i];
        }
    }
    scope.material=winner;
}
}
void* WithCallScope(const Admission& admission,StockInvocation f,void* r,const void* a,const void* c){
    if(!f)return nullptr;
    const auto error=GetLastError();
    Scope scope{};scope.admission=admission;
    scope.admitted=admission.supported_natural_invocation&&admission.enabled&&
        admission.radius_tier>=1&&admission.radius_tier<=3&&
        admission.revalidate&&admission.action_context;
    Scope* previous=current;
    // Suppression crosses nested native actions during a selector. Admission,
    // selected material and publication bindings never cross that boundary.
    scope.selector=previous&&previous->selector;
    current=&scope; // Unsupported nested invocations deliberately shadow parent.
    void* returned{};
    __try {
        if(scope.admitted&&!scope.selector)RevalidateCurrentInvocation(&scope);
        SetLastError(error);returned=f(r,a,c);
    }
    __finally { const auto after=GetLastError();current=previous;SetLastError(after); }
    return returned;
}
std::uint8_t InvokeQuery(NativeQuery f,const QueryArguments& a){
    if(!f)return 0;
    const auto error=GetLastError();
    Scope* scope=current;
    if(scope&&scope->admitted&&!scope->selector&&!scope->first_seen){
        scope->first_seen=true;
        if(a.phase==1&&RevalidateCurrentInvocation(scope)&&query_inputs(a)){
            scope->center={a.center[0],a.center[1],a.center[2]};
            scope->range=a.range;scope->source=a.source;scope->allocator=a.allocator;
            select(*scope,f,a);
        }
    }
    RegionBinding previous{};
    if(scope)previous=scope->publication;
    if(scope&&scope->admitted&&!scope->selector&&scope->material&&a.phase==0&&
       RevalidateCurrentInvocation(scope)&&query_inputs(a)&&
       a.source==scope->source&&a.allocator==scope->allocator&&
       a.range==scope->range&&a.quantity&&a.coefficients&&
       range(a.coefficients,sizeof(NativeFloatSpan))&&
       std::memcmp(a.center,scope->center.data(),12)==0){
        scope->publication={true,scope->material,5.0f*scope->admission.radius_tier,
            scope->range,scope->center,a.quantity,*a.coefficients,scope};
    } else if(scope)scope->publication={}; // Unknown nested query cannot inherit.
    std::uint8_t returned{};
    __try { SetLastError(error);returned=invoke(f,a); }
    __finally {
        const auto after=GetLastError();
        if(scope)scope->publication=previous;
        SetLastError(after);
    }
    return returned;
}
RegionBinding CurrentRegionBinding() noexcept{
    return current&&current->admitted&&!current->selector?current->publication:RegionBinding{};
}
bool RevalidateCurrentInvocation(const void* expected_invocation) noexcept{
    Scope* scope=current;
    if(!scope||scope!=expected_invocation||!scope->admitted||scope->selector)return false;
    const auto error=GetLastError();
    bool valid=false;
    // The caller owns the borrowed context until scope completion, even after
    // refusal. This reader must not reenter, block, allocate or acquire locks.
    __try {
        if(scope->admission.revalidate&&scope->admission.action_context)
            valid=scope->admission.revalidate(scope->admission.action_context);
    }
    __finally {
        if(!valid){
            scope->admitted=false;
            scope->material=0;
            scope->publication={};
        }
        SetLastError(error);
    }
    return valid;
}
}
