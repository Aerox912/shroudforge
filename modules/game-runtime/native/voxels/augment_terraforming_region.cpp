#include "augment_terraforming_region.h"
#include "../impact/terraforming_call_scope.h"
#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>
#define NOMINMAX
#include <windows.h>
namespace NativeTerraforming {
namespace {
using ImpactRuntime::TerraformingEffects::RegionBinding;
using ImpactRuntime::TerraformingEffects::CurrentRegionBinding;
using ImpactRuntime::TerraformingEffects::RevalidateCurrentInvocation;
static_assert(sizeof(void*)==8&&sizeof(NativeFloatSpan)==16);
static_assert(offsetof(NativeQuantity,other)==0x14&&offsetof(NativeQuantity,aggregate)==0x18);
static_assert(offsetof(NativeCellBuffer,dimensions)==0x10);
template<class T>T get(const void* p,std::size_t offset) noexcept{
    T result;std::memcpy(&result,static_cast<const std::byte*>(p)+offset,sizeof(T));return result;
}
bool address(const void* p,std::uint64_t bytes) noexcept{
    return p&&bytes<=UINTPTR_MAX-reinterpret_cast<std::uintptr_t>(p);
}
bool overlap(const void* a,std::uint64_t n,const void* b,std::uint64_t m) noexcept{
    const auto x=reinterpret_cast<std::uintptr_t>(a),y=reinterpret_cast<std::uintptr_t>(b);
    return x<y+m&&y<x+n; // callers establish both ranges first
}
bool same(const RegionBinding& a,const RegionBinding& b) noexcept{
    return a.admitted&&b.admitted&&a.invocation==b.invocation&&a.quantity==b.quantity&&
        a.material==b.material&&a.radius==b.radius&&a.normal_range==b.normal_range&&
        a.center==b.center&&a.coefficients.data==b.coefficients.data&&
        a.coefficients.count==b.coefficients.count;
}
bool visitor(const void* v,const RegionBinding& b) noexcept{
    if(!b.admitted||!b.material||b.material>=128||!address(v,144)||
       !address(b.quantity,28)||b.quantity->count!=256||!address(b.quantity->data,1024)||
       b.coefficients.count!=256||!address(b.coefficients.data,1024)||
       get<std::uint8_t>(v,136)!=0||get<NativeQuantity*>(v,128)!=b.quantity||
       get<const float*>(v,112)!=b.coefficients.data||get<std::uint64_t>(v,120)!=b.coefficients.count||
       get<float>(v,0)!=b.normal_range||get<float>(v,16)!=0.5f||
       get<std::uint8_t>(v,32)!=0||get<std::uint8_t>(v,108)>1)return false;
    if(overlap(v,144,b.quantity,28)||overlap(v,144,b.quantity->data,1024)||
       overlap(v,144,b.coefficients.data,1024)||
       overlap(b.quantity,28,b.quantity->data,1024)||
       overlap(b.quantity,28,b.coefficients.data,1024)||
       overlap(b.quantity->data,1024,b.coefficients.data,1024))return false;
    const float coefficient=b.coefficients.data[b.material];
    if(!std::isfinite(coefficient)||coefficient<=0.0f)return false;
    for(unsigned i=0;i<3;++i)if(get<float>(v,20+i*4)!=b.center[i]||!std::isfinite(b.center[i]))return false;
    for(unsigned i=48;i<108;i+=4)if(!std::isfinite(get<float>(v,i)))return false;
    return std::isfinite(b.radius)&&b.radius>=5&&b.radius<=15&&
        std::isfinite(b.normal_range)&&b.normal_range>0&&b.normal_range<=50;
}
struct Layout{
    NativeCellBuffer buffer{};Int3 offset{},origin{};UInt3 extent{};
    float* quantities{};std::array<std::byte,144> context{};
    std::array<std::byte,3> opaque{};
    std::array<std::uint32_t,3> begin{},end{};
};
bool layout(const RegionArguments& a,const RegionBinding& b,Layout& out) noexcept{
    if(!visitor(a.visitor,b)||!address(a.buffer,28)||!address(a.offset,12)||
       !address(a.extent,12)||!address(b.quantity,28)||b.quantity->count!=256||
       !address(b.quantity->data,1024)||b.coefficients.count!=256||
       !address(b.coefficients.data,1024))return false;
    // The proved descriptor extent is28 bytes; do not copy tail padding from
    // the32-byte C++ representation of that native header.
    out.buffer.data=a.buffer->data;out.buffer.count=a.buffer->count;
    out.buffer.dimensions=a.buffer->dimensions;
    out.offset=*a.offset;out.extent=*a.extent;
    out.origin=get<Int3>(a.visitor,4);out.quantities=b.quantity->data;out.opaque=b.quantity->opaque;
    const auto d=out.buffer.dimensions;const std::uint64_t dims[]{d.x,d.y,d.z};
    std::uint64_t volume=1;
    for(auto n:dims){if(!n||volume>UINT64_MAX/n)return false;volume*=n;}
    if(volume>out.buffer.count||out.buffer.count>UINT64_MAX/2||
       !address(out.buffer.data,out.buffer.count*2))return false;
    const void* pointers[]{a.visitor,b.quantity,b.quantity->data,b.coefficients.data,out.buffer.data,a.buffer,a.offset,a.extent};
    const std::uint64_t sizes[]{144,28,1024,1024,out.buffer.count*2,28,12,12};
    for(unsigned i=0;i<8;++i)for(unsigned j=i+1;j<8;++j)
        if(overlap(pointers[i],sizes[i],pointers[j],sizes[j]))return false;
    const std::int32_t o[]{out.offset.x,out.offset.y,out.offset.z},base[]{out.origin.x,out.origin.y,out.origin.z};
    const std::uint32_t e[]{out.extent.x,out.extent.y,out.extent.z};
    for(unsigned i=0;i<3;++i){
        if(o[i]<0||base[i]<0||!e[i]||static_cast<std::uint64_t>(o[i])+e[i]>dims[i]||
           static_cast<std::uint64_t>(base[i])+e[i]>INT32_MAX)return false;
        const double low=std::ceil((static_cast<double>(b.center[i])-b.radius)*2-base[i]-0.5);
        const double high=std::floor((static_cast<double>(b.center[i])+b.radius)*2-base[i]-0.5)+1;
        out.begin[i]=static_cast<std::uint32_t>(std::clamp(low,0.0,static_cast<double>(e[i])));
        out.end[i]=static_cast<std::uint32_t>(std::clamp(high,0.0,static_cast<double>(e[i])));
        if(out.end[i]<out.begin[i]||out.end[i]-out.begin[i]>61)return false;
    }
    std::memcpy(out.context.data(),a.visitor,144);
    return true;
}
bool unchanged(const RegionArguments& a,const RegionBinding& b,const Layout& p) noexcept{
    return same(b,CurrentRegionBinding())&&visitor(a.visitor,b)&&
        std::memcmp(a.visitor,p.context.data(),144)==0&&
        a.buffer->data==p.buffer.data&&a.buffer->count==p.buffer.count&&
        std::memcmp(&a.buffer->dimensions,&p.buffer.dimensions,12)==0&&
        std::memcmp(a.offset,&p.offset,12)==0&&std::memcmp(a.extent,&p.extent,12)==0&&
        b.quantity->data==p.quantities&&b.quantity->count==256&&b.quantity->opaque==p.opaque;
}
template<class Action>void each(const Layout& p,const RegionBinding& b,Action action) noexcept{
    const auto width=p.buffer.dimensions.x,height=p.buffer.dimensions.y;
    for(auto z=p.begin[2];z<p.end[2];++z)for(auto y=p.begin[1];y<p.end[1];++y)for(auto x=p.begin[0];x<p.end[0];++x){
        const double dx=(static_cast<double>(p.origin.x)+x+0.5)*0.5-b.center[0];
        const double dy=(static_cast<double>(p.origin.y)+y+0.5)*0.5-b.center[1];
        const double dz=(static_cast<double>(p.origin.z)+z+0.5)*0.5-b.center[2];
        if(dx*dx+dy*dy+dz*dz>static_cast<double>(b.radius)*b.radius)continue;
        const auto index=(static_cast<std::uint64_t>(p.offset.z)+z)*height*width+
            (static_cast<std::uint64_t>(p.offset.y)+y)*width+p.offset.x+x;
        const auto word=p.buffer.data[index];
        if((word&255)==b.material&&(word>>8)!=0)action(p.buffer.data[index],static_cast<float>(word>>8)/255.0f);
    }
}
}
bool PrepareEnumeration(const void* v,Int3 low,Int3 high,Enumeration& output) noexcept{
    const auto b=CurrentRegionBinding();
    if(!RevalidateCurrentInvocation(b.invocation)||!visitor(v,b)||!address(&output,sizeof(output))||
       overlap(v,144,&output,sizeof(output))||
       overlap(b.quantity,28,&output,sizeof(output))||
       overlap(b.quantity->data,1024,&output,sizeof(output))||
       overlap(b.coefficients.data,1024,&output,sizeof(output))||
       get<Int3>(v,4).x!=low.x||get<Int3>(v,4).y!=low.y||get<Int3>(v,4).z!=low.z)return false;
    const std::int32_t lows[]{low.x,low.y,low.z},highs[]{high.x,high.y,high.z};
    std::array<std::int32_t,3> expanded_low{},expanded_high{};
    for(unsigned i=0;i<3;++i){
        if(lows[i]<0||highs[i]<=lows[i])return false;
        const double lo=std::max(0.0,std::floor((static_cast<double>(b.center[i])-b.radius)*2));
        const double hi=std::max(0.0,std::ceil((static_cast<double>(b.center[i])+b.radius)*2));
        if(lo>INT32_MAX||hi>INT32_MAX)return false;
        expanded_low[i]=std::min(lows[i],static_cast<std::int32_t>(lo));
        expanded_high[i]=std::max(highs[i],static_cast<std::int32_t>(hi));
        // Native radius cap50/spacing0.5 implies at most201 cells per axis;
        //256 also accommodates normal8-cell alignment, including tier3.
        if(static_cast<std::int64_t>(expanded_high[i])-expanded_low[i]>256)return false;
    }
    std::memcpy(output.visitor.data(),v,144);
    output.low={expanded_low[0],expanded_low[1],expanded_low[2]};
    output.high={expanded_high[0],expanded_high[1],expanded_high[2]};
    std::memcpy(output.visitor.data()+4,&output.low,12);
    return true;
}
std::uint8_t InvokeRegion(NativeTransform f,const RegionArguments& a){
    if(!f)return 0;
    const auto error=GetLastError();const auto binding=CurrentRegionBinding();Layout plan{};
    const bool ready=RevalidateCurrentInvocation(binding.invocation)&&layout(a,binding,plan);
    SetLastError(error);
    const auto returned=f(a.visitor,a.buffer,a.auxiliary,a.offset,a.extent);
    const auto after=GetLastError();
    if(returned&&ready&&RevalidateCurrentInvocation(binding.invocation)&&unchanged(a,binding,plan)){
        float quantity=plan.quantities[binding.material],aggregate=binding.quantity->aggregate;
        bool valid=std::isfinite(quantity)&&quantity>=0&&std::isfinite(aggregate)&&aggregate>=0;
        each(plan,binding,[&](std::uint16_t&,float amount) noexcept{
            quantity=quantity+amount;aggregate=aggregate+amount;
            valid=valid&&std::isfinite(quantity)&&std::isfinite(aggregate);
        });
        if(valid&&RevalidateCurrentInvocation(binding.invocation)){
            each(plan,binding,[](std::uint16_t& word,float) noexcept{word=0;});
            plan.quantities[binding.material]=quantity;binding.quantity->aggregate=aggregate;
        }
    }
    SetLastError(after);return returned;
}
}
