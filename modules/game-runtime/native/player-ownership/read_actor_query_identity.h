#pragma once
#include <cstddef>
#include <cstdint>

namespace PlayerOwnership {
// Internal exact-server decoder. The caller must already own the original
// callback, stack, query-row and World lifetimes. Success copies identity only;
// it never grants permission to dereference a cached manager or modify an actor.
class ActorQueryMemory {
public:
    virtual ~ActorQueryMemory() = default;
    virtual bool read(std::uintptr_t, void*, std::size_t) noexcept = 0;
};
struct ActorQueryFrame {
    std::uintptr_t image{}, actor_rbp{}, stack_low{}, stack_high{}, expected_manager{};
};
struct ActorQueryIdentity {
    std::uintptr_t manager{};
    std::uint32_t entity_id{}, row_index{};
};
inline bool read_actor_query_identity(ActorQueryMemory& memory, const ActorQueryFrame& frame,
                                      ActorQueryIdentity& output) noexcept {
    output = {};
    constexpr std::uintptr_t return_rva = 0x644288;
    if (!frame.image || frame.image > UINTPTR_MAX-return_rva || !frame.expected_manager ||
        frame.expected_manager % 8 || frame.actor_rbp % 8 || frame.stack_low >= frame.stack_high ||
        frame.actor_rbp < frame.stack_low || frame.actor_rbp > UINTPTR_MAX-0x1ee8 ||
        frame.actor_rbp+0x1ee8 > frame.stack_high) return false;
    const auto dispatcher = frame.actor_rbp+0x1770;
    const auto query = dispatcher-0xd0;
    const auto context = dispatcher-0xc0;
    struct Observation {
        std::uintptr_t saved_rbp{}, return_pc{}, saved_query{}, descriptor{}, manager{}, rows{};
        std::uint64_t count{};
        std::uint32_t index{}, entity{};
        bool operator==(const Observation&) const = default;
    };
    const auto observe = [&](Observation& observed) noexcept {
        const auto read = [&](std::uintptr_t at, auto& value) noexcept {
            return memory.read(at,&value,sizeof(value));
        };
        if (!read(frame.actor_rbp+0x1660,observed.saved_rbp) || observed.saved_rbp!=dispatcher ||
            !read(frame.actor_rbp+0x1668,observed.return_pc) || observed.return_pc!=frame.image+return_rva ||
            !read(frame.actor_rbp+0x1670,observed.saved_query) || observed.saved_query!=query ||
            !read(query,observed.descriptor) || observed.descriptor!=context ||
            !read(query+8,observed.index) || !observed.index ||
            !read(context,observed.manager) || observed.manager!=frame.expected_manager ||
            !read(context+0x828,observed.rows) || !observed.rows || observed.rows%8 ||
            !read(context+0x830,observed.count) || observed.count>0x100000 ||
            observed.index>observed.count) return false;
        const auto offset=static_cast<std::uintptr_t>(observed.index-1)*16;
        if (observed.rows>UINTPTR_MAX-offset || observed.rows+offset>UINTPTR_MAX-sizeof(observed.entity))
            return false;
        return read(observed.rows+offset,observed.entity) && observed.entity;
    };
    Observation before,after;
    if (!observe(before) || !observe(after) || before!=after) return false;
    output={before.manager,before.entity,before.index-1};
    return true;
}
}
