#include "../bootstrap_state.h"
#include "../startup_zoom.h"
#include "../startup_admission.h"
#include <cassert>
#include <iostream>

namespace Test {
std::filesystem::path root;
unsigned mode=2,handshake=StartupAdmission::FullV1,zoom=0,loaders=0x80000000u;
unsigned first_person{};
bool prepare=true,commit=true,initialize=true,hooks=true,has_hooks=true,graphics=true,exports=true,create=true;
int loads{},frees{},legacy_prepare{},full_prepare{},updaters{},created{},updated{},destroyed{},rollbacks{},provider_calls{};
int stop_waits{};
StartupZoom::Result zoom_result=StartupZoom::Result::Disabled;
void Reset(){mode=2;handshake=StartupAdmission::FullV1;zoom=0;first_person=0;loaders=0x80000000u;StartupGate::direct_static_attachment=true;
    prepare=commit=initialize=hooks=has_hooks=graphics=exports=create=true;
    loads=frees=legacy_prepare=full_prepare=updaters=created=updated=destroyed=rollbacks=provider_calls=stop_waits=0;zoom_result=StartupZoom::Result::Disabled;}
bool __cdecl Prepare(const wchar_t*){++legacy_prepare;return prepare;}
bool __cdecl PrepareFull(const wchar_t*){++full_prepare;return prepare;}
unsigned __cdecl Mode(const wchar_t*){return mode;}
unsigned __cdecl Take(const wchar_t*){return handshake;}
unsigned __cdecl Zoom(const wchar_t*){return zoom;}
unsigned __cdecl Loaders(const wchar_t*){return loaders;}
unsigned __cdecl FirstPerson(const wchar_t*){return first_person;}
bool __cdecl Commit(const wchar_t*,unsigned){return commit;}
void* __cdecl Create(const wchar_t*){++created;assert(StartupGate::succeeded==1);return create?reinterpret_cast<void*>(1):nullptr;}
bool __cdecl Update(void*,double){++updated;return false;}
void __cdecl Destroy(void*){++destroyed;}
HMODULE WINAPI Load(LPCWSTR){++loads;return reinterpret_cast<HMODULE>(1);}
BOOL WINAPI Free(HMODULE){++frees;return TRUE;}
FARPROC WINAPI Proc(HMODULE,LPCSTR name){
    const std::string n=name;
    if(n=="shroudforge_prepare_startup")return reinterpret_cast<FARPROC>(&Prepare);
    if(n=="shroudforge_startup_mode")return reinterpret_cast<FARPROC>(&Mode);
    if(n=="shroudforge_create")return reinterpret_cast<FARPROC>(&Create);
    if(n=="shroudforge_update")return reinterpret_cast<FARPROC>(&Update);
    if(n=="shroudforge_destroy")return reinterpret_cast<FARPROC>(&Destroy);
    if(n=="shroudforge_take_startup_loader_demand")return reinterpret_cast<FARPROC>(&Loaders);
    if(n=="shroudforge_take_startup_zoom_demand")return reinterpret_cast<FARPROC>(&Zoom);
    if(n=="shroudforge_startup_overlay_demand")return reinterpret_cast<FARPROC>(&Zoom);
    if(n=="shroudforge_startup_first_person_demand")return reinterpret_cast<FARPROC>(&FirstPerson);
    if(!exports)return nullptr;
    if(n=="shroudforge_prepare_full_startup")return reinterpret_cast<FARPROC>(&PrepareFull);
    if(n=="shroudforge_take_full_startup")return reinterpret_cast<FARPROC>(&Take);
    if(n=="shroudforge_commit_full_startup")return reinterpret_cast<FARPROC>(&Commit);
    return nullptr;
}
DWORD WINAPI Wait(HANDLE event,DWORD){return event==BootstrapImplementation::stop_event?(stop_waits++?WAIT_OBJECT_0:WAIT_TIMEOUT):WAIT_OBJECT_0;}
DWORD WINAPI ModulePath(HMODULE,LPWSTR output,DWORD capacity){const auto path=(root/L"winmm.dll").wstring();assert(path.size()<capacity);std::copy(path.begin(),path.end(),output);output[path.size()]=0;return static_cast<DWORD>(path.size());}
}
namespace StartupGate {
EngineHooks::HeldEntryProof FixtureProof() noexcept {
    EngineHooks::HeldEntryProof p;p.parked_thread_id=GetCurrentThreadId()+1;
    p.is_held=+[](std::uint32_t id) noexcept{return id!=0;};return p;
}
}
#define LoadLibraryW Test::Load
#define FreeLibrary Test::Free
#define GetProcAddress Test::Proc
#define WaitForSingleObject Test::Wait
#define GetModuleFileNameW Test::ModulePath
#define CaptureHeldEntryProof FixtureProof
#include "../runtime_lifecycle.cpp"
#undef CaptureHeldEntryProof
#undef GetModuleFileNameW
#undef WaitForSingleObject
#undef GetProcAddress
#undef FreeLibrary
#undef LoadLibraryW

