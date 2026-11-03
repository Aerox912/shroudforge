#include "bootstrap_state.h"
#include "startup_zoom.h"

using namespace BootstrapImplementation;


namespace BootstrapImplementation {
void write_runtime_heartbeat(const std::filesystem::path& root) {
    const auto directory = ShroudforgeConfig::Directory(root, "runtime", "shroudforge/runtime");
    std::error_code error;
    std::filesystem::create_directories(directory, error);
    if (error) return;
    const auto path = directory / L"heartbeat.json";
    const auto temporary = directory / L"heartbeat.json.tmp";
    const auto now = std::chrono::duration_cast<std::chrono::seconds>(
        std::chrono::system_clock::now().time_since_epoch()).count();
    std::ofstream stream(temporary, std::ios::trunc);
    if (!stream) return;
    stream << "{\"schemaVersion\":1,\"pid\":" << GetCurrentProcessId()
        << ",\"mode\":\"" << (std::filesystem::is_regular_file(root / L"enshrouded.exe") ? "CLIENT" : "SERVER")
        << "\",\"updatedAt\":" << now << "}";
    stream.close();
    if (stream) MoveFileExW(temporary.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH);
}

void clear_runtime_heartbeat(const std::filesystem::path& root) {
    std::error_code ignored;
    std::filesystem::remove(ShroudforgeConfig::Directory(root, "runtime", "shroudforge/runtime") / L"heartbeat.json", ignored);
}

DWORD WINAPI run(void*) {
    // Never touch KFC files until the main thread is parked outside loader lock.
    // A late-loaded proxy cannot safely prepare assets for an already running game.
    if (WaitForSingleObject(StartupGate::entered, 30000) != WAIT_OBJECT_0) {
        StartupGate::Complete(false);
        return 1;
    }
    struct CompleteStartup {
        bool released = false;
        ~CompleteStartup() { if (!released) StartupGate::Complete(false); }
        void release() { released = true; StartupGate::Complete(true); }
    } startup;
    const auto root = module_directory();
    if (root.empty()) {
        log('E', "Bootstrap failed: loader directory is unavailable");
        return 1;
    }
    begin_log_session(root);
    session_started = std::chrono::steady_clock::now();
    clear_runtime_heartbeat(root);
    startup_stage(root, "bootstrap-thread-started");
    log("Log session started for game PID " + std::to_string(GetCurrentProcessId()) +
        ". Archived log files contain earlier sessions");
    log_shroudforge_banner(is_dedicated_server_process());
    // Check the release manifest before starting any UI or loading runtime DLLs.
    startup_stage(root, "startup-update-check");
    if (!run_startup_update_check(root)) return 1;
    const auto runtime_path = shroudforge_directory(root) / L"shroudforge-runtime.dll";
    startup_stage(root, "runtime-dll-load");
    const auto runtime = LoadLibraryW(runtime_path.c_str());
    if (!runtime) {
        const auto error = GetLastError();
        startup_failed(root, "runtime-dll-load", "shroudforge-runtime.dll could not be loaded. " + windows_error(error));
        EcsRuntime::Shutdown();
        return 1;
    }
    const auto prepare_startup = reinterpret_cast<PrepareStartup>(
        GetProcAddress(runtime, "shroudforge_prepare_startup"));
    const auto create = reinterpret_cast<CreateRuntime>(GetProcAddress(runtime, "shroudforge_create"));
    const auto update = reinterpret_cast<UpdateRuntime>(GetProcAddress(runtime, "shroudforge_update"));
    const auto destroy = reinterpret_cast<DestroyRuntime>(GetProcAddress(runtime, "shroudforge_destroy"));
    using EarlyProbePrepare=unsigned (__cdecl*)(const EarlyOwnerProbe::BootstrapHostV1*);
    using EarlyProbeTick=unsigned (__cdecl*)();
    using EarlyProbeRevoke=void (__cdecl*)();
    const auto early_probe_prepare=reinterpret_cast<EarlyProbePrepare>(GetProcAddress(runtime,"shroudforge_prepare_early_owner_probe"));
    const auto early_probe_tick=reinterpret_cast<EarlyProbeTick>(GetProcAddress(runtime,"shroudforge_tick_early_owner_probe"));
    const auto early_probe_revoke=reinterpret_cast<EarlyProbeRevoke>(GetProcAddress(runtime,"shroudforge_revoke_early_owner_probe"));
    const auto release_runtime=[&] { if(early_probe_revoke)early_probe_revoke();FreeLibrary(runtime); };
    if (!prepare_startup || !create || !update || !destroy) {
        startup_failed(root, "runtime-exports", "runtime exports are incomplete");
        release_runtime();
        return 1;
    }
    startup_stage(root, "runtime-exports-resolved");
    const auto proof = StartupGate::CaptureHeldEntryProof();
    if (!proof.is_held || !proof.is_held(proof.parked_thread_id)) {
        startup_failed(root, "native-preparation-gate", "The executable entrypoint is not held by this bootstrap. Native preparation was refused");
        release_runtime();
        return 1;
    }
    startup_stage(root, "native-provider-init");
    using NativeDemand = unsigned (__cdecl*)(const wchar_t*);
    const auto overlay_demand = reinterpret_cast<NativeDemand>(GetProcAddress(runtime, "shroudforge_startup_overlay_demand"));
    const auto first_person_demand = reinterpret_cast<NativeDemand>(GetProcAddress(runtime, "shroudforge_startup_first_person_demand"));
    const auto overlay = overlay_demand ? overlay_demand(root.c_str()) : 2u;
    const auto first_person = first_person_demand ? first_person_demand(root.c_str()) : 2u;
    std::optional<unsigned> vein_mining_demand;
    if (is_dedicated_server_process()) {
        const auto read_vein_mining_demand = reinterpret_cast<NativeDemand>(
            GetProcAddress(runtime, "shroudforge_startup_vein_mining_demand"));
        const auto demand = read_vein_mining_demand ? read_vein_mining_demand(root.c_str()) : 2u;
        vein_mining_demand = demand <= 1u ? demand : 2u;
        if (*vein_mining_demand == 2u) {
            log('W', "Vein Mining server startup demand unavailable. Native preparation is disabled; repair package configuration and restart");
        }
    }
    std::optional<unsigned> gifting_range_demand;
    if (is_dedicated_server_process()) {
        const auto read_gifting_range_demand = reinterpret_cast<NativeDemand>(
            GetProcAddress(runtime, "shroudforge_startup_gifting_range_demand"));
        const auto demand = read_gifting_range_demand ? read_gifting_range_demand(root.c_str()) : 2u;
        gifting_range_demand = demand <= 1u ? demand : 2u;
        if (*gifting_range_demand == 2u) {
            log('W', "Unlimited Gifting Range server startup demand unavailable. Native preparation is disabled; repair package configuration and restart");
        }
    }
    if (overlay == 2) log('W', "Native overlay boot demand unavailable. Repair package configuration and restart before rendering");
    if (first_person == 2) log('W', "First Person boot demand unavailable. Early preparation is disabled; repair package configuration and restart");
    // Before UI threads or arbitrary package-native DLLs: only this worker may
    // issue the private token while the actual game entrypoint remains parked.
    if (!EcsRuntime::Initialize(overlay == 1, &proof, first_person == 1, vein_mining_demand,
            gifting_range_demand)) {
        log('W', "KFC Runtime initialization failed: " + EcsRuntime::Status());
    }
    if (vein_mining_demand && *vein_mining_demand == 1u) {
        const auto error = EcsRuntime::VeinMiningPreparationError();
        if (!error.empty()) log('W', "Vein Mining server feature unavailable: " + error);
    }
    if (gifting_range_demand && *gifting_range_demand == 1u) {
        const auto error = EcsRuntime::GiftingRangePreparationError();
        if (!error.empty()) log('W', "Unlimited Gifting Range server feature unavailable: " + error);
    }
    // A separately selected immutable diagnostic may prepare here, on this
    // same retained owner thread, before UI/ordinary SDK startup and PE release.
    // No environment/configuration is created or enabled by the loader.
    if(early_probe_prepare && early_probe_tick && early_probe_revoke) {
        const auto host=EcsRuntime::EarlyProbeBootstrapHost(reinterpret_cast<std::uintptr_t>(runtime),proof);
        if(early_probe_prepare(&host)>1u) {
            startup_failed(root,"early-owner-probe","Selected diagnostic admission or retained atomic preparation refused; executable entry remains held");
            EcsRuntime::Shutdown();release_runtime();return 1;
        }
    } else if(GetEnvironmentVariableW(L"SHROUDFORGE_EARLY_OWNER_PROBE_ADMISSION",nullptr,0)) {
        startup_failed(root,"early-owner-probe","Selected diagnostic requires matching private early host exports");
        EcsRuntime::Shutdown();release_runtime();return 1;
    }
    start_debug_console(root);
    start_modloader_ui(root);
    start_world_editor_ui(root);
    startup_stage(root, "startup-preparation");
    if (!prepare_startup(root.c_str())) {
        startup_failed(root, "startup-preparation", "Startup preparation failed. See the preceding asset or DLL Manager error. Game startup was stopped before releasing the entrypoint");
        EcsRuntime::Shutdown();
        release_runtime();
        return 1;
    }
    using ZoomDemand = unsigned (__cdecl*)(const wchar_t*);
    const auto take_zoom_demand = reinterpret_cast<ZoomDemand>(
        GetProcAddress(runtime, "shroudforge_take_startup_zoom_demand"));
    const auto zoom_demand = take_zoom_demand ? take_zoom_demand(root.c_str()) : 2u;
    StartupZoom::Cap zoom_cap;
    const auto zoom_result = zoom_cap.Apply(zoom_demand, proof);
    if (zoom_result == StartupZoom::Result::Unsafe) {
        const bool restored = zoom_cap.Rollback(proof);
        startup_failed(root, "startup-zoom", restored
            ? "Extended Zoom write failed and was rolled back. Game startup was stopped before releasing the entrypoint"
            : "Extended Zoom write or ownership is uncertain. Game startup was stopped before releasing the entrypoint");
        EcsRuntime::Shutdown();
        release_runtime();
        return 1;
    }
    if (zoom_result == StartupZoom::Result::Unavailable ||
        (zoom_demand == 0 && zoom_result != StartupZoom::Result::Disabled) ||
        (zoom_demand == 1 && zoom_result != StartupZoom::Result::Applied)) {
        startup_failed(root, "startup-zoom", "Required Extended Zoom startup demand or executable profile is unavailable. Game startup was stopped before releasing the entrypoint");
        EcsRuntime::Shutdown();
        release_runtime();
        return 1;
    }
    if (zoom_result == StartupZoom::Result::Applied)
        log("Extended Zoom fifth level activated before game entrypoint release");
    else
        log("Extended Zoom cap disabled by verified current-assets selection");
    zoom_cap.Commit();
    log("Startup assets and native-provider setup complete. Releasing the game entrypoint");
    startup.release();
    startup_stage(root, "runtime-create");
    std::mutex create_watchdog_mutex;
    std::condition_variable create_watchdog_condition;
    bool create_finished = false;
    std::thread create_watchdog;
    try {
        create_watchdog = std::thread([&] {
            std::unique_lock lock(create_watchdog_mutex);
            if (!create_watchdog_condition.wait_for(lock, std::chrono::seconds(15),
                    [&] { return create_finished; })) {
                lock.unlock();
                log('E', "Runtime initialization has not returned after 15 seconds at runtime-create. Inspect the last [shroudforge::runtime] startup stage above");
            }
        });
    } catch (...) {
        log('W', "Could not start runtime-create watchdog");
    }
    void* handle = create(root.c_str());
    {
        std::lock_guard lock(create_watchdog_mutex);
        create_finished = true;
    }
    create_watchdog_condition.notify_one();
    if (create_watchdog.joinable()) create_watchdog.join();
    if (!handle) {
        startup_failed(root, "runtime-create", "Runtime initialization returned no handle. See preceding runtime log entries");
        EcsRuntime::Shutdown();
        release_runtime();
        return 1;
    }
    startup_stage(root, "runtime-ready");
    log("Runtime initialized");
    auto previous = std::chrono::steady_clock::now();
    auto next_runtime_status = previous;
    std::string previous_runtime_status;
    while (WaitForSingleObject(stop_event, 16) == WAIT_TIMEOUT) {
        const auto now = std::chrono::steady_clock::now();
        const auto delta = std::chrono::duration<double>(now - previous).count();
        previous = now;
        if(early_probe_tick && early_probe_tick()>1u) {
            log('E',"Early owner observation drain failed; capture revoked and runtime stopping");break;
        }
        EcsRuntime::Tick();
        if (now >= next_runtime_status) {
            write_runtime_heartbeat(root);
            const auto status = EcsRuntime::Status();
            const auto state = runtime_status_state(status);
            if (state != previous_runtime_status) {
                log('D', "KFC Runtime details " + status);
                log("KFC Runtime state: " + state);
                previous_runtime_status = state;
            }
            next_runtime_status = now + std::chrono::seconds(2);
        }
        if (!update(handle, delta)) {
            startup_failed(root, "runtime-update", "Runtime update failed. Stopping the Lua runtime");
            break;
        }
    }
    if(early_probe_revoke)early_probe_revoke();
    destroy(handle);
    EcsRuntime::Shutdown();
    stop_world_editor_ui();
    stop_modloader_ui();
    stop_debug_console();
    clear_runtime_heartbeat(root);
    release_runtime();
    log("Runtime stopped");
    start_pending_update(root);
    return 0;
}
}
