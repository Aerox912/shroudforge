#include "../startup_gate.h"
#include "zoom_fixture.h"
#include <fstream>
#include <iterator>
#include <filesystem>
using namespace StartupZoom;
static unsigned char* image;
static bool verified=false;
static unsigned expected=2;
std::wstring Environment(const wchar_t* key) {
    wchar_t text[32768]{}; const auto n=GetEnvironmentVariableW(key,text,32768);
    if(!n || n>=32768)return {};return std::wstring(text,n);
}
unsigned char* RetainedImage() {
    std::ifstream stream("E:/Build/esm-realmap-startup-r9-20261028/inputs/enshrouded.exe",std::ios::binary);
    if(!stream)return nullptr;
    std::vector<unsigned char> raw((std::istreambuf_iterator<char>(stream)),{});
    if(raw.size()!=35338312)return nullptr;
    const auto* dos=reinterpret_cast<const IMAGE_DOS_HEADER*>(raw.data());
    if(dos->e_magic!=IMAGE_DOS_SIGNATURE || dos->e_lfanew<=0 || dos->e_lfanew>0x1000)return nullptr;
    const auto* nt=reinterpret_cast<const IMAGE_NT_HEADERS64*>(raw.data()+dos->e_lfanew);
    if(nt->OptionalHeader.SizeOfImage!=Profile::ImageSize || nt->OptionalHeader.SizeOfHeaders>raw.size())return nullptr;
    auto* result=static_cast<unsigned char*>(VirtualAlloc(nullptr,Profile::ImageSize,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE));if(!result)return nullptr;
    std::memcpy(result,raw.data(),nt->OptionalHeader.SizeOfHeaders);const auto* s=IMAGE_FIRST_SECTION(nt);
    for(unsigned i=0;i<nt->FileHeader.NumberOfSections;++i) {
        if(std::uint64_t(s[i].PointerToRawData)+s[i].SizeOfRawData>raw.size() ||
            std::uint64_t(s[i].VirtualAddress)+s[i].SizeOfRawData>Profile::ImageSize)return nullptr;
        std::memcpy(result+s[i].VirtualAddress,raw.data()+s[i].PointerToRawData,s[i].SizeOfRawData);
    }
    ZoomFixture::Protect(result);return result;
}
DWORD WINAPI Prepare(void*) {
    if(WaitForSingleObject(StartupGate::entered,3000)!=WAIT_OBJECT_0) { StartupGate::Complete(false);return 1; }
    const auto root=Environment(L"SF_ZOOM_FIXTURE_ROOT");
    if(root.rfind(L"E:\\Build\\hdmap-zoom-layout-migration-20261029-r1\\fixtures",0)!=0) {StartupGate::Complete(false);return 1;}
    expected=Environment(L"SF_ZOOM_EXPECT_DEMAND")==L"1"?1u:0u;
    const auto runtime=LoadLibraryW(L"E:\\Build\\hdmap-zoom-layout-migration-20261029-r1\\artifacts\\shroudforge-runtime.dll");
    if(!runtime) {StartupGate::Complete(false);return 1;}
    using PrepareAssets=bool(__cdecl*)(const wchar_t*);using Demand=unsigned(__cdecl*)(const wchar_t*);
    const auto prepare=reinterpret_cast<PrepareAssets>(GetProcAddress(runtime,"shroudforge_prepare_startup"));
    const auto take=reinterpret_cast<Demand>(GetProcAddress(runtime,"shroudforge_take_startup_zoom_demand"));
    if(!prepare || !take || !prepare(root.c_str())) {StartupGate::Complete(false);return 1;}
    const auto demand=take(root.c_str());
    if(demand!=expected || take(root.c_str())!=2) {StartupGate::Complete(false);return 1;}
    image=RetainedImage();if(!image) {StartupGate::Complete(false);return 1;}
    Cap cap;
    const auto result=cap.TestApply(image,Profile::ImageSize,demand,
        WaitForSingleObject(StartupGate::entered,0)==WAIT_OBJECT_0 && WaitForSingleObject(StartupGate::completed,0)==WAIT_TIMEOUT,
        EngineHooks::ReplaceExecutableBytes);
    verified=result==(expected?Result::Applied:Result::Disabled) &&
        !std::memcmp(image+Profile::Signatures[0].rva,expected?Profile::Replacement:Profile::Original,4);
    if(!verified) {StartupGate::Complete(false);return 1;}
    cap.Commit();StartupGate::Complete(true);return 0;
}
extern "C" __declspec(dllexport) bool GateVerified() {
    return verified && WaitForSingleObject(StartupGate::completed,0)==WAIT_OBJECT_0 &&
        !std::memcmp(StartupGate::entrypoint,StartupGate::original,sizeof(StartupGate::original)) &&
        !std::memcmp(image+Profile::Signatures[0].rva,expected?Profile::Replacement:Profile::Original,4);
}
BOOL WINAPI DllMain(HINSTANCE,DWORD reason,LPVOID) {
    if(reason==DLL_PROCESS_ATTACH) { if(!StartupGate::Install())return FALSE;
        HANDLE worker=CreateThread(nullptr,0,Prepare,nullptr,0,nullptr);if(!worker)return FALSE;CloseHandle(worker); }
    return TRUE;
}
