#pragma once
#include <windows.h>
#include <cstdint>
#include "engine-hooks/held_entry_preparation.h"

namespace PlayerOwnership {
class ServiceReadLease final {
    friend ServiceReadLease TryReadServiceLifetime() noexcept;
    SRWLOCK* gate_{};
    explicit ServiceReadLease(SRWLOCK* gate) noexcept:gate_(gate){}
public:
    ServiceReadLease(const ServiceReadLease&)=delete;
    ServiceReadLease& operator=(const ServiceReadLease&)=delete;
    ServiceReadLease(ServiceReadLease&& other) noexcept:gate_(other.gate_){other.gate_=nullptr;}
    ~ServiceReadLease(){if(gate_)ReleaseSRWLockShared(gate_);}
    explicit operator bool() const noexcept{return gate_!=nullptr;}
};
bool ServiceLifetimeReady() noexcept;
ServiceReadLease TryReadServiceLifetime() noexcept;
bool PrepareServiceRetirement(const EngineHooks::HeldEntryToken&) noexcept;

namespace NativeDetail {
using OriginalSessionDestroy=void (__fastcall*)(std::uintptr_t);
// Bootstrap-only preparation, before publishing the one qualified callsite.
void SetOriginalSessionDestroy(OriginalSessionDestroy) noexcept;
bool PublishServiceLifetimeReady() noexcept;
void __fastcall DestroySessionAfterReaderDrain(std::uintptr_t context);
}
}
