#pragma once
#include "read_actor_query_identity.h"

namespace PlayerOwnership {
struct GliderMotionFrame {
    ActorQueryFrame actor;
    std::uintptr_t native_rsp{},query{},context{},velocity{},site{};
};
struct GliderMotionObservation {
    ActorQueryIdentity actor;
    std::uintptr_t velocity{};
    std::uint64_t time{},delta{};
    std::uint32_t scale{};
};
// Exact Server26083076 glider write at 1CD332. This only copies stack values
// inside the genuine original helper invocation. No pointer read from the
// stack is dereferenced outside that stack, including the Velocity destination.
// It is NOT a canonical entity handle, component borrow, neutral-input decoder,
// scheduler-order proof, or proof of a clock shared with another callback.
// No production hook calls this decoder until those separate gaps are closed.
inline bool read_glider_motion_frame(ActorQueryMemory& memory,const GliderMotionFrame& frame,
                                    GliderMotionObservation& output) noexcept {
    output={};
    const auto& f=frame.actor;
    const auto g=f.actor_rbp;
    if(!f.image || f.image>UINTPTR_MAX-0x644288 || !f.expected_manager || f.expected_manager%8 ||
       g%8 || g<0x100 || g>UINTPTR_MAX-0xcf8 || f.stack_low>=f.stack_high ||
       frame.native_rsp!=g-0x100 || frame.native_rsp<f.stack_low || g+0xcf8>f.stack_high ||
       frame.query!=g+0x4b0 || frame.context!=g+0x150 || !frame.velocity ||
       frame.site!=f.image+0x1cd332)return false;
    const auto descriptor=g+0x4c0;
    struct Observation {
        std::uintptr_t parent{},helper_return{},dispatcher{},outer_return{},query{},descriptor{},manager{},velocity{};
        std::uint64_t count{},time{},delta{};
        std::uint32_t index{},entity{},scale{};
        bool operator==(const Observation&) const=default;
    };
    const auto observe=[&](Observation& o) noexcept {
        const auto read=[&](std::uintptr_t at,auto& value) noexcept {return memory.read(at,&value,sizeof(value));};
        return read(g+0xe0,o.parent) && o.parent==g+0x1f0 &&
            read(g+0xe8,o.helper_return) && o.helper_return==f.image+0x1d7e91 &&
            read(g+0x470,o.dispatcher) && o.dispatcher==g+0x580 &&
            read(g+0x478,o.outer_return) && o.outer_return==f.image+0x644288 &&
            read(frame.context,o.query) && o.query==frame.query &&
            read(frame.query,o.descriptor) && o.descriptor==descriptor &&
            read(frame.query+8,o.index) && o.index &&
            read(descriptor,o.manager) && o.manager==f.expected_manager &&
            read(descriptor+0x830,o.count) && o.count<=0x100000 && o.index<=o.count &&
            read(g-0x38,o.entity) && o.entity &&
            read(frame.context+0x158,o.velocity) && o.velocity==frame.velocity &&
            read(descriptor+0x50,o.time) && read(descriptor+0x58,o.delta) && read(descriptor+0x6c,o.scale);
    };
    Observation before,after;
    if(!observe(before) || !observe(after) || before!=after)return false;
    output={{before.manager,before.entity,before.index-1},before.velocity,before.time,before.delta,before.scale};
    return true;
}
}
