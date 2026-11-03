#pragma once
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct SFVeinLeaseV1 {
    uint32_t size,version;
    uint64_t token[2];
    uint32_t ttl_ms,renew_after_ms,flags,reserved;
} SFVeinLeaseV1;
typedef struct SFVeinUpdateV1 {
    uint32_t size,version;
    uint64_t token[2];
    uint32_t sequence,active,tier,reserved;
} SFVeinUpdateV1;
// Optional ABI-12 extensions. Intent acceptance is not mining readiness.
// All outputs require size=40/version=1. Failure clears the output payload.
// A matched Off may succeed with zero token/durations when renewal is unavailable.
// 0=accepted,1=argument/ABI,3=unavailable/stale,4=busy/capacity,6=ambiguous,7=fault.
#ifdef KFC_RUNTIME_BUILD
__declspec(dllexport)
#endif
int32_t KfcRuntimeBeginPeerVeinV1(uint64_t peer,uint64_t expected_session,SFVeinLeaseV1* out);
#ifdef KFC_RUNTIME_BUILD
__declspec(dllexport)
#endif
int32_t KfcRuntimeSetPeerVeinV1(uint64_t peer,uint64_t expected_session,const SFVeinUpdateV1* update,SFVeinLeaseV1* out);
#ifdef KFC_RUNTIME_BUILD
__declspec(dllexport)
#endif
int32_t KfcRuntimeRevokePeerVeinV1(uint64_t peer,const uint64_t token[2]);
#ifdef __cplusplus
}
static_assert(sizeof(SFVeinLeaseV1)==40 && sizeof(SFVeinUpdateV1)==40);
namespace PlayerOwnership {
class VeinIntents;
// Private desired-state access for future qualified callbacks/lifecycle cleanup.
// This function is not a provider, native ownership grant or exported SDK API.
VeinIntents& NativeVeinIntents() noexcept;
}
#endif
