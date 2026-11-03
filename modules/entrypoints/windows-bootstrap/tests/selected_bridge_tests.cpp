#include "../runtime_bridge.h"
#include "../../../game-runtime/native/engine-hooks/held_entry_preparation.h"
#include <windows.h>
#include <cassert>
#include <string>
#include <iostream>
#include <algorithm>
namespace Fixture {
int scenario{},prepare_calls{},activate_calls{},legacy_calls{},shutdown_calls{},free_calls{};
unsigned __cdecl Abi(){return scenario==6?13u:12u;}
bool __cdecl Legacy(){++legacy_calls;return true;}
void __cdecl Tick(){}
void __cdecl Shutdown(){++shutdown_calls;}
void __cdecl Status(char* output,std::size_t size){if(size)output[0]=0;}
bool __cdecl Prepare(const SelectedStartup::PlanV2* plan,const EngineHooks::HeldEntryProof*,const wchar_t*,unsigned,unsigned* completed){
    ++prepare_calls;*completed=scenario==3?0u:SelectedStartup::PreparedServices(*plan);return true;
}
bool __cdecl Activate(const SelectedStartup::PlanV2* plan,unsigned* completed){++activate_calls;*completed=plan->services|(scenario==4?256u:0u);return true;}
BOOL WINAPI Module(DWORD,LPCWSTR,HMODULE* output){*output=reinterpret_cast<HMODULE>(1);return TRUE;}
DWORD WINAPI ModulePath(HMODULE,LPWSTR output,DWORD capacity){const std::wstring path=L"E:\\owned-fixture\\winmm.dll";assert(path.size()+1<capacity);std::copy(path.begin(),path.end(),output);output[path.size()]=0;return static_cast<DWORD>(path.size());}
HMODULE WINAPI Load(LPCWSTR,HANDLE,DWORD){return reinterpret_cast<HMODULE>(2);}
BOOL WINAPI Free(HMODULE){++free_calls;return TRUE;}
FARPROC WINAPI Proc(HMODULE,LPCSTR name){
    const std::string n=name;
    if(n=="KfcRuntimeAbi")return reinterpret_cast<FARPROC>(&Abi);
    if(n=="KfcRuntimeInitialize")return reinterpret_cast<FARPROC>(&Legacy);
    if(n=="KfcRuntimeTick")return reinterpret_cast<FARPROC>(&Tick);
    if(n=="KfcRuntimeShutdown")return reinterpret_cast<FARPROC>(&Shutdown);
    if(n=="KfcRuntimeStatus")return reinterpret_cast<FARPROC>(&Status);
    if(n=="KfcRuntimePrepareSelectedV1"&&scenario!=1)return reinterpret_cast<FARPROC>(&Prepare);
    if(n=="KfcRuntimeActivateSelectedV1"&&scenario!=2)return reinterpret_cast<FARPROC>(&Activate);
    return nullptr;
}
}
#define GetModuleHandleExW Fixture::Module
#define GetModuleFileNameW Fixture::ModulePath
#define LoadLibraryExW Fixture::Load
#define FreeLibrary Fixture::Free
#define GetProcAddress Fixture::Proc
#include "../runtime_bridge.cpp"
#undef GetModuleHandleExW
#undef GetModuleFileNameW
#undef LoadLibraryExW
#undef FreeLibrary
#undef GetProcAddress
int main(int argc,char** argv){
    assert(argc==2);Fixture::scenario=std::stoi(argv[1]);
    SelectedStartup::PlanV2 plan;EngineHooks::HeldEntryProof proof;unsigned completed{};
    const bool prepared=EcsRuntime::PrepareSelected(plan,proof,nullptr,0,completed);
    assert(Fixture::legacy_calls==0); // Never fall back to generic initialization.
    if(Fixture::scenario==1||Fixture::scenario==2||Fixture::scenario==6){
        assert(!prepared&&Fixture::prepare_calls==0);assert(!EcsRuntime::ActivateSelected(plan,completed));
        assert(Fixture::activate_calls==0);
        if(Fixture::scenario==6)assert(Fixture::free_calls==1);else assert(Fixture::free_calls==0);
    }else if(Fixture::scenario==3){
        assert(!prepared&&Fixture::prepare_calls==1);assert(!EcsRuntime::ActivateSelected(plan,completed));
        assert(Fixture::activate_calls==0);
    }else{
        assert(prepared&&completed==SelectedStartup::Metadata);
        const bool activated=EcsRuntime::ActivateSelected(plan,completed);
        assert(activated==(Fixture::scenario==5));
        assert(!EcsRuntime::ActivateSelected(plan,completed)&&Fixture::activate_calls==1);
        assert(Fixture::free_calls==0);EcsRuntime::Shutdown();assert(Fixture::shutdown_calls==1);
    }
    std::cout<<"selected provider bridge scenario "<<Fixture::scenario<<" passed without DLL loading or legacy fallback\n";
}
