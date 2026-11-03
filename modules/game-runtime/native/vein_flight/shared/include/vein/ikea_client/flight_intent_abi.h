// SPDX-License-Identifier: MIT
#pragma once
#include <stdint.h>
#include <stddef.h>
#ifndef __cplusplus
#include <stdbool.h>
#endif
#ifdef _MSC_VER
#define ESM_VEIN_FLIGHT_CALL __cdecl
#else
#define ESM_VEIN_FLIGHT_CALL
#endif
#ifdef __cplusplus
extern "C" {
#endif
typedef struct EsmVeinFlightIntentV1 {
    uint32_t struct_bytes,abi_version;
    uint64_t publisher_epoch;
    uint32_t requested_on,reserved;
} EsmVeinFlightIntentV1;
typedef uint32_t (ESM_VEIN_FLIGHT_CALL *EsmVeinPublishFlightIntentV1Fn)(const EsmVeinFlightIntentV1*);

// Canonical final client lifecycle ABI. The serial is LOCAL to the one resident
// provider/module lifetime, nonzero and monotonically increasing until it
// saturates. It is not a native peer/entity/session generation. A serial read is
// an observation only: V2 publication atomically rechecks it against reset.
typedef struct EsmVeinFlightIntentV2 {
    uint32_t struct_bytes,abi_version;
    uint64_t publisher_epoch;
    uint32_t requested_on,reserved0;
    uint64_t observed_reset_serial;
} EsmVeinFlightIntentV2;

typedef bool (ESM_VEIN_FLIGHT_CALL *EsmVeinGetFlightResetSerialV1Fn)(uint64_t* outSerial);
typedef bool (ESM_VEIN_FLIGHT_CALL *EsmVeinPublishFlightIntentV2Fn)(const EsmVeinFlightIntentV2* intent);

bool ESM_VEIN_FLIGHT_CALL ESM_VeinGetFlightResetSerialV1(uint64_t* outSerial);
bool ESM_VEIN_FLIGHT_CALL ESM_VeinPublishFlightIntentV2(const EsmVeinFlightIntentV2* intent);
#ifdef __cplusplus
}
static_assert(sizeof(EsmVeinFlightIntentV1)==24);
static_assert(offsetof(EsmVeinFlightIntentV1,publisher_epoch)==8);
static_assert(offsetof(EsmVeinFlightIntentV1,requested_on)==16);
static_assert(offsetof(EsmVeinFlightIntentV1,reserved)==20);
static_assert(sizeof(EsmVeinFlightIntentV2)==32);
static_assert(offsetof(EsmVeinFlightIntentV2,struct_bytes)==0);
static_assert(offsetof(EsmVeinFlightIntentV2,abi_version)==4);
static_assert(offsetof(EsmVeinFlightIntentV2,publisher_epoch)==8);
static_assert(offsetof(EsmVeinFlightIntentV2,requested_on)==16);
static_assert(offsetof(EsmVeinFlightIntentV2,reserved0)==20);
static_assert(offsetof(EsmVeinFlightIntentV2,observed_reset_serial)==24);
#endif
