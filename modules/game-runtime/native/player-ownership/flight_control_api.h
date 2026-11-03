#pragma once
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
// Optional ABI-12 addition. Desired state is keyed by the transport-supplied
// Steam64 UID and current host epoch. 0 accepts intent, not observed gameplay.
// 1=argument,3=unavailable,4=busy,7=fault. Off does not need a current binding.
#ifdef KFC_RUNTIME_BUILD
__declspec(dllexport)
#endif
int32_t KfcRuntimeFlightSetPeerIntentV1(uint64_t peer_uid,uint64_t expected_session,uint32_t enabled);
#ifdef __cplusplus
}
#endif
