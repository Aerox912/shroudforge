#include "bootstrap_state.h"
#include "startup_zoom.h"
#include "startup_admission.h"
#include "greeneye_entry.h"

using namespace BootstrapImplementation;


namespace BootstrapImplementation {
long long seconds_now() {
    return std::chrono::duration_cast<std::chrono::seconds>(
        std::chrono::system_clock::now().time_since_epoch()).count();
}

void write_runtime_heartbeat(const std::filesystem::path& root, long long updated_at) {
    const auto directory = ShroudforgeConfig::Directory(root, "runtime", "shroudforge/runtime");
    std::error_code error;
    std::filesystem::create_directories(directory, error);
    if (error) return;
    const auto path = directory / L"heartbeat.json";
    const auto temporary = directory / L"heartbeat.json.tmp";
    std::ofstream stream(temporary, std::ios::trunc);
    if (!stream) return;
    stream << "{\"schemaVersion\":1,\"pid\":" << GetCurrentProcessId()
        << ",\"mode\":\"" << (std::filesystem::is_regular_file(root / L"enshrouded.exe") ? "CLIENT" : "SERVER")
        << "\",\"updatedAt\":" << updated_at << "}";
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
    StartupAdmission::Completion startup(&StartupGate::Complete);
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
    const auto current_config = root / L"shroudforge" / L"config";
    const bool prepared_installation = std::filesystem::exists(current_config / L"current-assets.json") ||
        std::filesystem::exists(current_config / L"current-assets-state.json") ||
        std::filesystem::exists(current_config / L"current-assets-pending.json");
    if (prepared_installation && is_dedicated_server_process()) {
        startup_failed(root, "current-assets", "Current-composition policy is client-only");
        return 1;
    }
    // Check the release manifest before starting any UI or loading runtime DLLs.
    startup_stage(root, "startup-update-check");
    if (!prepared_installation && !run_startup_update_check(root)) return 1;
    // Start the independent log viewer before any ShroudForge DLL or mod code
    // can fail during process startup.
    if (!prepared_installation) start_debug_console(root);
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
    using StartupMode=unsigned (__cdecl*)(const wchar_t*);
    const auto startup_mode=reinterpret_cast<StartupMode>(GetProcAddress(runtime,"shroudforge_startup_mode"));
    const auto mode=startup_mode?startup_mode(root.c_str()):(prepared_installation?~0u:0u);
    if(mode>2u||prepared_installation!=(mode!=0u)) {
        startup_failed(root,"startup-mode","Prepared installation lacks a supported explicit startup contract");
        return 1;
    }
    const bool assets_only=mode==1u;
    const bool full_runtime=mode==2u;
    if(full_runtime) {
        wchar_t module_path[32768]{};
        const auto size=GetModuleFileNameW(self_module,module_path,32768);
        std::error_code host_error;
        if(!StartupGate::direct_static_attachment||!size||size>=32768||
            !std::filesystem::equivalent(std::filesystem::path(module_path),root/L"winmm.dll",host_error)) {
            startup_failed(root,"full-startup-host","Full runtime requires the statically attached SF winmm host started by its selected proxy");return 1;
        }
    }
    const auto prepare_full=reinterpret_cast<PrepareStartup>(GetProcAddress(runtime,"shroudforge_prepare_full_startup"));
    using FullTake=bool (__cdecl*)(const wchar_t*,SelectedStartup::PlanV2*,unsigned);
    using FullCommit=bool (__cdecl*)(const wchar_t*,const SelectedStartup::PlanV2*,unsigned);
    const auto take_full=reinterpret_cast<FullTake>(GetProcAddress(runtime,"shroudforge_take_full_startup_v2"));
    const auto release_ini=reinterpret_cast<FullCommit>(GetProcAddress(runtime,"shroudforge_release_vein_ini_v2"));
    const auto commit_full=reinterpret_cast<FullCommit>(GetProcAddress(runtime,"shroudforge_commit_full_startup_v2"));
    const auto take_entry=reinterpret_cast<GreeneyeEntry::Take>(GetProcAddress(runtime,"shroudforge_take_entry_v1"));
    const auto load_entry=reinterpret_cast<GreeneyeEntry::Load>(GetProcAddress(runtime,"shroudforge_load_entry_v1"));
    const auto confirm_entry=reinterpret_cast<GreeneyeEntry::Confirm>(GetProcAddress(runtime,"shroudforge_confirm_entry_v1"));
    if(full_runtime&&(!prepare_full||!take_full||!release_ini||!commit_full||!take_entry||!load_entry||!confirm_entry)) {
        startup_failed(root,"full-startup-contract","Full runtime/bootstrap handshake exports are incomplete");return 1;
    }
    if (!prepare_startup || (!assets_only && (!create || !update || !destroy))) {
        startup_failed(root, "runtime-exports", "runtime exports are incomplete");
        FreeLibrary(runtime);
        return 1;
    }
    startup_stage(root, "runtime-exports-resolved");
    if (!prepared_installation) {
        start_modloader_ui(root);
        start_world_editor_ui(root);
    }
    startup_stage(root, "startup-assets");
    if (!(full_runtime?prepare_full:prepare_startup)(root.c_str())) {
        startup_failed(root, "startup-assets", "Asset preparation and baseline recovery failed. Game startup stopped before loading inconsistent assets");
        if(!full_runtime)FreeLibrary(runtime);
        return 1;
    }
    if (assets_only) {
        // The reviewed recipe verified these existing loader bytes before
        // preparation. Activate them explicitly outside DllMain/loader lock;
        // do not assume file presence or system dbghelp happens to load them.
        using LoaderDemand = unsigned (__cdecl*)(const wchar_t*);
        const auto loader_demand = reinterpret_cast<LoaderDemand>(GetProcAddress(runtime, "shroudforge_take_startup_loader_demand"));
        if (!loader_demand) {
            startup_failed(root, "preserved-native-loader", "Runtime/bootstrap source pair lacks the preserved-loader contract");
            return 1;
        }
        const unsigned declared = loader_demand(root.c_str());
        if ((declared != 0 && (declared & ~0x80000003u) != 0) ||
            (declared != 0 && (declared & 0x80000000u) == 0)) {
            startup_failed(root, "preserved-native-loader", "Preserved-loader demand is invalid");
            return 1;
        }
        const unsigned mask = declared == 0 ? 3u : declared & 3u;
        const wchar_t* names[] = {L"dbgcore.dll", L"winmm.dll"};
        const unsigned flags[] = {2u, 1u};
        for (unsigned i=0; i<2; ++i) {
            if ((mask & flags[i]) != 0 && !LoadLibraryW((root / names[i]).c_str())) {
                startup_failed(root, "preserved-native-loader", windows_error(GetLastError()));
                return 1;
            }
        }
        using ZoomDemand = unsigned (__cdecl*)(const wchar_t*);
        const auto zoom_demand = reinterpret_cast<ZoomDemand>(GetProcAddress(runtime, "shroudforge_take_startup_zoom_demand"));
        StartupZoom::Cap zoom_cap;
        const auto zoom_result = zoom_cap.Apply(zoom_demand ? zoom_demand(root.c_str()) : 2u);
        if (zoom_result == StartupZoom::Result::Unsafe) {
            const bool restored = zoom_cap.Rollback();
            startup_failed(root, "startup-zoom", restored ?
                "Extended Zoom write failed and was rolled back; startup stopped" :
                "Extended Zoom write or ownership uncertain; startup stopped");
            return 1;
        }
        if (zoom_result == StartupZoom::Result::Applied)
            log("Extended Zoom fifth level activated before game entrypoint release");
        else if (zoom_result == StartupZoom::Result::Disabled)
            log("Extended Zoom native cap disabled by verified current composition");
        else
            log('W', "Extended Zoom native cap unavailable; no executable change accepted");
        startup_stage(root, "current-assets-ready");
        log("Current-composition assets ready; preserved native loaders activated; ShroudForge native provider and ingame runtime remain inactive");
        zoom_cap.Commit();
        startup.release();
        // Keep runtime and existing-loader modules resident until process exit.
        // Their shutdown code is not safe to invent or unload from this worker.
        return 0;
    }
    StartupZoom::Cap full_zoom_cap;
    SelectedStartup::PlanV2 full_plan;
    if(full_runtime) {
        if(!take_full(root.c_str(),&full_plan,sizeof(full_plan))||!SelectedStartup::Valid(full_plan)||
                full_plan.process_role!=(is_dedicated_server_process()?SelectedStartup::DedicatedServer:SelectedStartup::Client)) {
            startup_failed(root,"full-startup-contract","Missing, duplicate or unsupported full runtime preparation receipt");return 1;
        }
        const auto fp_demand=reinterpret_cast<StartupMode>(GetProcAddress(runtime,"shroudforge_startup_first_person_demand"));
        const auto fp=fp_demand?fp_demand(root.c_str()):2u;
        if(fp>1u||fp!=unsigned((full_plan.services&SelectedStartup::FirstPerson)!=0)) {
            startup_failed(root,"first-person-demand","Full-runtime First Person demand is unavailable or differs from verified composition");return 1;
        }
        using ZoomDemand=unsigned (__cdecl*)(const wchar_t*);
        const auto zoom_demand=reinterpret_cast<ZoomDemand>(GetProcAddress(runtime,"shroudforge_take_startup_zoom_demand"));
        const auto zoom=zoom_demand?zoom_demand(root.c_str()):2u;
        const auto result=full_zoom_cap.Apply(zoom);
        if(zoom>1u||(zoom==1u&&result!=StartupZoom::Result::Applied)||result==StartupZoom::Result::Unsafe) {
            const bool restored=full_zoom_cap.Rollback();
            startup_failed(root,"startup-zoom",restored?"Required full-runtime zoom unavailable; startup stopped":"Full-runtime zoom ownership uncertain; startup stopped");return 1;
        }
    }
    // Install native hooks while the executable entrypoint is still parked.
    // Releasing it first races hook writes against short-lived startup threads.
    startup_stage(root, "native-provider-init");
    using OverlayDemand = unsigned (__cdecl*)(const wchar_t*);
    const auto overlay_demand = reinterpret_cast<OverlayDemand>(GetProcAddress(runtime, "shroudforge_startup_overlay_demand"));
    const auto demand = full_runtime ? unsigned((full_plan.services&SelectedStartup::Graphics)!=0) : overlay_demand ? overlay_demand(root.c_str()) : 2u;
    if (demand == 2) log('W', "Native overlay boot demand unavailable. Repair package configuration and restart before rendering");
    const auto held_proof=full_runtime?StartupGate::CaptureHeldEntryProof():EngineHooks::HeldEntryProof{};
    if(full_runtime&&(!held_proof.is_held||!held_proof.is_held(held_proof.parked_thread_id))) {
        full_zoom_cap.Rollback();startup_failed(root,"held-entry","Original entrypoint proof is no longer held");return 1;
    }
    unsigned native_completed{};
    std::wstring vein_ini;
    if(full_runtime&&(full_plan.services&SelectedStartup::Vein))
        vein_ini=(root/L"mods"/L"shroudweave-vein-mining-test.ini").wstring();
    const bool native_prepared=full_runtime ?
        EcsRuntime::PrepareSelected(full_plan,held_proof,vein_ini.empty()?nullptr:vein_ini.c_str(),
            static_cast<unsigned>(vein_ini.size()),native_completed) : EcsRuntime::Initialize(demand==1);
    if (!native_prepared) {
        if(full_runtime) {
            EcsRuntime::Shutdown();
            const bool restored=full_zoom_cap.Rollback();
            startup_failed(root,"native-provider-init",EcsRuntime::Status()+(restored?"; owned zoom restored":"; zoom ownership uncertain"));return 1;
        }
        log('W', "KFC Runtime initialization failed: " + EcsRuntime::Status());
    }
    if(full_runtime) {
        // Native preparation has actually consumed the still-leased INI, with
        // Vein disabled. Only this one-shot release permits persistence next.
        if(!release_ini(root.c_str(),&full_plan,native_completed)||
                !EcsRuntime::ActivateSelected(full_plan,native_completed)||
                !commit_full(root.c_str(),&full_plan,native_completed)) {
            EcsRuntime::Shutdown();full_zoom_cap.Rollback();
            startup_failed(root,"full-startup-contract","Full runtime commit refused");return 1;
        }
    }
    bool entry_continuation_armed=false;
    if(full_runtime) {
        GreeneyeEntry::TicketV1 ticket;
        const auto pending=take_entry(root.c_str(),&ticket,sizeof(ticket));
        if(pending<0||pending>1||(pending==1&&!GreeneyeEntry::Valid(ticket))) {
            EcsRuntime::Shutdown();startup_failed(root,"entry-continuation","Original post-restore work could not be taken");return 1;
        }
        if(pending==1) {
            // PIN is conservative process residency, including rejected arming.
            HMODULE retained{};
            if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,
                    reinterpret_cast<LPCWSTR>(runtime),&retained)||retained!=runtime) {
                EcsRuntime::Shutdown();startup_failed(root,"entry-continuation","Runtime callback ownership could not be retained");return 1;
            }
            GreeneyeEntry::Context* context{};
            try {context=new GreeneyeEntry::Context{root.wstring(),ticket,load_entry,confirm_entry};}
            catch(...) {EcsRuntime::Shutdown();startup_failed(root,"entry-continuation","Retained continuation allocation failed");return 1;}
            if(!StartupGate::ArmPostRestore(held_proof.parked_thread_id,&GreeneyeEntry::Continue,context)) {
                delete context; // Never published; runtime PIN intentionally remains.
                EcsRuntime::Shutdown();startup_failed(root,"entry-continuation","Original held gate could not arm its continuation");return 1;
            }
            entry_continuation_armed=true; // Context/events/modules remain resident.
        }
    }
    if(full_runtime)full_zoom_cap.Commit();
    log("Startup assets and native-provider setup complete. Releasing the game entrypoint");
    if(!startup.release())return 1;
    if(entry_continuation_armed&&!StartupGate::AwaitPostRestore()) {
        EcsRuntime::Shutdown();startup_failed(root,"entry-continuation","Greeneye load or exact entry hook witness failed");return 1;
    }
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
        if(!full_runtime)FreeLibrary(runtime);
        return 1;
    }
    startup_stage(root, "runtime-ready");
    log("Runtime initialized");
    auto previous = std::chrono::steady_clock::now();
    auto next_runtime_status = previous;
    std::string previous_runtime_status;
    // The heartbeat is written by a thread of its own: its flush to the disk
    // held every mod up to half a second while the game loads. updatedAt
    // stays the loop's last turn, so a stalled loop still reads as inactive.
    std::atomic<long long> last_turn{seconds_now()};
    const HANDLE heartbeat_stop = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    std::thread heartbeat;
    if (heartbeat_stop) {
        try {
            heartbeat = std::thread([&] {
                do write_runtime_heartbeat(root, last_turn.load(std::memory_order_relaxed));
                while (WaitForSingleObject(heartbeat_stop, 2000) == WAIT_TIMEOUT);
            });
        } catch (...) {
            log('W', "Could not start the heartbeat thread: the mods loop writes it");
        }
    }
    const bool heartbeat_thread = heartbeat.joinable();
    while (WaitForSingleObject(stop_event, 16) == WAIT_TIMEOUT) {
        const auto now = std::chrono::steady_clock::now();
        const auto delta = std::chrono::duration<double>(now - previous).count();
        previous = now;
        last_turn.store(seconds_now(), std::memory_order_relaxed);
        EcsRuntime::Tick();
        if (now >= next_runtime_status) {
            if (!heartbeat_thread) write_runtime_heartbeat(root, last_turn.load(std::memory_order_relaxed));
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
    if (heartbeat_stop) SetEvent(heartbeat_stop);
    if (heartbeat.joinable()) heartbeat.join();
    if (heartbeat_stop) CloseHandle(heartbeat_stop);
    destroy(handle);
    EcsRuntime::Shutdown();
    stop_world_editor_ui();
    stop_modloader_ui();
    stop_debug_console();
    clear_runtime_heartbeat(root);
    if(!full_runtime)FreeLibrary(runtime);
    log("Runtime stopped");
    if(!prepared_installation)start_pending_update(root);
    return 0;
}
}
