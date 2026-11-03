#pragma once
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct SFPlayerBindingV1 {
    uint32_t size,version;
    uint64_t session,world_uuid[2],peer_uid;
    uint32_t gameplay_key,machine_key,entity_id,entity_generation,flags,reserved;
} SFPlayerBindingV1;
// Optionaler ABI-12-Zusatzexport. Ausgabe benoetigt size=64/version=1.
// 0=aktuell,1=Argument/ABI,3=nicht verfuegbar,4=busy,6=mehrdeutig,7=Fault.
// Ein Fehler liefert keinen Entitywert. Ein Wert ist kein spaeterer World-Pin.
#ifdef KFC_RUNTIME_BUILD
__declspec(dllexport)
#endif
int32_t KfcRuntimeReadPlayerBindingV1(uint64_t peer_uid,uint64_t expected_session,SFPlayerBindingV1* output);
#ifdef __cplusplus
}
static_assert(sizeof(SFPlayerBindingV1)==64);
#endif
