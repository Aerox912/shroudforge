#include "semantic-attribution/native_frame_capture.h"
#include <windows.h>
#include <winternl.h>

namespace SemanticAttribution::Native {
namespace {
bool in_image(std::uintptr_t p,std::size_t n,std::uintptr_t image,std::uint32_t size) noexcept {
    return p>=image && n<=size && p-image<=size-n;
}
bool primary(const RUNTIME_FUNCTION& row,std::uintptr_t image,std::uint32_t size,
             std::uint32_t& begin) noexcept {
    auto current=row;
    for(unsigned i=0;i<8;++i) {
        const auto address=image+current.UnwindData;
        if(!in_image(address,4,image,size)) return false;
        const auto* h=reinterpret_cast<const std::uint8_t*>(address);
        if((h[0]&7)!=1) return false;
        if(!(h[0]&0x20)) {begin=current.BeginAddress;return true;}
        const auto offset=(4+2*std::size_t(h[2])+3)&~std::size_t(3);
        if(!in_image(address+offset,sizeof(current),image,size)) return false;
        current=*reinterpret_cast<const RUNTIME_FUNCTION*>(address+offset);
    }
    return false;
}
bool stack_range(std::uintptr_t p,std::size_t n) noexcept {
    const auto* tib=reinterpret_cast<const NT_TIB*>(NtCurrentTeb());
    const auto low=reinterpret_cast<std::uintptr_t>(tib->StackLimit);
    const auto high=reinterpret_cast<std::uintptr_t>(tib->StackBase);
    return p>=low && p<=high && n<=high-p;
}
}
const NativeFrame* FrameChain::Find(std::uint32_t function) const noexcept {
    for(std::uint32_t i=0;i<count;++i) if(frames[i].function==function) return &frames[i];
    return nullptr;
}
bool ReadStackScalar(std::uintptr_t p,std::uint64_t& value) noexcept {
    value=0;
    if(!stack_range(p,sizeof(value))) return false;
    __try {value=*reinterpret_cast<const std::uint64_t*>(p);return true;}
    __except(EXCEPTION_EXECUTE_HANDLER) {return false;}
}
bool ReadStackU32(std::uintptr_t p,std::uint32_t& value) noexcept {
    value=0;if(!stack_range(p,sizeof(value)))return false;
    __try{value=*reinterpret_cast<const std::uint32_t*>(p);return true;}
    __except(EXCEPTION_EXECUTE_HANDLER){return false;}
}
bool CaptureOriginalFrames(const Site& site,CapturePhase phase,const SavedMachine& f,
                           std::uintptr_t image,std::uint32_t size,FrameChain& out) noexcept {
    out.count=0;
    if(!stack_range(reinterpret_cast<std::uintptr_t>(&f),sizeof(f)) ||
       f.site_return!=image+site.rva+5) return false;
    CONTEXT c{};c.ContextFlags=CONTEXT_FULL;
    c.Rsp=f.NativeRsp();c.Rip=image+site.rva+(phase==CapturePhase::Before?0:site.expected_size);
    c.Rax=f.rax;c.Rcx=f.rcx;c.Rdx=f.rdx;c.Rbx=f.rbx;c.Rbp=f.rbp;c.Rsi=f.rsi;c.Rdi=f.rdi;
    c.R8=f.r8;c.R9=f.r9;c.R10=f.r10;c.R11=f.r11;c.R12=f.r12;c.R13=f.r13;c.R14=f.r14;c.R15=f.r15;
    __try {
        while(out.count<out.frames.size() && in_image(c.Rip,1,image,size)) {
            DWORD64 base{};auto* row=RtlLookupFunctionEntry(c.Rip,&base,nullptr);
            if(!row) {
                // The sole admitted no-pdata function is the pinned B-vector
                // callback: a real leaf with no stack adjustment, including its tail route.
                if(out.count || site.native_function_begin!=0x5ffeb0 || site.native_unwind_rva) return out.count!=0;
                std::uint64_t ret{};if(!ReadStackScalar(c.Rsp,ret))return false;
                out.frames[out.count++]={0x5ffeb0,c.Rsp,ret,c.Rbp,c.Rbx,c.Rsi,c.Rdi,c.R12,c.R13,c.R14,c.R15};
                c.Rsp+=8;c.Rip=ret;continue;
            }
            if(base!=image)return out.count!=0;
            std::uint32_t begin{};
            if(!primary(*row,image,size,begin) || !stack_range(c.Rsp,8)) return false;
            const auto prior=c;
            PVOID handler{};DWORD64 establisher{};
            RtlVirtualUnwind(UNW_FLAG_NHANDLER,image,c.Rip,row,&c,&handler,&establisher,nullptr);
            if(c.Rsp<=prior.Rsp || !stack_range(c.Rsp-8,8)) return false;
            std::uint64_t ret{};
            if(!ReadStackScalar(c.Rsp-8,ret) || ret!=c.Rip) return false;
            out.frames[out.count++]={begin,c.Rsp-8,ret,prior.Rbp,prior.Rbx,prior.Rsi,prior.Rdi,
                                      prior.R12,prior.R13,prior.R14,prior.R15};
        }
        return out.count!=0;
    } __except(EXCEPTION_EXECUTE_HANDLER) {out.count=0;return false;}
}
}
