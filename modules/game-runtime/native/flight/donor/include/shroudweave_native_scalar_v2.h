#ifndef SHROUDWEAVE_NATIVE_SCALAR_V2_H
#define SHROUDWEAVE_NATIVE_SCALAR_V2_H

#include "shroudweave_native_scalar_v1.h"

#ifdef __cplusplus
extern "C" {
#endif

#define SHROUDWEAVE_NATIVE_SCALAR_API_VERSION_V2 2u
#define SHROUDWEAVE_NATIVE_SCALAR_FRAME_VERSION_V2 2u
#define SHROUDWEAVE_NATIVE_SCALAR_REQUEST_VERSION_V2 2u

enum ShroudweaveNativeScalarCapabilityV2 {
    SHROUDWEAVE_NATIVE_SCALAR_CAP_SYNCHRONOUS_FRAME_V2 = 1u,
    SHROUDWEAVE_NATIVE_SCALAR_CAP_XMM8_LOW_OVERRIDE_V2 = 2u
};

enum ShroudweaveNativeScalarCallbackResultV2 {
    SHROUDWEAVE_NATIVE_SCALAR_USE_STOCK_V2 = 0,
    SHROUDWEAVE_NATIVE_SCALAR_USE_SCALAR_OVERRIDE_V2 = 1,
    SHROUDWEAVE_NATIVE_SCALAR_USE_SCALAR_AND_XMM8_OVERRIDE_V2 = 2
};

typedef struct ShroudweaveNativeScalarFrameV2 {
    ShroudweaveNativeScalarFrameV1 base;
    uint32_t nativeXmm8LowBits;
    uint32_t nativeXmm1Lane1Bits;
} ShroudweaveNativeScalarFrameV2;

typedef int32_t(SHROUDWEAVE_NATIVE_SCALAR_CALL* ShroudweaveNativeScalarCallbackV2)(
    const ShroudweaveNativeScalarFrameV2* frame,
    uint64_t context,
    uint32_t* selectedScalarBits,
    uint32_t* selectedXmm8LowBits);

typedef struct ShroudweaveNativeScalarRequestV2 {
    uint32_t cbSize;
    uint32_t version;
    uint64_t targetAddress;
    uint64_t expectedScalarAddress;
    uint8_t expectedInstruction[8];
    ShroudweaveNativeScalarCallbackV2 callback;
    uint64_t context;
    uint64_t reserved[2];
} ShroudweaveNativeScalarRequestV2;

typedef struct ShroudweaveNativeScalarApiV2 {
    uint32_t cbSize;
    uint32_t version;
    uint64_t capabilityFlags;
    uint64_t reserved[2];
    int32_t(SHROUDWEAVE_NATIVE_SCALAR_CALL* Register)(
        const ShroudweaveNativeScalarRequestV2* request, uint64_t* handle);
    int32_t(SHROUDWEAVE_NATIVE_SCALAR_CALL* Remove)(uint64_t handle);
} ShroudweaveNativeScalarApiV2;

// Opt-in extension of the same admitted eight-byte MOVSS XMM0 boundary.
// V1 callers retain their exact ABI and XMM1-XMM15 preservation contract.
// A V2 callback sees owned copies of two saved native input lanes. Its base
// cbSize/version describe this complete V2 frame (48 bytes/version2).
// Status1 changes only the finite scalar. Status2 atomically accepts two
// finite outputs: scalar plus XMM8 low32, preserving XMM8 upper96. Unknown
// status, callback exception or invalid requested output uses stock scalar
// and original XMM8. No private save-slot address is exposed.
//
// The consumer must independently qualify the semantic role of XMM8 at its
// exact site and current subject before requesting the secondary effect.
// This API supplies no game-world, actor or multiplayer authority. All V1
// site-validation, SSE/x87 quiescence, one-lifetime, module-retention and
// Remove/BUSY/RETAINED requirements also apply. V1 and V2 share one registry.
SHROUDWEAVE_NATIVE_SCALAR_EXPORT const ShroudweaveNativeScalarApiV2*
SHROUDWEAVE_NATIVE_SCALAR_CALL ShroudweaveGetNativeScalarApiV2(void);

#ifdef __cplusplus
}
static_assert(sizeof(ShroudweaveNativeScalarFrameV2) == 48, "FrameV2 ABI drift");
static_assert(sizeof(ShroudweaveNativeScalarRequestV2) == 64, "RequestV2 ABI drift");
static_assert(sizeof(ShroudweaveNativeScalarApiV2) == 48, "ApiV2 ABI drift");
static_assert(offsetof(ShroudweaveNativeScalarFrameV2, nativeXmm8LowBits) == 40, "FrameV2 input offset drift");
static_assert(offsetof(ShroudweaveNativeScalarRequestV2, callback) == 32, "RequestV2 callback offset drift");
#endif

#endif
