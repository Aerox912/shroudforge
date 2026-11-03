#pragma once
#include "semantic-attribution/native_site_profile.h"
#include <array>
#include <cstdint>

namespace SemanticAttribution::Native {
// Exact layout emitted by native_thunks.asm. native_rsp is a coordinate, not a
// readable native object. Only bounded current-stack scalar reads are allowed.
struct SavedMachine final {
    std::uint64_t r15,r14,r13,r12,r11,r10,r9,r8,rdi,rsi,rbp,rbx,rdx,rcx,rax,flags;
    std::uintptr_t relay_return,site_return;
    std::uintptr_t NativeRsp() const noexcept {return reinterpret_cast<std::uintptr_t>(this)+0x90;}
};
static_assert(sizeof(SavedMachine)==0x90);
struct NativeFrame final {
    std::uint32_t function{};
    std::uintptr_t entry_rsp{},return_pc{},rbp{},rbx{},rsi{},rdi{},r12{},r13{},r14{},r15{};
};
struct FrameChain final {
    std::array<NativeFrame,32> frames{};
    std::uint32_t count{};
    const NativeFrame* Find(std::uint32_t function) const noexcept;
};
bool ReadStackScalar(std::uintptr_t address,std::uint64_t&) noexcept;
bool ReadStackU32(std::uintptr_t address,std::uint32_t&) noexcept;
bool CaptureOriginalFrames(const Site&,CapturePhase,const SavedMachine&,
                           std::uintptr_t image,std::uint32_t image_size,
                           FrameChain&) noexcept;
}
