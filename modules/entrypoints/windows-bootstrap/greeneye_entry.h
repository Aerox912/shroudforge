#pragma once
#include <windows.h>
#include <cstdint>
#include <cstring>
#include <string>
#include <limits>

namespace GreeneyeEntry {
struct TicketV1 {
    std::uint32_t bytes{},version{},kind{},reserved{};
    unsigned char binding[32]{};
};
static_assert(sizeof(TicketV1)==48);
using Take=int (__cdecl*)(const wchar_t*,TicketV1*,unsigned);
using Load=std::uintptr_t (__cdecl*)(const wchar_t*,const TicketV1*);
using Confirm=bool (__cdecl*)(const wchar_t*,const TicketV1*,std::uintptr_t,bool);
inline bool Valid(const TicketV1& t) noexcept {
    return t.bytes==48 && t.version==1 && t.kind==1 && !t.reserved;
}
// Only the exact Rust-admitted 7b97 host has this readback profile. No private
// RVA is invoked. The real EP is invoked by StartupGate after this returns.
inline bool Witness(const unsigned char* entry,const unsigned char* original,std::uintptr_t module) noexcept {
    if (!entry||!original||!module||module>(std::numeric_limits<std::uintptr_t>::max)()-0x2050) return false;
    unsigned char expected[14];std::memcpy(expected,original,14);
    expected[0]=0x48;expected[1]=0xb8;
    const auto destination=module+0x2050;
    std::memcpy(expected+2,&destination,8);
    expected[10]=0xff;expected[11]=0xe0;
    return std::memcmp(entry,expected,14)==0;
}
// Retained by the process, never a borrowed worker stack object.
struct Context {
    std::wstring root;TicketV1 ticket;Load load{};Confirm confirm{};
};
inline bool __cdecl Continue(void* opaque,unsigned char* entry,const unsigned char* original) noexcept {
    auto* c=static_cast<Context*>(opaque);
    if (!c||!Valid(c->ticket)||!c->load||!c->confirm) return false;
    const auto module=c->load(c->root.c_str(),&c->ticket);
    const bool witness=Witness(entry,original,module);
    // False witness explicitly poisons the original operation in Rust.
    return c->confirm(c->root.c_str(),&c->ticket,module,witness)&&witness;
}
}
