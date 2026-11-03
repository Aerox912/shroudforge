#pragma once
#include <windows.h>

namespace BootstrapAdmission {
// Private root-proxy/bootstrap ABI. The verifier is valid only synchronously
// within the outer proxy's real, non-null-reserved process-attach callback.
// It is trusted package provenance, not a sandbox against privileged native code.
struct StaticProxyAttachProof {
    DWORD size{sizeof(StaticProxyAttachProof)};
    DWORD version{1};
    HMODULE proxy_module{};
    DWORD attach_thread{};
    const void* context{};
    BOOL (WINAPI* verify)(const void*, HMODULE, DWORD){};
};
static_assert(sizeof(StaticProxyAttachProof) == 40, "Private x64 proxy ABI changed");
}
