#include "../startup_gate.h"
#include "zoom_fixture.h"
using namespace StartupZoom;
using namespace ZoomFixture;
static unsigned char* image;
static bool verified=false;
DWORD WINAPI Prepare(void*) {
    if(WaitForSingleObject(StartupGate::entered,3000)!=WAIT_OBJECT_0) { StartupGate::Complete(false); return 1; }
    image=Create();Protect(image);
    Cap cap;
    const bool fail=GetEnvironmentVariableW(L"SF_GATE_TEST_FAIL",nullptr,0)!=0;
    const bool conflict=GetEnvironmentVariableW(L"SF_ZOOM_CONFLICT",nullptr,0)!=0;
    const auto result=cap.TestApply(image,Profile::ImageSize,1,true,fail?Partial:EngineHooks::ReplaceExecutableBytes);
    if(result!=Result::Applied) { StartupGate::Complete(false);return 1; }
    if(conflict) { const unsigned char bad=6;Put(reinterpret_cast<std::uintptr_t>(image+Profile::Signatures[0].rva+3),&bad,1);
        verified=cap.Rollback();StartupGate::Complete(false);return 1; }
    verified=!std::memcmp(image+Profile::Signatures[0].rva,Profile::Replacement,4);
    Sleep(50); // main must remain parked for the entire preparation window
    cap.Commit();StartupGate::Complete(verified);return 0;
}
extern "C" __declspec(dllexport) bool GateVerified() {
    return verified && WaitForSingleObject(StartupGate::completed,0)==WAIT_OBJECT_0 &&
        !std::memcmp(StartupGate::entrypoint,StartupGate::original,sizeof(StartupGate::original)) &&
        !std::memcmp(image+Profile::Signatures[0].rva,Profile::Replacement,4);
}
BOOL WINAPI DllMain(HINSTANCE,DWORD reason,LPVOID) {
    if(reason==DLL_PROCESS_ATTACH) { if(!StartupGate::Install())return FALSE;
        HANDLE worker=CreateThread(nullptr,0,Prepare,nullptr,0,nullptr);if(!worker)return FALSE;CloseHandle(worker); }
    return TRUE;
}
