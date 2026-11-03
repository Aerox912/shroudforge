#include "../runtime_bridge.h"

#include <windows.h>

#include <filesystem>
#include <initializer_list>
#include <optional>
#include <string>

namespace {
using CountFunction = unsigned (__cdecl*)();
using AtFunction = unsigned (__cdecl*)(unsigned);

bool expect_call_order(HMODULE provider, std::initializer_list<unsigned> expected) {
    const auto count = reinterpret_cast<CountFunction>(GetProcAddress(provider, "KfcRuntimeTestCallCount"));
    const auto at = reinterpret_cast<AtFunction>(GetProcAddress(provider, "KfcRuntimeTestCallAt"));
    if (!count || !at || count() != expected.size()) return false;
    unsigned index{};
    for (const auto call : expected) {
        if (at(index++) != call) return false;
    }
    return true;
}

std::filesystem::path provider_path() {
    wchar_t executable[32768]{};
    const auto length = GetModuleFileNameW(nullptr, executable, 32768);
    if (!length || length >= 32768) return {};
    return std::filesystem::path(executable).parent_path() / L"shroudforge" / L"kfc-runtime.dll";
}
}

int main(int argc, char** argv) {
    if (argc != 2) return 10;
    const std::string scenario = argv[1];
    std::optional<unsigned> demand;
    bool expect_runtime_initialized = true;
    bool expect_error = false;
    if (scenario == "qualified") demand = 1u;
    else if (scenario == "no-demand") demand = 0u;
    else if (scenario == "no-demand-ownership-refused") { demand = 0u; SetEnvironmentVariableW(L"SF_PLAYER_OWNERSHIP_TEST_REJECT", L"1"); }
    else if (scenario == "client") demand = std::nullopt;
    else if (scenario == "unavailable") demand = 2u;
    else if (scenario == "rejected") { demand = 1u; SetEnvironmentVariableW(L"SF_VEIN_TEST_REJECT", L"1"); expect_error = true; }
    else if (scenario == "missing-export") { demand = 1u; expect_error = true; }
    else if (scenario == "ownership-refused") { demand = 1u; SetEnvironmentVariableW(L"SF_PLAYER_OWNERSHIP_TEST_REJECT", L"1"); expect_error = true; }
    else if (scenario == "ownership-missing") { demand = 1u; expect_error = true; }
    else if (scenario == "missing-provider") { demand = 1u; expect_runtime_initialized = false; expect_error = true; }
    else return 11;

    EngineHooks::HeldEntryProof proof{};
    const auto initialized = EcsRuntime::Initialize(false, &proof, false, demand);
    if (initialized != expect_runtime_initialized) return 20;
    const auto preparation_error = EcsRuntime::VeinMiningPreparationError();
    if (expect_error != !preparation_error.empty()) return 21;
    if (scenario == "missing-provider") return 0;

    const auto path = provider_path();
    const auto provider = LoadLibraryW(path.c_str());
    if (!provider) return 22;

    if (scenario == "qualified" || scenario == "no-demand" || scenario == "no-demand-ownership-refused") {
        const auto expected_demand = scenario == "qualified" ? 1u : 0u;
        const auto get_demand = reinterpret_cast<CountFunction>(GetProcAddress(provider, "KfcRuntimeTestVeinDemand"));
        if (!expect_call_order(provider, {1u, 2u, 3u}) || !get_demand || get_demand() != expected_demand) return 23;
    } else if (scenario == "client" || scenario == "unavailable" || scenario == "missing-export") {
        if (!expect_call_order(provider, {1u, 2u})) return 24;
    } else if (scenario == "rejected") {
        if (!expect_call_order(provider, {1u, 2u, 3u})) return 25;
    } else if (scenario == "ownership-refused") {
        if (!expect_call_order(provider, {1u, 2u}) ||
            preparation_error.find("player-ownership-preparation-failed") == std::string::npos) return 26;
    } else if (scenario == "ownership-missing") {
        if (!expect_call_order(provider, {1u}) ||
            preparation_error.find("player-ownership-prepare-export-unavailable") == std::string::npos) return 27;
    }

    EcsRuntime::Shutdown();
    FreeLibrary(provider);
    return 0;
}
