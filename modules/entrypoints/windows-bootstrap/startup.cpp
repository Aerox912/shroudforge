#include "bootstrap_state.h"
#include <intrin.h>
#include "static_start.h"
// Keep the existing Stop ordinal 183; forwarding ordinals occupy 1..182.
#pragma comment(linker, "/export:ShroudforgeStart,@184")

using namespace BootstrapImplementation;


namespace BootstrapImplementation {
HMODULE self_module{};

HANDLE stop_event{};

HANDLE runtime_thread{};
namespace {
StaticStart start_once;
DWORD attachment_thread{};
bool static_winmm_host{};
bool supported_process{};

bool start_runtime() {
    stop_event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!stop_event) return false;
    if (!StartupGate::Install()) {
        CloseHandle(stop_event);
        stop_event = nullptr;
        return false;
    }
    runtime_thread = CreateThread(nullptr, 0, run, nullptr, 0, nullptr);
    if (!runtime_thread) {
        TerminateProcess(GetCurrentProcess(), ERROR_DLL_INIT_FAILED);
        __fastfail(FAST_FAIL_FATAL_APP_EXIT); // Published entrypoint code stays resident.
    }
    return true;
}

bool is_proxy_attach(HMODULE proxy, const void* reserved) {
    if (!proxy || !reserved || !static_winmm_host || !supported_process ||
        attachment_thread != GetCurrentThreadId()) return false;
    wchar_t path[32768]{};
    const auto length = GetModuleFileNameW(proxy, path, static_cast<DWORD>(std::size(path)));
    if (!length || length >= std::size(path)) return false;
    const std::filesystem::path loaded(path);
    const auto name = loaded.filename().wstring();
    return (CompareStringOrdinal(name.c_str(), -1, L"dbghelp.dll", -1, TRUE) == CSTR_EQUAL ||
            CompareStringOrdinal(name.c_str(), -1, L"dinput8.dll", -1, TRUE) == CSTR_EQUAL) &&
        std::filesystem::equivalent(loaded.parent_path(), module_directory());
}
}

std::filesystem::path module_directory() {
    std::wstring path(32768, L'\0');
    const auto length = GetModuleFileNameW(self_module, path.data(), static_cast<DWORD>(path.size()));
    if (length == 0 || length == path.size()) return {};
    path.resize(length);
    return std::filesystem::path(path).parent_path();
}

bool is_dedicated_server_process() {
    wchar_t path[32768]{};
    const auto length = GetModuleFileNameW(nullptr, path, static_cast<DWORD>(std::size(path)));
    if (length == 0 || length >= std::size(path)) return false;
    const std::wstring_view executable(path, length);
    const auto separator = executable.find_last_of(L"\\/");
    const auto name = separator == std::wstring_view::npos
        ? executable
        : executable.substr(separator + 1);
    return CompareStringOrdinal(name.data(), static_cast<int>(name.size()),
        L"enshrouded_server.exe", -1, TRUE) == CSTR_EQUAL;
}

std::filesystem::path shroudforge_directory(const std::filesystem::path& root) {
    return root / L"shroudforge";
}

void startup_failed(const std::filesystem::path& root, const char* stage,
        const std::string& detail) {
    log('E', std::string("Startup failed at ") + stage + ": " + detail);
    if (!is_dedicated_server_process()) show_startup_error(root, detail);
}

bool is_supported_game_process() {
    wchar_t executable_path[32768]{};
    const auto length = GetModuleFileNameW(nullptr, executable_path,
        static_cast<DWORD>(sizeof(executable_path) / sizeof(executable_path[0])));
    if (length == 0 || length >= sizeof(executable_path) / sizeof(executable_path[0])) return false;
    const std::wstring_view path(executable_path, length);
    const auto separator = path.find_last_of(L"\\/");
    const auto name = separator == std::wstring_view::npos ? path : path.substr(separator + 1);
    return CompareStringOrdinal(name.data(), static_cast<int>(name.size()),
        L"enshrouded.exe", -1, TRUE) == CSTR_EQUAL ||
        CompareStringOrdinal(name.data(), static_cast<int>(name.size()),
        L"enshrouded_server.exe", -1, TRUE) == CSTR_EQUAL;
}
}

extern "C" __declspec(dllexport) BOOL __cdecl ShroudforgeStop(DWORD timeout_milliseconds) {
    if (stop_event) SetEvent(stop_event);
    if (!runtime_thread) return TRUE;
    return WaitForSingleObject(runtime_thread, timeout_milliseconds) == WAIT_OBJECT_0;
}

// Private native ABI: the feature-enabled SF proxies pass their actual module
// and DLL_PROCESS_ATTACH lpReserved after selecting SF. This is an internal
// call contract, not a security boundary against code already in the process.
extern "C" __declspec(dllexport) BOOL __cdecl ShroudforgeStart(
        HMODULE proxy_module, const void* proxy_attach_reserved) noexcept {
    try {
        return start_once.Try(is_proxy_attach(proxy_module, proxy_attach_reserved), start_runtime);
    } catch (...) { return FALSE; }
}

BOOL APIENTRY DllMain(HMODULE module, DWORD reason, LPVOID reserved) {
    if (reason == DLL_PROCESS_ATTACH) {
        self_module = module;
        attachment_thread = GetCurrentThreadId();
        StartupGate::direct_static_attachment=reserved!=nullptr;
        DisableThreadLibraryCalls(module);
        // winmm.dll is found beside every executable in the game directory.
        // The bundled ShroudForge UI also runs there, but must never bootstrap
        // another runtime or race Enshrouded's asset startup.
        supported_process = is_supported_game_process();
        if (!supported_process) return TRUE;
        {
            wchar_t loaded_path[32768]{};
            const auto size=GetModuleFileNameW(module,loaded_path,32768);
            if(!size||size>=32768)return FALSE;
            const auto name=std::filesystem::path(loaded_path).filename().wstring();
            // The SF winmm host is never a dynamically loaded dependency.
            // Historical separately named assets bootstraps retain their route.
            const auto attach = ClassifyHostAttach(supported_process,
                CompareStringOrdinal(name.c_str(),-1,L"winmm.dll",-1,TRUE)==CSTR_EQUAL,
                reserved != nullptr);
            if (attach == HostAttach::Reject) return FALSE;
            if (attach == HostAttach::Deferred) {
                static_winmm_host = true;
                // Importing winmm must not select SF. EML-selected proxies leave
                // it dormant; the SF-selected proxy explicitly starts it later.
                return TRUE;
            }
        }
        // The historical separately named assets bootstrap keeps its old route.
        return start_once.Try(true, start_runtime);
    }
    if (reason == DLL_PROCESS_DETACH && reserved == nullptr && stop_event) {
        SetEvent(stop_event);
    }
    return TRUE;
}
