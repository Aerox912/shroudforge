#pragma once
#include "current_player_binding.h"
#include <cstring>
#include <limits>

namespace PlayerOwnership {
// Interner Speicherzugriff. Der Aufrufer besitzt bereits die originalen
// Session-/World-Lebensdauergrenzen. Diese Schnittstelle erzeugt keinen Pin
// fuer die Serviceallokation und darf nicht direkt an Lua gegeben werden.
struct SnapshotMemory {
    virtual ~SnapshotMemory() = default;
    virtual bool read(std::uintptr_t, void*, std::size_t) noexcept = 0;
    // Atomarer bedingter Reader-Inkrement; False veraendert den Counter nicht.
    virtual bool increment_if(std::uintptr_t, std::uint32_t) noexcept = 0;
    virtual bool decrement(std::uintptr_t) noexcept = 0;
};
struct NetworkSnapshot {
    std::array<PlayerRow, player_limit> players{};
    std::array<MachineRow, machine_limit> machines{};
};
enum class SnapshotStatus { Ok, Busy, Invalid, Unreadable, ReleaseFailed };

namespace SnapshotDetail {
template<class T> bool read(SnapshotMemory& memory, std::uintptr_t address, T& value) noexcept {
    return address && address <= UINTPTR_MAX-sizeof(T) && memory.read(address,&value,sizeof(value));
}
inline SnapshotStatus rows(SnapshotMemory& memory, std::uintptr_t buffer, NetworkSnapshot& output) noexcept {
    for(std::size_t i=0;i<player_limit;++i) {
        std::array<unsigned char,32> raw{};
        if(!memory.read(buffer+0x178+i*0x150,raw.data(),raw.size()))return SnapshotStatus::Unreadable;
        auto& player=output.players[i];
        std::memcpy(&player.gameplay_key,raw.data(),4);
        std::memcpy(&player.machine_key,raw.data()+8,4);
        player.platform_tag=raw[0x10];
        std::memcpy(&player.platform_uid,raw.data()+0x18,8);
    }
    for(std::size_t i=0;i<machine_limit;++i) {
        const auto address=buffer+0x1678+i*0xf0;
        auto& machine=output.machines[i];
        std::array<std::uint64_t,3> vector{};
        if(!read(memory,address,machine.key) || !read(memory,address+0xc0,vector))return SnapshotStatus::Unreadable;
        const auto pointer=vector[0],count=vector[1],capacity=vector[2];
        if(count>player_limit || count>capacity || (count && (!pointer || pointer>UINTPTR_MAX-count*4)))
            return SnapshotStatus::Invalid;
        machine.player_count=static_cast<std::size_t>(count);
        if(count && !memory.read(static_cast<std::uintptr_t>(pointer),machine.players.data(),machine.player_count*4))
            return SnapshotStatus::Unreadable;
    }
    return SnapshotStatus::Ok;
}
}

// Ausschliesslich das qualifizierte Server26083076-Layout. Ein nativer Adapter
// muss vor Aufruf den exakten Imagehash und die Service-Lebensdauer bestaetigen.
// Maximal vier Versuche; native Schreibzugriffe betreffen nur Readercounter.
// Kein gecachter Borrow, kein Heap, kein Engine-/Lua-Callback. Erst Ok gibt eine
// eigene Kopie frei. ReleaseFailed muss der Host als unsicheren Pin behandeln.
inline SnapshotStatus copy_server_snapshot(SnapshotMemory& memory,std::uintptr_t service,
                                          NetworkSnapshot& output) noexcept {
    static_assert(sizeof(std::uintptr_t)==8);
    output={};
    if(!service || service>UINTPTR_MAX-0x4d18 || service%4)return SnapshotStatus::Invalid;
    for(unsigned attempt=0;attempt<4;++attempt) {
        std::uint32_t index{},readers{};
        if(!SnapshotDetail::read(memory,service+0x3c,index))return SnapshotStatus::Unreadable;
        if(index>1)return SnapshotStatus::Invalid;
        const auto counter=service+0x40+index*4;
        if(!SnapshotDetail::read(memory,counter,readers))return SnapshotStatus::Unreadable;
        if(readers>=0x7fffffffU)return SnapshotStatus::Invalid;
        if(!memory.increment_if(counter,readers))continue;
        std::uint32_t current{};
        if(!SnapshotDetail::read(memory,service+0x3c,current)) {
            return memory.decrement(counter)?SnapshotStatus::Unreadable:SnapshotStatus::ReleaseFailed;
        }
        if(current!=index) {
            if(!memory.decrement(counter))return SnapshotStatus::ReleaseFailed;
            continue;
        }
        NetworkSnapshot copied{};
        const auto status=SnapshotDetail::rows(memory,service+0x48+index*0x2668,copied);
        if(!memory.decrement(counter))return SnapshotStatus::ReleaseFailed;
        if(status==SnapshotStatus::Ok)output=copied;
        return status;
    }
    return SnapshotStatus::Busy;
}
}
