#include "abi/runtime_api.h"

typedef bool (KFC_RUNTIME_CALL *FirstPersonAvailableFn)(void);
typedef size_t (KFC_RUNTIME_CALL *FirstPersonDetailFn)(char*, size_t);
typedef uint32_t (KFC_RUNTIME_CALL *FirstPersonConfigureFn)(uint64_t, double);
typedef uint32_t (KFC_RUNTIME_CALL *FirstPersonSetModeFn)(uint64_t, bool);
typedef uint32_t (KFC_RUNTIME_CALL *FirstPersonReleaseOwnerFn)(uint64_t);

_Static_assert(KFC_RUNTIME_ABI_VERSION == 12u, "first-person exports are optional ABI-12 additions");
_Static_assert(KFC_RUNTIME_FIRST_PERSON_READY == 0u, "ready status ABI");
_Static_assert(KFC_RUNTIME_FIRST_PERSON_UNSUPPORTED_BUILD == 1u, "unsupported status ABI");
_Static_assert(KFC_RUNTIME_FIRST_PERSON_INVALID_ARGUMENT == 2u, "argument status ABI");
_Static_assert(KFC_RUNTIME_FIRST_PERSON_OWNER_CONFLICT == 3u, "owner status ABI");
_Static_assert(KFC_RUNTIME_FIRST_PERSON_FAILED == 4u, "failure status ABI");
_Static_assert(_Generic(&KfcRuntimeFirstPersonAvailable, FirstPersonAvailableFn: 1, default: 0),
    "availability export signature");
_Static_assert(_Generic(&KfcRuntimeFirstPersonDetail, FirstPersonDetailFn: 1, default: 0),
    "detail export signature");
_Static_assert(_Generic(&KfcRuntimeFirstPersonConfigure, FirstPersonConfigureFn: 1, default: 0),
    "configure export signature");
_Static_assert(_Generic(&KfcRuntimeFirstPersonSetMode, FirstPersonSetModeFn: 1, default: 0),
    "mode export signature");
_Static_assert(_Generic(&KfcRuntimeFirstPersonReleaseOwner, FirstPersonReleaseOwnerFn: 1, default: 0),
    "owner release export signature");

int main(void) {
    return 0;
}
