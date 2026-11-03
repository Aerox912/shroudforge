#pragma once
#include "engine-context/engine_owners.h"
#include <windows.h>
#include <utility>

namespace PlayerOwnership {
// Internal host interval, emitted ONLY by the exact original Scene-held World
// call wrapper. The lock is not itself native ownership. End runs before that
// wrapper returns to the engine, extending the original borrow through our
// readers. It may wait there; actor callbacks only try-acquire shared access.
// No caller may hold another host/native gate while entering Begin or End.
class OriginalWorldInterval {
public:
    class Lease {
    public:
        Lease()=default;
        Lease(Lease&& other) noexcept:lock_(std::exchange(other.lock_,nullptr)),view_(other.view_),
            generation_(std::exchange(other.generation_,0)){}
        Lease(const Lease&)=delete;
        Lease& operator=(const Lease&)=delete;
        ~Lease(){Release();}
        bool Held() const noexcept {return lock_!=nullptr;}
        // Identifies a live interval in this retained producer instance only.
        // A saved number is not a borrow; released/moved-from leases return 0.
        std::uint64_t Generation() const noexcept {return Held()?generation_:0;}
        const EngineContext::EcsView& View() const noexcept {return view_;}
        void Release() noexcept {if(lock_){ReleaseSRWLockShared(lock_);lock_=nullptr;}generation_=0;}
    private:
        friend class OriginalWorldInterval;
        SRWLOCK* lock_{};
        EngineContext::EcsView view_{};
        std::uint64_t generation_{};
    };
    bool Begin(const EngineContext::EcsView& view) noexcept {
        if(!TryAcquireSRWLockExclusive(&lock_))return false;
        const bool start=!active_ && generation_!=UINT64_MAX;
        if(start){++generation_;view_=view;active_=true;}
        ReleaseSRWLockExclusive(&lock_);
        return start;
    }
    void End() noexcept {
        AcquireSRWLockExclusive(&lock_);
        active_=false;view_={};
        ReleaseSRWLockExclusive(&lock_);
    }
    Lease TryRead() noexcept {
        Lease result;
        if(!TryAcquireSRWLockShared(&lock_))return result;
        if(!active_){ReleaseSRWLockShared(&lock_);return result;}
        result.lock_=&lock_;result.view_=view_;result.generation_=generation_;
        return result;
    }
private:
    SRWLOCK lock_=SRWLOCK_INIT;
    bool active_{};
    EngineContext::EcsView view_{};
    std::uint64_t generation_{};
};
}
