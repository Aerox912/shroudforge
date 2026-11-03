#include "abi/runtime_api.h"
#include "abi/map_overlay_api.h"
#include "host/selected_startup_state.h"
#include "host/runtime_lifecycle_state.h"
#include "engine-hooks/held_entry_preparation.h"
#include "engine-hooks/install_engine_hooks.h"
#include "graphics/render_overlay.h"
#include "flight/client_scalar_bridge.hpp"
#include "vein_flight/provider_adapter.hpp"
#include <windows.h>
#include <filesystem>
#include <string>

namespace FirstPersonControl { bool Prepare(const EngineHooks::HeldEntryToken&) noexcept; }
// Supplied by the real Map capture owner, never a fallback implementation.
namespace MapRuntime { std::int32_t PrepareCapture(const EngineHooks::HeldEntryToken&) noexcept; }
namespace {
SelectedStartup::State selected;
struct Context {
    const SelectedStartup::PlanV2* plan{};
    const EngineHooks::HeldEntryToken* token{};
    std::filesystem::path ini;
};
std::uint32_t actual_role() {
    wchar_t path[32768]{};
    const auto size = GetModuleFileNameW(nullptr, path, 32768);
    if (!size || size >= 32768) return 0;
    const auto name = std::filesystem::path(path).filename().wstring();
    if (!_wcsicmp(name.c_str(), L"enshrouded.exe")) return SelectedStartup::Client;
    if (!_wcsicmp(name.c_str(), L"enshrouded_server.exe")) return SelectedStartup::DedicatedServer;
    return 0;
}
SelectedStartup::Operations operations(Context& context) {
    return {&context,
        +[](void*, bool execution) { return ProviderImplementation::initialize_provider(execution); },
        +[](void*) { return ProviderImplementation::provider_initialized.load() && EngineHooks::StartupReady(); },
        +[](void* value) { const auto& c = *static_cast<Context*>(value); return c.token && MapRuntime::PrepareCapture(*c.token) == SF_MAP_READY; },
        +[](void* value) { const auto& c = *static_cast<Context*>(value); return c.token && FirstPersonControl::Prepare(*c.token); },
        +[](void*) { return OverlayRuntime::PrepareGraphicsBoot(true); },
        +[](void* value) {
            const auto& c = *static_cast<Context*>(value);
            VeinFlight::ProviderConfig config;
            config.is_client = c.plan->process_role == SelectedStartup::Client;
            config.is_server = c.plan->process_role == SelectedStartup::DedicatedServer;
            config.dedicated_guest_qualified = (c.plan->opt_ins & SelectedStartup::DedicatedGuest) != 0;
            config.allow_unverified_cursor_gate = (c.plan->services & SelectedStartup::Vein) && (c.plan->opt_ins & SelectedStartup::CursorGate);
            config.flight_transport_enabled = (c.plan->services & SelectedStartup::FlightScalar) && (c.plan->opt_ins & SelectedStartup::FlightTransport);
            config.vein_settings_path = c.ini;
            if (!VeinFlight::prepare_provider(config) || !VeinFlight::provider_ready()) return false;
            // Shared readiness may survive a Vein-only settings failure. This
            // actual disabled operation also requires the parsed coordinator,
            // without enabling input/persistence while Rust holds its INI lease.
            return !(c.plan->services & SelectedStartup::Vein) || VeinFlight::set_vein_enabled(false);
        },
        +[](void*) { return shroudforge::flight::prepare_scalar() && shroudforge::flight::scalar_ready(); },
        +[](void*) { return VeinFlight::set_vein_enabled(true); },
        +[](void*) { ProviderImplementation::shutdown_provider_components(); }};
}
bool admitted_ini(const SelectedStartup::PlanV2& plan, const wchar_t* path,
        std::uint32_t units, std::filesystem::path& result) {
    if (!(plan.services & SelectedStartup::Vein)) return !path && units == 0;
    if (!path || units == 0 || units >= 32768 || path[units] != 0) return false;
    const std::wstring copied(path, units);
    if (copied.find(L'\0') != std::wstring::npos) return false;
    wchar_t executable[32768]{};
    const auto size = GetModuleFileNameW(nullptr, executable, 32768);
    if (!size || size >= 32768) return false;
    const auto expected = std::filesystem::canonical(std::filesystem::path(executable).parent_path() /
        L"mods" / L"shroudweave-vein-mining-test.ini");
    result = std::filesystem::canonical(copied);
    return result == expected && std::filesystem::is_regular_file(result);
}
}
namespace ProviderImplementation {
bool selected_startup_attempted() noexcept { return selected.Attempted(); }
void close_selected_startup() noexcept { if (selected.Attempted()) selected.Close(); }
}
extern "C" {
KFC_RUNTIME_API bool __cdecl KfcRuntimePrepareSelectedV1(const SelectedStartup::PlanV2* plan,
        const EngineHooks::HeldEntryProof* proof, const wchar_t* ini,
        std::uint32_t units, std::uint32_t* completed) noexcept {
    if (!completed) return false;
    *completed = 0;
    if (!plan || !proof || !SelectedStartup::Valid(*plan) || ProviderImplementation::provider_initialized.load()) return false;
    try {
        const auto original = *plan;
        if (original.process_role != actual_role()) return false;
        Context context{&original};
        if (!admitted_ini(original, ini, units, context.ini)) return false;
        struct Call { Context* context; std::uint32_t* completed; } call{&context, completed};
        return EngineHooks::RunHeldEntryPreparation(*proof,
            +[](const EngineHooks::HeldEntryToken& token, void* value) noexcept {
                auto& call = *static_cast<Call*>(value);
                call.context->token = &token;
                return selected.Prepare(*call.context->plan, operations(*call.context), *call.completed);
            }, &call);
    } catch (...) { return false; }
}
KFC_RUNTIME_API bool __cdecl KfcRuntimeActivateSelectedV1(const SelectedStartup::PlanV2* plan,
        std::uint32_t* completed) noexcept {
    if (!completed) return false;
    *completed = 0;
    if (!plan) return false;
    try {
        const auto original = *plan;
        Context context{&original};
        return selected.Activate(original, operations(context), *completed);
    } catch (...) { return false; }
}
}
