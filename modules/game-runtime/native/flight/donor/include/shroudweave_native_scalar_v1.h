#ifndef SHROUDWEAVE_NATIVE_SCALAR_V1_H
#define SHROUDWEAVE_NATIVE_SCALAR_V1_H

#include <stdint.h>
#include <stddef.h>

#if defined(_WIN32)
#define SHROUDWEAVE_NATIVE_SCALAR_CALL __cdecl
#if defined(SHROUDWEAVE_NATIVE_SCALAR_BUILD)
#define SHROUDWEAVE_NATIVE_SCALAR_EXPORT __declspec(dllexport)
#else
#define SHROUDWEAVE_NATIVE_SCALAR_EXPORT
#endif
#else
#error Shroudweave Native Scalar V1 is supported only on Windows x64.
#endif

#ifdef __cplusplus
extern "C" {
#endif

#define SHROUDWEAVE_NATIVE_SCALAR_API_VERSION_V1 1u
#define SHROUDWEAVE_NATIVE_SCALAR_FRAME_VERSION_V1 1u
#define SHROUDWEAVE_NATIVE_SCALAR_REQUEST_VERSION_V1 1u

enum ShroudweaveNativeScalarStatusV1 {
    SHROUDWEAVE_NATIVE_SCALAR_OK = 0,
    SHROUDWEAVE_NATIVE_SCALAR_INVALID_ARGUMENT = 1,
    SHROUDWEAVE_NATIVE_SCALAR_UNSUPPORTED_VERSION = 2,
    SHROUDWEAVE_NATIVE_SCALAR_UNSUPPORTED_SITE = 3,
    SHROUDWEAVE_NATIVE_SCALAR_PROFILE_MISMATCH = 4,
    SHROUDWEAVE_NATIVE_SCALAR_ALREADY_ACTIVE = 5,
    SHROUDWEAVE_NATIVE_SCALAR_INVALID_HANDLE = 6,
    SHROUDWEAVE_NATIVE_SCALAR_REENTRANT = 7,
    SHROUDWEAVE_NATIVE_SCALAR_BUSY = 8,
    SHROUDWEAVE_NATIVE_SCALAR_HOOK_FAILED = 9,
    SHROUDWEAVE_NATIVE_SCALAR_RETAINED = 10
};

enum ShroudweaveNativeScalarCallbackResultV1 {
    SHROUDWEAVE_NATIVE_SCALAR_USE_STOCK = 0,
    SHROUDWEAVE_NATIVE_SCALAR_USE_OVERRIDE = 1
};

enum ShroudweaveNativeScalarCapabilityV1 {
    SHROUDWEAVE_NATIVE_SCALAR_CAP_SYNCHRONOUS_FRAME_V1 = 1u
};

typedef struct ShroudweaveNativeScalarFrameV1 {
    uint32_t cbSize;
    uint32_t version;
    uint64_t site;
    uint64_t nativeRbp;
    uint64_t nativeRsp;
    uint32_t originalScalarBits;
    uint32_t reserved;
} ShroudweaveNativeScalarFrameV1;

typedef int32_t(SHROUDWEAVE_NATIVE_SCALAR_CALL* ShroudweaveNativeScalarCallbackV1)(
    const ShroudweaveNativeScalarFrameV1* frame,
    uint64_t context,
    uint32_t* selectedScalarBits);

typedef struct ShroudweaveNativeScalarRequestV1 {
    uint32_t cbSize;
    uint32_t version;
    uint64_t targetAddress;
    uint64_t expectedScalarAddress;
    uint8_t expectedInstruction[8];
    ShroudweaveNativeScalarCallbackV1 callback;
    uint64_t context;
    uint64_t reserved[2];
} ShroudweaveNativeScalarRequestV1;

typedef struct ShroudweaveNativeScalarApiV1 {
    uint32_t cbSize;
    uint32_t version;
    uint64_t capabilityFlags;
    uint64_t reserved[2];
    int32_t(SHROUDWEAVE_NATIVE_SCALAR_CALL* Register)(
        const ShroudweaveNativeScalarRequestV1* request,
        uint64_t* handle);
    int32_t(SHROUDWEAVE_NATIVE_SCALAR_CALL* Remove)(uint64_t handle);
} ShroudweaveNativeScalarApiV1;

// The stock scalar is reread from the request's verified RIP-relative source
// on every call. Callback status 0, any unrecognized status, a thrown callback,
// or a non-finite override uses those current stock bits. Override status 1
// accepts finite IEEE-754 binary32 bits only.
//
// Register owns one exact eight-byte MOVSS XMM0,[RIP+disp32] site at a time.
// The supplied request context remains borrowed until Remove returns OK.
// Remove restores the source before reporting OK. A BUSY or RETAINED response
// keeps the handle, callback module reference, and context live; retry Remove.
// One disabled MinHook relay is retained per support DLL instance to protect a
// thread already redirected through its relay. Remove is final for that DLL
// instance; its immutable site/scalar fallback record remains for late relay
// entries, and the DLL will not accept a second registration.
// The admitted instruction must also be a quiescent SIMD/x87 boundary: no
// live upper YMM/ZMM state or x87 register state may cross the replaced MOVSS.
// V1 preserves the XMM and MXCSR state required by the pinned SSE-only site;
// it does not save/restore AVX upper halves or x87 state.
// The callback follows the Windows x64 C ABI and must preserve its nonvolatile
// registers, including XMM6-XMM15. V1 restores source GPRs and XMM1-XMM15,
// MXCSR, flags, and incoming GetLastError; XMM0 receives the scalar result with
// memory-MOVSS upper-96 zeroing semantics.
// The caller must keep the target code and RIP-relative scalar storage mapped
// and valid for the process lifetime. A retired relay may still receive a
// thread that had not reached the dispatch counter when Remove completed.

SHROUDWEAVE_NATIVE_SCALAR_EXPORT const ShroudweaveNativeScalarApiV1*
SHROUDWEAVE_NATIVE_SCALAR_CALL ShroudweaveGetNativeScalarApiV1(void);

#ifdef __cplusplus
}
#endif

#if defined(__cplusplus)
static_assert(sizeof(ShroudweaveNativeScalarFrameV1) == 40, "FrameV1 ABI drift");
static_assert(sizeof(ShroudweaveNativeScalarRequestV1) == 64, "RequestV1 ABI drift");
static_assert(sizeof(ShroudweaveNativeScalarApiV1) == 48, "ApiV1 ABI drift");
static_assert(offsetof(ShroudweaveNativeScalarFrameV1, originalScalarBits) == 32, "FrameV1 offset drift");
static_assert(offsetof(ShroudweaveNativeScalarRequestV1, callback) == 32, "RequestV1 offset drift");
static_assert(offsetof(ShroudweaveNativeScalarApiV1, Register) == 32, "ApiV1 offset drift");
#endif

#endif
