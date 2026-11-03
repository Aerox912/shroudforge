#pragma once
#include "read_actor_query_identity.h"

namespace PlayerOwnership {
struct ActorScalarFrame {
    ActorQueryFrame actor;
    std::uintptr_t native_rsp{}, site{};
};
// Exact Server26083076 ActorRotation scalar invocation. The original getter
// copies the full entity DWORD to RBP+560 before the clamp. That stack value
// survives to 7DE0E, unlike RSP+70, which the original code overwrites. Never
// follow the heap query-row pointer or component storage here. The caller still
// owns the original callback/stack/World interval; this is not a durable handle.
inline bool read_actor_stack_identity(ActorQueryMemory& memory,const ActorScalarFrame& scalar,
                                     ActorQueryIdentity& output) noexcept {
    output={};
    const auto& f=scalar.actor;
    if(!f.image || f.image>UINTPTR_MAX-0x644288 || !f.expected_manager || f.expected_manager%8 ||
       f.actor_rbp%8 || f.actor_rbp<0x100 || scalar.native_rsp!=f.actor_rbp-0x100 ||
       scalar.site!=f.image+0x7de0e || f.stack_low>=f.stack_high ||
       scalar.native_rsp<f.stack_low || f.actor_rbp>UINTPTR_MAX-0x1ee8 ||
       f.actor_rbp+0x1ee8>f.stack_high) return false;
    const auto dispatcher=f.actor_rbp+0x1770,query=dispatcher-0xd0,context=dispatcher-0xc0;
    struct Observation {
        std::uintptr_t saved_rbp{},return_pc{},saved_query{},descriptor{},manager{};
        std::uint64_t count{};
        std::uint32_t index{},entity{};
        bool operator==(const Observation&) const=default;
    };
    const auto observe=[&](Observation& o) noexcept {
        const auto read=[&](std::uintptr_t p,auto& v) noexcept {return memory.read(p,&v,sizeof(v));};
        return read(f.actor_rbp+0x1660,o.saved_rbp) && o.saved_rbp==dispatcher &&
            read(f.actor_rbp+0x1668,o.return_pc) && o.return_pc==f.image+0x644288 &&
            read(f.actor_rbp+0x1670,o.saved_query) && o.saved_query==query &&
            read(query,o.descriptor) && o.descriptor==context &&
            read(query+8,o.index) && o.index &&
            read(context,o.manager) && o.manager==f.expected_manager &&
            read(context+0x830,o.count) && o.count<=0x100000 && o.index<=o.count &&
            read(f.actor_rbp+0x560,o.entity) && o.entity;
    };
    Observation before,after;
    if(!observe(before) || !observe(after) || before!=after)return false;
    output={before.manager,before.entity,before.index-1};
    return true;
}
}
