#include <windows.h>
static unsigned count;
extern "C" __declspec(dllexport) unsigned WINAPI FixtureCount(){return count;}
extern "C" __declspec(dllexport) HRESULT WINAPI DirectInput8Create(HINSTANCE instance,DWORD version,const void* iid,void** output,void* outer){
    if(instance!=reinterpret_cast<HINSTANCE>(0x9876)||version!=42||iid!=reinterpret_cast<void*>(0x1234)||outer!=reinterpret_cast<void*>(0x5678)||!output)return E_INVALIDARG;
    *output=reinterpret_cast<void*>(static_cast<ULONG_PTR>(++count));return 0x12345678;
}
