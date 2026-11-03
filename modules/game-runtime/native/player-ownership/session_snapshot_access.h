#pragma once
#include "player-ownership/native_player_snapshot.h"
#include "engine-context/engine_context_state.h"

namespace PlayerOwnership::NativeDetail {
using namespace EngineContextImplementation;
class NativeSnapshotMemory final : public PlayerOwnership::SnapshotMemory {
public:
    bool read(std::uintptr_t p,void* out,std::size_t size) noexcept override {
        return p && size && p<=UINTPTR_MAX-size && read_span(p,out,size);
    }
    bool increment_if(std::uintptr_t p,std::uint32_t expected) noexcept override {
        if(!p || p%4 || expected>=0x7fffffffU)return false;
        __try { return InterlockedCompareExchange(reinterpret_cast<volatile LONG*>(p),
            static_cast<LONG>(expected+1),static_cast<LONG>(expected))==static_cast<LONG>(expected); }
        __except(EXCEPTION_EXECUTE_HANDLER){return false;}
    }
    bool decrement(std::uintptr_t p) noexcept override {
        __try { return InterlockedDecrement(reinterpret_cast<volatile LONG*>(p))>=0; }
        __except(EXCEPTION_EXECUTE_HANDLER){return false;}
    }
};
struct SessionIdentity {
    std::uintptr_t service{},thread{},handle{},reference{};
    bool operator==(const SessionIdentity&) const=default;
};
inline bool session_identity(const Layout& layout,const EngineContext::EcsView& view,
                      SessionIdentity& identity) noexcept {
    const auto image=view.owners.image;
    const auto& w=layout.write;
    std::uintptr_t service{},context{},thread{},inline_name{};
    if(!field(view.scene_context,0x10,service) || !service ||
       !add(service,0x4dd0,context) || !add(context,0x3a8,thread) ||
       !add(thread,w.inline_name,inline_name))return false;
    std::uint16_t count{};
    if(!field(image,w.registry_count,count) || count>w.capacity)return false;
    std::uintptr_t rows{};
    if(!add(image,w.registry_entries,rows))return false;
    std::uintptr_t registered_handle{};
    unsigned matches{};
    for(std::size_t i=0;i<count;++i) {
        std::uintptr_t row{},name{},handle{};
        std::uint64_t length{};
        if(!add(rows,i*w.stride,row) || !field(row,w.row_name,name) ||
           !field(row,w.row_length,length) || !field(row,w.row_handle,handle))return false;
        if(name!=inline_name || length!=7)continue;
        if(++matches!=1 || !handle)return false;
        registered_handle=handle;
    }
    if(matches!=1)return false;
    std::array<char,7> name{};
    std::uintptr_t name_pointer{},handle{},callback{},actual_context{},reference{};
    std::uint64_t length{},capacity{};
    std::uint32_t stop{};
    std::uint8_t exited{};
    if(!read_span(inline_name,name.data(),name.size()) || std::memcmp(name.data(),"Session",7) ||
       !field(thread,w.name_pointer,name_pointer) || name_pointer!=inline_name ||
       !field(thread,w.name_length,length) || length!=7 ||
       !field(thread,w.name_capacity,capacity) || capacity!=w.name_capacity_value ||
       !field(thread,w.thread_handle,handle) || handle!=registered_handle ||
       !field(thread,w.thread_callback,callback) || callback!=image+0x9ff2a0 ||
       !field(thread,w.thread_context,actual_context) || actual_context!=context ||
       !field(thread,w.thread_stop,stop) || stop ||
       !field(context,8,reference) || !reference || !field(context,0xb8,exited) || exited)return false;
    identity={service,thread,handle,reference};
    return true;
}
}
