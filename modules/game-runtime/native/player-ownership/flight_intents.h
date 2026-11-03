#pragma once
#include "current_player_binding.h"
#include <atomic>
#include <mutex>
#include <optional>

namespace PlayerOwnership {
// Host-thread setters only. Callback reads are bounded, lock-free and allocate
// nothing. Slots contain desired state, never native ownership or entity pins.
class FlightIntents {
public:
    struct Read {
        WorldIdentity world;
        std::uint64_t uid{},revision{};
        std::size_t slot{};
        bool enabled{};
        std::uint32_t gameplay_key{},machine_key{};
    };
    bool Disable(std::uint64_t uid) noexcept {
        if(!uid)return false;
        try {
            std::scoped_lock lock(writer_);
            for(auto& s:slots_) {
                if(s.uid.load()!=uid)continue;
                const auto version=s.revision.load();
                if(version>UINT64_MAX-2){fenced_.store(true);return false;}
                s.revision.store(version+1);s.enabled.store(false);s.revision.store(version+2);
            }
            return !fenced_.load();
        }catch(...){fenced_.store(true);return false;}
    }
    bool Set(const WorldIdentity& world,std::uint64_t uid,bool enabled,
             std::uint32_t gameplay_key=0,std::uint32_t machine_key=0) noexcept {
        if(!uid || !world.session || world.uuid==std::array<std::uint64_t,2>{} ||
           (enabled && (!gameplay_key || !machine_key)))return false;
        try {
            std::scoped_lock lock(writer_);
            if(fenced_.load())return false;
            std::size_t candidate=slots_.size();
            for(std::size_t i=0;i<slots_.size();++i) {
                const auto old=Load(i);
                if(!old)continue;
                if(old->uid==uid) {candidate=i;break;}
                if(candidate==slots_.size() && (!old->uid || old->world!=world))candidate=i;
            }
            if(candidate==slots_.size())return false;
            auto& s=slots_[candidate];const auto version=s.revision.load();
            if(version>UINT64_MAX-2) {fenced_.store(true);return false;}
            s.revision.store(version+1);
            s.uid.store(uid);s.session.store(world.session);s.uuid0.store(world.uuid[0]);s.uuid1.store(world.uuid[1]);
            s.gameplay_key.store(gameplay_key);s.machine_key.store(machine_key);
            s.enabled.store(enabled);
            s.revision.store(version+2);
            return true;
        } catch(...) {fenced_.store(true);return false;}
    }
    std::optional<Read> Get(const WorldIdentity& world,std::uint64_t uid) const noexcept {
        if(fenced_.load() || !uid)return {};
        std::optional<Read> result;
        for(std::size_t i=0;i<slots_.size();++i) {
            const auto item=Load(i);
            if(!item || item->uid!=uid || item->world!=world)continue;
            if(result)return {};
            result=item;
        }
        return result;
    }
    bool Unchanged(const Read& previous) const noexcept {
        if(fenced_.load() || !previous.enabled)return false;
        const auto current=Load(previous.slot);
        return current && current->revision==previous.revision && current->uid==previous.uid &&
            current->world==previous.world && current->enabled &&
            current->gameplay_key==previous.gameplay_key && current->machine_key==previous.machine_key;
    }
private:
    struct Slot {
        std::atomic<std::uint64_t> revision{},uid{},session{},uuid0{},uuid1{};
        std::atomic<bool> enabled{};
        std::atomic<std::uint32_t> gameplay_key{},machine_key{};
    };
    static_assert(std::atomic<std::uint64_t>::is_always_lock_free && std::atomic<bool>::is_always_lock_free);
    std::optional<Read> Load(std::size_t index) const noexcept {
        if(index>=slots_.size())return {};
        const auto& s=slots_[index];const auto version=s.revision.load();
        if(version&1)return {};
        Read copied{{s.session.load(),{s.uuid0.load(),s.uuid1.load()}},s.uid.load(),version,index,s.enabled.load(),
            s.gameplay_key.load(),s.machine_key.load()};
        if(s.revision.load()!=version)return {};
        return copied;
    }
    std::array<Slot,player_limit> slots_{};
    std::mutex writer_;
    std::atomic<bool> fenced_{};
};
}
