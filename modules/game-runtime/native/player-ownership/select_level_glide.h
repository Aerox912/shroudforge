#pragma once
#include "flight_intents.h"

namespace PlayerOwnership {
// Values copied inside the genuine native callback, never a Lua request.
// interval belongs to the retained common World producer instance. The native
// time/delta/scale are additional comparison keys. Their cross-phase equality
// and ordering must be established by the native adapter, not by this type.
struct SimulationStep {
    std::uint64_t interval{},time{},delta{};
    std::uint32_t scale{};
    bool Valid() const noexcept {return interval && delta && scale;}
    bool operator==(const SimulationStep&) const=default;
};
inline bool neutral_pitch_input(std::uint8_t mode,std::uint32_t axis) noexcept {
    return mode==0 && (axis&0x7fffffffU)==0;
}
inline bool finite_flight_scalar(std::uint32_t bits) noexcept {
    return (bits&0x7f800000U)!=0x7f800000U;
}
// Bounded reservation, never a wrapping fetch_add. Heavy concurrent churn or
// exhaustion permanently refuses this retained slot rather than spinning in
// an engine callback. Zero is never a usable publication number.
inline std::uint64_t reserve_flight_publication(std::atomic<std::uint64_t>& revision,
                                               std::atomic<bool>& exhausted) noexcept {
    auto previous=revision.load();
    for(unsigned attempt=0;attempt!=4;++attempt){
        if(exhausted.load())return 0;
        if(previous==UINT64_MAX)break;
        if(revision.compare_exchange_strong(previous,previous+1))return previous+1;
    }
    exhausted.store(true);
    return 0;
}

// A bounded, single-consumer handoff between original physics callbacks.
// It owns copied facts, never native memory. Acquiring a slot never waits.
// Not wired to a native producer: Binding must come from a qualified canonical
// owner; an invocation-only registry join cannot supply its entity generation.
// Concurrent publication invalidates the earlier version before trying the
// slot, so contention cannot leave a previously neutral decision usable.
class LevelGlideSamples {
    struct Sample {
        Binding binding{};
        FlightIntents::Read intent{};
        SimulationStep step{};
        std::uintptr_t velocity{};
        std::uint64_t publication{};
        bool neutral{},consumed{};
    };
    struct Slot {
        std::atomic_flag busy=ATOMIC_FLAG_INIT;
        std::atomic<std::uint64_t> revision{};
        std::atomic<bool> exhausted{};
        Sample value{};
        std::uint64_t consumed_interval{};
    };
    const FlightIntents& intents_;
    std::array<Slot,player_limit> slots_{};
    static bool SameIntent(const FlightIntents::Read& a,const FlightIntents::Read& b) noexcept {
        return a.enabled && b.enabled && a.uid==b.uid && a.world==b.world &&
            a.slot==b.slot && a.revision==b.revision &&
            a.gameplay_key==b.gameplay_key && a.machine_key==b.machine_key;
    }
    static bool Matches(const Sample& value,const Binding& binding,
                        const FlightIntents::Read& intent,const SimulationStep& step,
                        std::uintptr_t velocity) noexcept {
        return value.binding==binding && SameIntent(value.intent,intent) &&
            value.step==step && value.velocity==velocity;
    }
public:
    explicit LevelGlideSamples(const FlightIntents& intents) noexcept:intents_(intents){}
    // These selectors change one operand only. Their caller must establish the
    // actual glider/query lifetime, component ownership and matching native step.
    // They neither obtain that authority nor write velocity/transform/stamina.
    std::uint32_t SelectGliderY(const Binding& binding,const FlightIntents::Read& intent,
        const SimulationStep& step,std::uintptr_t velocity,std::uint8_t mode,
        std::uint32_t axis_bits,std::uint32_t original_bits) noexcept {
        const bool neutral=neutral_pitch_input(mode,axis_bits) && finite_flight_scalar(original_bits);
        return Publish(binding,intent,step,velocity,neutral) && neutral?0:original_bits;
    }
    // The consumer operand is gravity's computed vertical delta BEFORE the
    // original addition to Velocity.Y. Never pass total velocity/displacement:
    // doing so would also suppress other forces in that update.
    std::uint32_t SelectGravityY(const Binding& binding,const FlightIntents::Read& intent,
        const SimulationStep& step,std::uintptr_t velocity,std::uint32_t original_bits) noexcept {
        const bool selected=Take(binding,intent,step,velocity);
        return selected && finite_flight_scalar(original_bits)?0:original_bits;
    }
    bool Publish(const Binding& binding,const FlightIntents::Read& intent,
                 const SimulationStep& step,std::uintptr_t velocity,bool neutral) noexcept {
        if(intent.slot>=slots_.size())return false;
        auto& slot=slots_[intent.slot];
        const auto publication=reserve_flight_publication(slot.revision,slot.exhausted);
        if(!publication)return false;
        if(slot.exhausted.load() || slot.busy.test_and_set(std::memory_order_acquire))return false;
        // At most one successful consumer per player slot and retained World
        // call. A rejected publication cannot erase this high-water mark. A
        // second native substep in one World call conservatively stays stock.
        const bool duplicate=step.interval && step.interval<=slot.consumed_interval;
        const bool accepted=!duplicate && step.Valid() && velocity && binding.entity_id &&
            binding.platform_uid==intent.uid && binding.world==intent.world && intent.enabled &&
            binding.gameplay_key==intent.gameplay_key && binding.machine_key==intent.machine_key && intents_.Unchanged(intent) &&
            slot.revision.load()==publication;
        if(!duplicate)slot.value={binding,intent,step,velocity,publication,accepted && neutral,false};
        slot.busy.clear(std::memory_order_release);
        return accepted && !slot.exhausted.load() && slot.revision.load()==publication && intents_.Unchanged(intent);
    }
    bool Take(const Binding& binding,const FlightIntents::Read& intent,
              const SimulationStep& step,std::uintptr_t velocity) noexcept {
        if(intent.slot>=slots_.size())return false;
        auto& slot=slots_[intent.slot];
        if(slot.exhausted.load() || slot.busy.test_and_set(std::memory_order_acquire))return false;
        auto& value=slot.value;
        const bool selected=step.Valid() && value.neutral && !value.consumed &&
            value.publication==slot.revision.load() && Matches(value,binding,intent,step,velocity) && intents_.Unchanged(intent);
        const auto publication=value.publication;
        // Even an early or rejected consumer closes that interval. Otherwise
        // producer-after-consumer ordering could re-arm a repeated phase.
        if(step.Valid() && step.interval>slot.consumed_interval)slot.consumed_interval=step.interval;
        // A wrong/stale consumer must not leave a delayed grant in this slot.
        value.consumed=true;
        slot.busy.clear(std::memory_order_release);
        return selected && !slot.exhausted.load() && slot.revision.load()==publication && intents_.Unchanged(intent);
    }
};
}
