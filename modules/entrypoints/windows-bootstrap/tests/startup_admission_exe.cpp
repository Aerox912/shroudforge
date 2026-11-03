#include <windows.h>
#include <unknwn.h>
#include <cstdio>
#include <cstring>
#include "../static_proxy_attach.h"
#ifdef STATIC_PROXY
extern "C" __declspec(dllimport) DWORD WINAPI SymGetOptions();
#endif
#ifdef STATIC_BOOTSTRAP
extern "C" __declspec(dllimport) unsigned FixtureState();
#endif
#ifdef STATIC_DINPUT_PROXY
extern "C" __declspec(dllimport) HRESULT WINAPI DirectInput8Create(HINSTANCE, DWORD, REFIID, LPVOID*, LPUNKNOWN);
#endif

int main(int argc, char** argv) {
    const bool late = argc > 1 && std::strcmp(argv[1], "late") == 0;
    const bool unsupported = argc > 1 && std::strcmp(argv[1], "unsupported") == 0;
    auto image = reinterpret_cast<unsigned char*>(GetModuleHandleW(nullptr));
    auto dos = reinterpret_cast<IMAGE_DOS_HEADER*>(image);
    auto nt = reinterpret_cast<IMAGE_NT_HEADERS64*>(image + dos->e_lfanew);
    auto entry = image + nt->OptionalHeader.AddressOfEntryPoint;
    unsigned char before[14]; std::memcpy(before, entry, 14);
#ifdef STATIC_PROXY
    (void)SymGetOptions(); // Force the actual Rust proxy into the static imports.
#endif
#ifdef STATIC_DINPUT_PROXY
    auto volatile force_dinput_import = &DirectInput8Create;
    (void)force_dinput_import;
#endif
#ifdef STATIC_BOOTSTRAP
    if (FixtureState() != 1) return 10;
#endif
    auto bootstrap = late ? LoadLibraryW(L".\\winmm.dll") : GetModuleHandleW(L"winmm.dll");
    if (!bootstrap) return 11;
    auto state = reinterpret_cast<unsigned(*)()>(GetProcAddress(bootstrap, "FixtureState"));
    auto start = reinterpret_cast<BOOL(WINAPI*)(const BootstrapAdmission::StaticProxyAttachProof*)>(
        GetProcAddress(bootstrap, "ShroudforgeStartFromStaticProxy"));
    if (!state || !start || state() != ((late || unsupported) ? 0u : 1u)) return 12;
    if (late) {
        if (start(nullptr)) return 13;
        BootstrapAdmission::StaticProxyAttachProof invalid;
        if (start(&invalid) || state() || std::memcmp(before, entry, 14)) return 14;
        // The real root proxy's late DllMain has null reserved and must refuse
        // before loading/activating the bootstrap, even with SF selected.
        if (LoadLibraryW(L".\\dbghelp.dll") != nullptr) return 15;
        if (LoadLibraryW(L".\\dinput8.dll") != nullptr) return 17;
        if (state() || std::memcmp(before, entry, 14)) return 16;
    }
    std::puts(late ? "late bootstrap and actual root proxy refused without entry mutation or worker" :
        unsupported ? "unsupported process remains forwarding-only" :
        "static attachment admitted once; held entry restored before main");
    return 0;
}
