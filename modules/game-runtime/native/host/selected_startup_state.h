#pragma once
#include "selected_startup.h"
#include <atomic>

namespace SelectedStartup {
// Fixed host operations. The private C ABI accepts no callback/function table.
struct Operations {
    void* context{};
    bool (*metadata)(void*, bool){};
    bool (*generic_ready)(void*){};
    bool (*map)(void*){};
    bool (*first_person)(void*){};
    bool (*graphics)(void*){};
    bool (*provider)(void*){};
    bool (*scalar)(void*){};
    bool (*vein)(void*){};
    void (*stop)(void*){};
};
class State final {
    enum class Phase { Dormant, Preparing, Prepared, Activating, Active, Failed, Closed };
    std::atomic<Phase> phase_{Phase::Dormant};
    PlanV2 plan_{}; // Immutable after the sole Preparing transition.
    bool fail(const Operations& ops) noexcept {
        auto current = phase_.load();
        while (current != Phase::Closed && !phase_.compare_exchange_weak(current, Phase::Failed)) {}
        try { if (ops.stop) ops.stop(ops.context); } catch (...) {}
        return false;
    }
public:
    bool Attempted() const noexcept { return phase_.load() != Phase::Dormant; }
    bool Prepare(const PlanV2& plan, const Operations& ops, std::uint32_t& completed) noexcept {
        completed = 0;
        if (!Valid(plan)) return false;
        auto expected = Phase::Dormant;
        if (!phase_.compare_exchange_strong(expected, Phase::Preparing)) return false;
        plan_ = plan;
        try {
            if (!ops.metadata || !ops.metadata(ops.context, (plan.services & GenericHooks) != 0)) return fail(ops);
            if ((plan.services & GenericHooks) && (!ops.generic_ready || !ops.generic_ready(ops.context))) return fail(ops);
            const auto run = [&](std::uint32_t bit, bool (*call)(void*)) {
                return !(plan.services & bit) || (call && call(ops.context));
            };
            if (!run(MapCapture, ops.map) || !run(FirstPerson, ops.first_person) ||
                !run(Graphics, ops.graphics) || !run(SharedProvider, ops.provider) ||
                !run(FlightScalar, ops.scalar)) return fail(ops);
            expected = Phase::Preparing;
            if (!phase_.compare_exchange_strong(expected, Phase::Prepared)) return fail(ops);
            completed = PreparedServices(plan);
            return true;
        } catch (...) { return fail(ops); }
    }
    bool Activate(const PlanV2& plan, const Operations& ops, std::uint32_t& completed) noexcept {
        completed = 0;
        auto expected = Phase::Prepared;
        if (!phase_.compare_exchange_strong(expected, Phase::Activating)) return false;
        // No native address/current configuration is reacquired at activation.
        if (!Same(plan_, plan)) return fail(ops);
        try {
            if ((plan.services & Vein) && (!ops.vein || !ops.vein(ops.context))) return fail(ops);
            expected = Phase::Activating;
            if (!phase_.compare_exchange_strong(expected, Phase::Active)) return fail(ops);
            completed = plan.services;
            return true;
        } catch (...) { return fail(ops); }
    }
    void Close() noexcept { phase_.store(Phase::Closed); }
};
}
