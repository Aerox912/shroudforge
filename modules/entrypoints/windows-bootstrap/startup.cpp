#include "bootstrap_state.h"
#include "static_proxy_attach.h"
#include <atomic>
#include <intrin.h>

using namespace BootstrapImplementation;


namespace BootstrapImplementation {
HMODULE self_module{};

HANDLE stop_event{};

HANDLE runtime_thread{};

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

namespace {
// DllMain and the private root-proxy call are loader-serialized, but keep the
// once-only rule explicit for duplicate static proxies and rejected re-entry.
std::atomic<unsigned> startup_phase{}; // 0 inert, 1 installing, 2 started, 3 refused

bool same_text(std::wstring_view a, std::wstring_view b) {
    return CompareStringOrdinal(a.data(), static_cast<int>(a.size()),
        b.data(), static_cast<int>(b.size()), TRUE) == CSTR_EQUAL;
}
bool valid_static_proxy(const BootstrapAdmission::StaticProxyAttachProof* proof) {
    if (!proof || proof->size != sizeof(*proof) || proof->version != 1 ||
        !proof->proxy_module || !proof->context || !proof->verify ||
        !proof->attach_thread || proof->attach_thread != GetCurrentThreadId()) return false;
    HMODULE callback_module{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            reinterpret_cast<LPCWSTR>(proof->verify), &callback_module) || callback_module != proof->proxy_module) return false;
    wchar_t proxy_path[32768]{}, bootstrap_path[32768]{};
    const auto proxy_length = GetModuleFileNameW(proof->proxy_module, proxy_path, 32768);
    const auto bootstrap_length = GetModuleFileNameW(self_module, bootstrap_path, 32768);
    if (!proxy_length || proxy_length >= 32768 || !bootstrap_length || bootstrap_length >= 32768) return false;
    const std::wstring_view proxy(proxy_path, proxy_length), bootstrap(bootstrap_path, bootstrap_length);
    const auto proxy_slash = proxy.find_last_of(L"\\/");
    const auto bootstrap_slash = bootstrap.find_last_of(L"\\/");
    if (proxy_slash == std::wstring_view::npos || bootstrap_slash == std::wstring_view::npos ||
        !same_text(proxy.substr(0,proxy_slash), bootstrap.substr(0,bootstrap_slash))) return false;
    const auto name = proxy.substr(proxy_slash+1);
    if (!same_text(name,L"dbghelp.dll") && !same_text(name,L"dinput8.dll")) return false;
    return proof->verify(proof->context, proof->proxy_module, proof->attach_thread) != FALSE;
}
BOOL start_bootstrap_once() {
    unsigned expected{};
    if (!startup_phase.compare_exchange_strong(expected,1,std::memory_order_acq_rel))
        return expected == 2 ? TRUE : FALSE;
    stop_event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!stop_event || !StartupGate::Install()) {
        startup_phase.store(3,std::memory_order_release);
        return FALSE; // Gate installation returns normally only before publication.
    }
    runtime_thread = CreateThread(nullptr, 0, run, nullptr, 0, nullptr);
    if (!runtime_thread) {
        // A published entrypoint jump must never survive unloading this DLL.
        // Abrupt fail-stop avoids loader-lock teardown callbacks or unsafe undo.
        TerminateProcess(GetCurrentProcess(), ERROR_DLL_INIT_FAILED);
        __fastfail(FAST_FAIL_FATAL_APP_EXIT);
    }
    startup_phase.store(2,std::memory_order_release);
    return TRUE;
}
}

extern "C" __declspec(dllexport) BOOL WINAPI ShroudforgeStartFromStaticProxy(
        const BootstrapAdmission::StaticProxyAttachProof* proof) {
    if (!is_supported_game_process()) return TRUE; // Forwarding-only in unrelated processes.
    if (!valid_static_proxy(proof)) return FALSE;
    return start_bootstrap_once();
}

extern "C" __declspec(dllexport) BOOL __cdecl ShroudforgeStop(DWORD timeout_milliseconds) {
    if (stop_event) SetEvent(stop_event);
    if (!runtime_thread) return TRUE;
    return WaitForSingleObject(runtime_thread, timeout_milliseconds) == WAIT_OBJECT_0;
}

BOOL APIENTRY DllMain(HMODULE module, DWORD reason, LPVOID reserved) {
    if (reason == DLL_PROCESS_ATTACH) {
        self_module = module;
        DisableThreadLibraryCalls(module);
        // winmm.dll is found beside every executable in the game directory.
        // The bundled ShroudForge UI also runs there, but must never bootstrap
        // another runtime or race Enshrouded's asset startup.
        if (!is_supported_game_process()) return TRUE;
        // Non-null is the OS's direct static process-startup attachment. A
        // nested LoadLibrary from a static root proxy has null here and stays
        // inert until that proxy supplies the scoped private admission proof.
        // All ordinary late LoadLibrary attaches are therefore inert too.
        if (reserved == nullptr) return TRUE;
        return start_bootstrap_once();
    }
    if (reason == DLL_PROCESS_DETACH && reserved == nullptr && stop_event) {
        SetEvent(stop_event);
    }
    return TRUE;
}