namespace StartupZoom {
Cap::~Cap(){if(!committed_)Rollback();}
Result Cap::Apply(unsigned){owned_=Test::zoom_result==Result::Applied;return Test::zoom_result;}
bool Cap::Rollback(){++Test::rollbacks;owned_=false;return true;}
}
namespace EcsRuntime {
bool Initialize(bool overlay,bool required,const EngineHooks::HeldEntryProof* proof,bool fp){++Test::provider_calls;
    if(required)assert(proof&&proof->is_held);if(fp)assert(Test::first_person==1);
    return required?StartupAdmission::RequiredReady(Test::initialize,Test::has_hooks,Test::hooks,overlay,Test::graphics):Test::initialize;}
void Tick(){}void Shutdown(){}std::string Status(){return "synthetic-provider-status";}
}
namespace BootstrapImplementation {
HMODULE self_module{};HANDLE stop_event=reinterpret_cast<HANDLE>(2),runtime_thread{},console_stop_event{},console_process{},console_show_event{},modloader_ui_stop_event{},modloader_ui_process{},world_editor_ui_stop_event{},world_editor_ui_process{};
std::chrono::steady_clock::time_point session_started;
std::filesystem::path module_directory(){return Test::root;}
std::filesystem::path shroudforge_directory(const std::filesystem::path& root){return root/"shroudforge";}
bool is_dedicated_server_process(){return false;}
void begin_log_session(const std::filesystem::path&){}void log(char,const std::string&){}void log(const std::string&){}
void log_shroudforge_banner(bool){}void startup_stage(const std::filesystem::path&,const char*){}
bool run_startup_update_check(const std::filesystem::path&){++Test::updaters;return true;}
void start_pending_update(const std::filesystem::path&){++Test::updaters;}
void startup_failed(const std::filesystem::path&,const char*,const std::string&){}
std::string windows_error(DWORD){return "synthetic";}std::string runtime_status_state(std::string_view){return "late-bound";}
void start_debug_console(const std::filesystem::path&){}void stop_debug_console(){}
void start_modloader_ui(const std::filesystem::path&){}void stop_modloader_ui(){}
void start_world_editor_ui(const std::filesystem::path&){}void stop_world_editor_ui(){}
}
int main(int argc,char** argv){
    assert(argc==2);Test::root=std::filesystem::absolute(argv[1]);
    std::filesystem::create_directories(Test::root/"shroudforge/config");
    std::ofstream(Test::root/L"winmm.dll")<<"synthetic host path only; never loaded";
    std::ofstream(Test::root/"shroudforge/config/current-assets.json")<<"synthetic marker only";
    StartupGate::completed=CreateEventW(nullptr,TRUE,FALSE,nullptr);assert(StartupGate::completed);
    StartupGate::entered=reinterpret_cast<HANDLE>(1);
    auto run=[] {ResetEvent(StartupGate::completed);StartupGate::succeeded=0;return BootstrapImplementation::run(nullptr);};
    Test::Reset();assert(run()==0);assert(Test::full_prepare==1&&Test::legacy_prepare==0&&Test::loads==1&&Test::frees==0);
    assert(Test::created==1&&Test::updated==1&&Test::destroyed==1&&Test::updaters==0&&StartupGate::succeeded==1);
    // Historical assets-only schemas all use mode1, including their old zero-demand fallback.
    for(int schema:{3,4,5}){(void)schema;Test::Reset();Test::mode=1;Test::loaders=0;assert(run()==0);
        assert(Test::legacy_prepare==1&&Test::full_prepare==0&&Test::loads==3&&Test::created==0&&Test::provider_calls==0&&Test::updaters==0);}
    for(int failure=0;failure<9;++failure){Test::Reset();
        if(failure==0)Test::exports=false;
        if(failure==1)Test::prepare=false;
        if(failure==2)Test::handshake=0;
        if(failure==3)Test::handshake=StartupAdmission::FullV1|4;
        if(failure==4)Test::initialize=false;
        if(failure==5)Test::hooks=false; // Initialize=true is insufficient.
        if(failure==6)Test::has_hooks=false;
        if(failure==7){Test::handshake|=1;Test::graphics=false;}
        if(failure==8){Test::zoom=1;Test::zoom_result=StartupZoom::Result::Unavailable;}
        assert(run()==1);assert(StartupGate::succeeded==0&&Test::created==0&&Test::legacy_prepare==0&&Test::updaters==0&&Test::frees==0);
    }
    Test::Reset();StartupGate::direct_static_attachment=false;assert(run()==1&&StartupGate::succeeded==0&&Test::created==0);
    Test::Reset();Test::first_person=2;assert(run()==1&&StartupGate::succeeded==0);
    Test::Reset();Test::first_person=1;assert(run()==1&&StartupGate::succeeded==0); // mismatched policy demand
    Test::Reset();Test::first_person=1;Test::handshake|=2;assert(run()==0&&Test::provider_calls==1);
    Test::Reset();Test::commit=false;Test::zoom=1;Test::zoom_result=StartupZoom::Result::Applied;
    assert(run()==1&&StartupGate::succeeded==0&&Test::rollbacks>0);
    Test::Reset();Test::create=false;assert(run()==1&&StartupGate::succeeded==1&&Test::updaters==0&&Test::frees==0);
    int completions=0;static int* count;count=&completions;
    {StartupAdmission::Completion completion(+[](bool success){assert(success);++*count;});assert(completion.release());assert(!completion.release());}
    assert(completions==1);CloseHandle(StartupGate::completed);
    std::cout<<"RB11 passed actual bootstrap run: full lifecycle, old assets-only modes, nine admission/readiness refusals, zoom rollback, post-release create failure, no updater/copy fallback, single completion; no World wait or real DLL execution\n";
}
