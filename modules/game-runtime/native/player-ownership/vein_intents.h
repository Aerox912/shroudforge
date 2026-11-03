#pragma once
#include "current_player_binding.h"
#include <array>
#include <atomic>
#include <cstdint>
#include <mutex>
#include <optional>

namespace PlayerOwnership {
using VeinToken=std::array<std::uint64_t,2>;
struct VeinControlBinding {
    WorldIdentity world;
    std::uint64_t uid{};
    std::uint32_t gameplay_key{},machine_key{};
    bool operator==(const VeinControlBinding&) const=default;
};
struct VeinControlLease {
    VeinToken token{};
    std::uint32_t ttl_ms{},renew_after_ms{};
};
class VeinIntents {
public:
    static constexpr std::uint32_t ttl_ms=2000,renew_after_ms=500;
    struct Read {
        VeinControlBinding binding;
        VeinToken token{};
        std::uint64_t deadline{},revision{};
        std::size_t slot{};
        std::uint32_t sequence{},tier{};
        bool active{};
        bool operator==(const Read&) const=default;
    };
    // Control-thread writers only. A binding is a copied identity, never a
    // native pointer, entity pin or ownership cache. Readers never take writer_.
    template<class Reader,class Clock,class Tokens>
    int Begin(std::uint64_t uid,std::uint64_t session,VeinControlLease& out,
              Reader&& read,Clock&& clock,Tokens&& tokens) noexcept {
        out={};if(!uid)return 1;
        try {
            std::scoped_lock lock(writer_);
            if(fenced_.load())return 7;
            VeinControlBinding binding;
            const int status=session?read(uid,session,binding):3;
            if(status || !Valid(binding,uid,session)) {
                ClearPeer(uid);return status?status:3;
            }
            const auto now=clock();
            if(now>UINT64_MAX-ttl_ms){ClearPeer(uid);return 7;}
            std::size_t selected=slots_.size();
            for(std::size_t i=0;i<slots_.size();++i) {
                const auto old=Load(i);
                if(!old){fenced_.store(true);return 7;}
                if(old->binding.uid==uid){selected=i;break;}
                if(selected==slots_.size() && (!old->binding.uid || old->deadline<=now))selected=i;
            }
            if(selected==slots_.size())return 4;
            VeinToken token{};
            bool issued=false;
            for(unsigned attempt=0;attempt<4;++attempt) {
                token={};
                if(!tokens(token))break;
                if(token==VeinToken{})continue;
                bool duplicate=false;
                for(std::size_t i=0;i<slots_.size();++i) {
                    const auto old=Load(i);
                    if(!old || old->token==token){duplicate=true;break;}
                }
                if(!duplicate){issued=true;break;}
            }
            if(!issued){ClearPeer(uid);return 7;}
            Read value;value.binding=binding;value.token=token;value.deadline=now+ttl_ms;
            value.slot=selected;value.tier=2;
            if(!Store(selected,value))return 7;
            out={token,ttl_ms,renew_after_ms};return 0;
        }catch(...){fenced_.store(true);out={};return 7;}
    }
    template<class Reader,class Clock>
    int Set(std::uint64_t uid,std::uint64_t session,const VeinToken& token,
            std::uint32_t sequence,bool active,std::uint32_t tier,VeinControlLease& out,
            Reader&& read,Clock&& clock) noexcept {
        out={};
        if(!uid || token==VeinToken{} || !sequence || tier<1 || tier>3)return 1;
        try {
            std::scoped_lock lock(writer_);
            if(fenced_.load())return 7;
            auto current=Find(uid,token);
            if(!current)return 3;
            if(sequence<=current->sequence)return 3;
            if(current->deadline<=clock()) {
                Clear(current->slot);return active?3:0;
            }
            // Consume the ordered command and clear first. Busy, failed or slow
            // binding reads cannot leave the preceding active command effective.
            current->sequence=sequence;current->active=false;
            if(!Store(current->slot,*current))return 7;
            VeinControlBinding binding;
            const int status=session?read(uid,session,binding):3;
            const auto now=clock();
            if(status || !Valid(binding,uid,session) || binding!=current->binding ||
               now>=current->deadline || now>UINT64_MAX-ttl_ms) {
                if(status!=4 || !active || now>=current->deadline)Clear(current->slot);
                // Off succeeded in clearing even if it cannot renew: empty
                // output means no usable lease, so the next On must begin again.
                if(!active)return fenced_.load()?7:0;
                return fenced_.load()?7:(status?status:3);
            }
            current->active=active;current->tier=tier;current->deadline=now+ttl_ms;
            if(!Store(current->slot,*current))return 7;
            out={token,ttl_ms,renew_after_ms};return 0;
        }catch(...){fenced_.store(true);out={};return 7;}
    }
    int Revoke(std::uint64_t uid,const VeinToken& token) noexcept {
        if(!uid || token==VeinToken{})return 1;
        try {
            std::scoped_lock lock(writer_);
            if(const auto current=Find(uid,token))Clear(current->slot);
            return fenced_.load()?7:0;
        }catch(...){fenced_.store(true);return 7;}
    }
    void RevokePeer(std::uint64_t uid) noexcept {
        try {std::scoped_lock lock(writer_);ClearPeer(uid);}catch(...){fenced_.store(true);}
    }
    void RevokeAll() noexcept {
        try {std::scoped_lock lock(writer_);for(std::size_t i=0;i<slots_.size();++i)Clear(i);}
        catch(...){fenced_.store(true);}
    }
    // The eventual qualified callback must additionally compare the fresh
    // World/actor full-key join. This accessor supplies desired state only.
    std::optional<Read> GetActive(const WorldIdentity& world,std::uint64_t uid,std::uint64_t now) const noexcept {
        if(fenced_.load() || !uid)return {};
        std::optional<Read> result;
        for(std::size_t i=0;i<slots_.size();++i) {
            const auto value=Load(i);
            if(!value || value->binding.uid!=uid || value->binding.world!=world ||
               !value->active || value->deadline<=now || value->token==VeinToken{})continue;
            if(result)return {};
            result=value;
        }
        return fenced_.load()?std::nullopt:result;
    }
    bool Unchanged(const Read& before,std::uint64_t now) const noexcept {
        if(fenced_.load() || !before.active || before.deadline<=now)return false;
        const auto current=Load(before.slot);
        return current && *current==before && !fenced_.load();
    }
private:
    struct Slot {
        std::atomic<std::uint64_t> revision{},uid{},session{},uuid0{},uuid1{},token0{},token1{},deadline{};
        std::atomic<std::uint32_t> gameplay{},machine{},sequence{},tier{},active{};
    };
    static_assert(std::atomic<std::uint64_t>::is_always_lock_free && std::atomic<std::uint32_t>::is_always_lock_free);
    static bool Valid(const VeinControlBinding& value,std::uint64_t uid,std::uint64_t session) noexcept {
        return uid && session && value.uid==uid && value.world.session==session &&
            value.world.uuid!=std::array<std::uint64_t,2>{} && value.gameplay_key && value.machine_key;
    }
    std::optional<Read> Load(std::size_t i) const noexcept {
        if(i>=slots_.size())return {};
        const auto& s=slots_[i];const auto revision=s.revision.load();
        if(revision&1)return {};
        Read value{{{s.session.load(),{s.uuid0.load(),s.uuid1.load()}},s.uid.load(),s.gameplay.load(),s.machine.load()},
            {s.token0.load(),s.token1.load()},s.deadline.load(),revision,i,s.sequence.load(),s.tier.load(),s.active.load()!=0};
        if(s.revision.load()!=revision)return {};
        return value;
    }
    std::optional<Read> Find(std::uint64_t uid,const VeinToken& token) const noexcept {
        for(std::size_t i=0;i<slots_.size();++i) {
            const auto value=Load(i);
            if(value && value->binding.uid==uid && value->token==token)return value;
        }
        return {};
    }
    bool Store(std::size_t i,const Read& value) noexcept {
        auto& s=slots_[i];const auto revision=s.revision.load();
        if(revision>UINT64_MAX-2){fenced_.store(true);return false;}
        s.revision.store(revision+1);
        s.uid.store(value.binding.uid);s.session.store(value.binding.world.session);
        s.uuid0.store(value.binding.world.uuid[0]);s.uuid1.store(value.binding.world.uuid[1]);
        s.gameplay.store(value.binding.gameplay_key);s.machine.store(value.binding.machine_key);
        s.token0.store(value.token[0]);s.token1.store(value.token[1]);s.deadline.store(value.deadline);
        s.sequence.store(value.sequence);s.tier.store(value.tier);s.active.store(value.active?1U:0U);
        s.revision.store(revision+2);return true;
    }
    void Clear(std::size_t i) noexcept {Store(i,Read{});}
    void ClearPeer(std::uint64_t uid) noexcept {
        if(!uid)return;
        for(std::size_t i=0;i<slots_.size();++i) {
            const auto value=Load(i);if(value && value->binding.uid==uid)Clear(i);
        }
    }
    std::array<Slot,player_limit> slots_{};
    std::mutex writer_;
    std::atomic<bool> fenced_{};
};
}
