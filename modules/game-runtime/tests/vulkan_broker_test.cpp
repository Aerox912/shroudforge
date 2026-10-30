// Actual host broker + renderer, entirely offline with fake Vulkan dispatch.
#include "graphics/vulkan_overlay.cpp"
#include <cstdio>
#include <cstdlib>
using namespace OverlayRuntime;
namespace {
unsigned checks{},instance_calls{},device_calls{},destroy_calls{};bool supports=true,fail_optional{},fail_all{},instance_support=true,khr_device=true;
const void* expected_chain{};const void* expected_allocator=reinterpret_cast<void*>(33);
void Check(bool x){++checks;if(!x){std::fprintf(stderr,"broker check %u failed\n",checks);std::exit(1);}}
std::vector<const char*> observed_instance_names,observed_device_names;
const InstanceCreateInfo* exact_instance{};const DeviceCreateInfo* exact_device{};
std::int32_t __fastcall InstanceExtensions(const char*,std::uint32_t* count,ExtensionProperties* out){
    if(!instance_support){*count=0;return 0;}
    const char* names[]{"VK_KHR_surface","VK_KHR_get_surface_capabilities2","VK_KHR_surface_maintenance1","VK_EXT_surface_maintenance1","VK_KHR_get_physical_device_properties2"};
    if(!out){*count=5;return 0;}for(unsigned i=0;i<5;++i)strcpy_s(out[i].name,names[i]);*count=5;return 0;
}
std::int32_t __fastcall DeviceExtensions(void*,const char*,std::uint32_t* count,ExtensionProperties* out){
    if(!out){*count=khr_device?2:1;return 0;}strcpy_s(out[0].name,maintenance_names[khr_device?0:1]);if(khr_device)strcpy_s(out[1].name,maintenance_names[1]);*count=khr_device?2:1;return 0;
}
void __fastcall FeatureQuery(void*,PhysicalFeatures2* query){Check(query->sType==1000059000);static_cast<MaintenanceFeatures*>(query->pNext)->enabled=supports;}
std::int32_t __fastcall CreateInstanceFake(const InstanceCreateInfo* info,const void* allocator,void** out){
    ++instance_calls;Check(allocator==expected_allocator && info->pNext==expected_chain);observed_instance_names.assign(info->extensions,info->extensions+info->extensionCount);
    if(fail_all || (fail_optional && info!=exact_instance))return -7;*out=reinterpret_cast<void*>(101);return 0;
}
std::int32_t __fastcall EnumerateFake(void* instance,std::uint32_t* n,void** p){Check(instance==reinterpret_cast<void*>(101));if(p)p[0]=reinterpret_cast<void*>(202);*n=1;return 0;}
std::int32_t __fastcall GroupsFake(void*,std::uint32_t* n,PhysicalDeviceGroup* p){if(p){p[0].count=1;p[0].physical[0]=reinterpret_cast<void*>(203);}*n=1;return 0;}
std::int32_t __fastcall CreateDeviceFake(void* physical,const DeviceCreateInfo* info,const void* allocator,void** out){
    ++device_calls;Check(physical==reinterpret_cast<void*>(202) && allocator==expected_allocator);
    observed_device_names.assign(info->extensions,info->extensions+info->extensionCount);
    const BaseIn* feature{};const bool chain_ok=Chain(info->pNext,MaintenanceFeatureType,feature);
    if(info!=exact_device)Check(chain_ok && feature && reinterpret_cast<const MaintenanceFeatures*>(feature)->enabled && feature->pNext==expected_chain);
    if(fail_all || (fail_optional && info!=exact_device))return -8;*out=reinterpret_cast<void*>(303);return 0;
}
void __fastcall DestroyFake(void*,const void*){++destroy_calls;}
void __fastcall GetQueueFake(void*,std::uint32_t family,std::uint32_t index,void** out){Check(family==4 && index==2);*out=reinterpret_cast<void*>(404);}
void __fastcall GetQueue2Fake(void*,const DeviceQueueInfo2* info,void** out){Check(info->family==5 && info->index==3);*out=reinterpret_cast<void*>(405);}
std::int32_t __fastcall WaitFake(void*,std::uint32_t,void*const*,std::uint32_t,std::uint64_t){return 0;}
std::int32_t __fastcall SwapFake(void*,const VkSwapchainCreateInfoKHR*,const void*,void** out){*out=reinterpret_cast<void*>(606);return 0;}
std::int32_t __fastcall ImagesFake(void*,void*,std::uint32_t*,void*){return -1;}
std::int32_t __fastcall PresentFake(void*,const void*){return 0;}
void __fastcall DestroySwapFake(void*,void*,const void*){}
void* __fastcall Gdpa(void*,const char* name) {
#define RETURN(n,f) if(!std::strcmp(name,n))return reinterpret_cast<void*>(&f)
    RETURN("vkGetDeviceProcAddr",Gdpa);RETURN("vkCreateSwapchainKHR",SwapFake);RETURN("vkGetSwapchainImagesKHR",ImagesFake);RETURN("vkQueuePresentKHR",PresentFake);
    RETURN("vkDestroyDevice",DestroyFake);RETURN("vkDestroySwapchainKHR",DestroySwapFake);RETURN("vkWaitForFences",WaitFake);RETURN("vkGetDeviceQueue",GetQueueFake);RETURN("vkGetDeviceQueue2",GetQueue2Fake);
    return reinterpret_cast<void*>(&PresentFake);
}
void* __fastcall Gipa(void*,const char* name) {
    RETURN("vkGetInstanceProcAddr",Gipa);RETURN("vkGetDeviceProcAddr",Gdpa);RETURN("vkCreateInstance",CreateInstanceFake);RETURN("vkCreateDevice",CreateDeviceFake);
    RETURN("vkEnumerateInstanceExtensionProperties",InstanceExtensions);RETURN("vkEnumerateDeviceExtensionProperties",DeviceExtensions);
    RETURN("vkGetPhysicalDeviceFeatures2",FeatureQuery);RETURN("vkGetPhysicalDeviceFeatures2KHR",FeatureQuery);RETURN("vkEnumeratePhysicalDevices",EnumerateFake);
    RETURN("vkEnumeratePhysicalDeviceGroups",GroupsFake);RETURN("vkEnumeratePhysicalDeviceGroupsKHR",GroupsFake);RETURN("vkEnumeratePhysicalDeviceGroupsKHR",GroupsFake);RETURN("vkDestroyInstance",DestroyFake);
    return reinterpret_cast<void*>(&PresentFake);
#undef RETURN
}
FARPROC WINAPI WindowsResolver(HMODULE,LPCSTR name){if(reinterpret_cast<std::uintptr_t>(name)<=65535)return reinterpret_cast<FARPROC>(55);return reinterpret_cast<FARPROC>(&PresentFake);}
void ImportChecks(){
    std::vector<std::uint8_t> pe(4096);auto* dos=reinterpret_cast<IMAGE_DOS_HEADER*>(pe.data());dos->e_magic=IMAGE_DOS_SIGNATURE;dos->e_lfanew=128;
    auto* nt=reinterpret_cast<IMAGE_NT_HEADERS64*>(pe.data()+128);nt->Signature=IMAGE_NT_SIGNATURE;nt->FileHeader.Machine=IMAGE_FILE_MACHINE_AMD64;
    nt->OptionalHeader.Magic=IMAGE_NT_OPTIONAL_HDR64_MAGIC;nt->OptionalHeader.SizeOfImage=4096;nt->OptionalHeader.NumberOfRvaAndSizes=16;
    nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT]={512,40};auto* d=reinterpret_cast<IMAGE_IMPORT_DESCRIPTOR*>(pe.data()+512);
    d->Name=600;d->OriginalFirstThunk=640;d->FirstThunk=704;strcpy_s(reinterpret_cast<char*>(pe.data()+600),13,"KERNEL32.dll");
    auto* thunk=reinterpret_cast<IMAGE_THUNK_DATA64*>(pe.data()+640);thunk[0].u1.AddressOfData=800;thunk[1].u1.Ordinal=IMAGE_ORDINAL_FLAG64|7;
    strcpy_s(reinterpret_cast<char*>(pe.data()+802),15,"GetProcAddress");auto* cell=reinterpret_cast<std::uintptr_t*>(pe.data()+704);*cell=99;
    Check(EngineHooks::FindNamedImport(pe,"kernel32.DLL","GetProcAddress")==cell);
    Check(!EngineHooks::FindNamedImport(pe,"kernel32.dll","missing"));
    Check(EngineHooks::ReplaceImportPointer(cell,99,100) && *cell==100);
    Check(!EngineHooks::ReplaceImportPointer(cell,99,101) && *cell==100);
    Check(EngineHooks::ReplaceImportPointer(cell,100,99));
    thunk[1].u1.AddressOfData=800;cell[1]=98;Check(!EngineHooks::FindNamedImport(pe,"kernel32.dll","GetProcAddress"));thunk[1].u1.Ordinal=IMAGE_ORDINAL_FLAG64|7;
    d->OriginalFirstThunk=4094;Check(!EngineHooks::FindNamedImport(pe,"kernel32.dll","GetProcAddress"));d->OriginalFirstThunk=640;
    thunk[0].u1.AddressOfData=UINT64_MAX;Check(!EngineHooks::FindNamedImport(pe,"kernel32.dll","GetProcAddress"));thunk[0].u1.AddressOfData=800;
    nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].Size=20;Check(!EngineHooks::FindNamedImport(pe,"kernel32.dll","GetProcAddress"));
}
}
int main(){
    ImportChecks();boot_demand=true;boot_gipa=Gipa;boot_gdpa=Gdpa;boot_create_instance=CreateInstanceFake;boot_create_device=CreateDeviceFake;
    boot_get_proc=WindowsResolver;
    Check(BrokerGetProc(GetModuleHandleW(nullptr),reinterpret_cast<LPCSTR>(17))==reinterpret_cast<FARPROC>(55));
    Check(BrokerGetProc(GetModuleHandleW(nullptr),"vkCreateDevice")==reinterpret_cast<FARPROC>(&PresentFake));
    Check(BrokerGipa(nullptr,"unknown")==reinterpret_cast<void*>(&PresentFake));
    Check(BrokerGipa(nullptr,"vkGetInstanceProcAddr")==reinterpret_cast<void*>(&BrokerGipa));
    Check(BrokerGipa(nullptr,"vkGetDeviceProcAddr")==reinterpret_cast<void*>(&BrokerGdpa));
    BaseIn unknown{123,nullptr};expected_chain=&unknown;ApplicationInfo app{};app.apiVersion=0x401000;
    const char* original_names[]{"VK_KHR_surface"};InstanceCreateInfo instance{};instance.applicationInfo=&app;instance.pNext=&unknown;instance.extensionCount=1;instance.extensions=original_names;
    const auto original_instance=instance;exact_instance=&instance;void* created{};
    const auto resolved_create_instance=reinterpret_cast<CreateInstanceFn>(BrokerGipa(nullptr,"vkCreateInstance"));
    Check(resolved_create_instance==&BrokerCreateInstance && resolved_create_instance(&instance,expected_allocator,&created)==0 && created==reinterpret_cast<void*>(101));
    Check(std::memcmp(&instance,&original_instance,sizeof(instance))==0 && unknown.pNext==nullptr && original_names[0]==instance.extensions[0]);
    Check(HasName(observed_instance_names,surface_names[0]) && HasName(observed_instance_names,surface_names[1]));
    Check(boot_instances.at(created).families==3);
    std::uint32_t n=1;void* physical{};auto enumerate=reinterpret_cast<EnumeratePhysicalFn>(BrokerGipa(created,"vkEnumeratePhysicalDevices"));
    Check(enumerate(created,&n,&physical)==0 && boot_physical.at(physical)==created);
    PhysicalDeviceGroup group{};n=1;Check(BrokerGroups(created,&n,&group)==0 && boot_physical.at(group.physical[0])==created);
    const auto groups_khr=reinterpret_cast<EnumerateGroupsFn>(BrokerGipa(created,"vkEnumeratePhysicalDeviceGroupsKHR"));n=1;
    Check(groups_khr==&BrokerGroupsKHR && groups_khr(created,&n,&group)==0 && boot_physical.at(group.physical[0])==created);
    const char* device_names[]{"VK_KHR_swapchain"};DeviceCreateInfo device{};device.pNext=&unknown;device.extensionCount=1;device.extensions=device_names;exact_device=&device;
    const auto original_device=device;void* handle{};
    const auto resolved_create_device=reinterpret_cast<CreateDeviceFn>(BrokerGipa(created,"vkCreateDevice"));
    Check(resolved_create_device==&BrokerCreateDevice && resolved_create_device(physical,&device,expected_allocator,&handle)==0);
    Check(std::memcmp(&device,&original_device,sizeof(device))==0 && unknown.pNext==nullptr && device.extensions==device_names);
    Check(FindDevice(handle)->maintenance && FindDevice(handle)->broker && HasName(observed_device_names,maintenance_names[0]));
    void* queue{};auto get_queue=reinterpret_cast<GetDeviceQueue>(BrokerGdpa(handle,"vkGetDeviceQueue"));get_queue(handle,4,2,&queue);
    Check(boot_queues.at(queue).first==reinterpret_cast<std::uintptr_t>(handle) && boot_queues.at(queue).second==4);
    DeviceQueueInfo2 queue_info{};queue_info.family=5;queue_info.index=3;reinterpret_cast<void(__fastcall*)(void*,const DeviceQueueInfo2*,void**)>(BrokerGdpa(handle,"vkGetDeviceQueue2"))(handle,&queue_info,&queue);Check(boot_queues.at(queue).second==5);
    Check(BrokerGdpa(handle,"vkQueuePresentKHR")==reinterpret_cast<void*>(&Present));
    Check(BrokerGdpa(reinterpret_cast<void*>(999),"vkQueuePresentKHR")==reinterpret_cast<void*>(&PresentFake));
    VkSwapchainCreateInfoKHR initial{};initial.imageExtent={1280,720};initial.imageFormat=97;initial.imageColorSpace=1000104008;initial.pNext=&unknown;
    void* chain{};const auto create_swap=reinterpret_cast<CreateSwapchainFn>(BrokerGdpa(handle,"vkCreateSwapchainKHR"));
    Check(create_swap(handle,&initial,expected_allocator,&chain)==0 && swapchains.at(606).width==1280 && swapchains.at(606).format==97 && swapchains.at(606).colorSpace==1000104008 && initial.pNext==&unknown);
    DestroySwap(handle,chain,nullptr);Check(!swapchains.contains(606));
    Destroy(handle,nullptr);Check(!FindDevice(handle) && boot_queues.empty());
    khr_device=false;Check(BrokerCreateDevice(physical,&device,expected_allocator,&handle)==0 && FindDevice(handle)->maintenance && HasName(observed_device_names,maintenance_names[1]));Destroy(handle,nullptr);khr_device=true;
    boot_physical.erase(physical);Check(BrokerCreateDevice(physical,&device,expected_allocator,&handle)==0 && !FindDevice(handle)->maintenance);Destroy(handle,nullptr);boot_physical[physical]=created;
    MaintenanceFeatures already_on;already_on.enabled=1;already_on.pNext=&unknown;device.pNext=&already_on;
    Check(BrokerCreateDevice(physical,&device,expected_allocator,&handle)==0 && FindDevice(handle)->maintenance && already_on.enabled==1 && already_on.pNext==&unknown);Destroy(handle,nullptr);device.pNext=&unknown;

    supports=false;Check(BrokerCreateDevice(physical,&device,expected_allocator,&handle)==0 && !FindDevice(handle)->maintenance);
    Check(BrokerGdpa(handle,"vkQueuePresentKHR")==reinterpret_cast<void*>(&PresentFake));Destroy(handle,nullptr);supports=true;
    MaintenanceFeatures explicit_off;explicit_off.pNext=&unknown;device.pNext=&explicit_off;exact_device=&device;
    Check(BrokerCreateDevice(physical,&device,expected_allocator,&handle)==0 && !FindDevice(handle)->maintenance && graphics_reason==SF_OVERLAY_REASON_FEATURE_CONFLICT);
    Check(explicit_off.enabled==0 && explicit_off.pNext==&unknown);Destroy(handle,nullptr);
    device.pNext=&unknown;fail_optional=true;const auto before_calls=device_calls;
    Check(BrokerCreateDevice(physical,&device,expected_allocator,&handle)==0 && !FindDevice(handle)->maintenance && device_calls==before_calls+2);Destroy(handle,nullptr);
    fail_all=true;Check(BrokerCreateDevice(physical,&device,expected_allocator,&handle)!=0 && !FindDevice(handle));fail_all=false;
    BrokerDestroyInstance(created,nullptr);
    const auto before_instance=instance_calls;Check(BrokerCreateInstance(&instance,expected_allocator,&created)==0 && boot_instances.at(created).families==0 && instance_calls==before_instance+2);
    fail_optional=false;BrokerDestroyInstance(created,nullptr);
    instance_support=false;Check(BrokerCreateInstance(&instance,expected_allocator,&created)==0 && boot_instances.at(created).families==0);BrokerDestroyInstance(created,nullptr);instance_support=true;
    app.apiVersion=0x400000;Check(BrokerCreateInstance(&instance,expected_allocator,&created)==0 && HasName(observed_instance_names,"VK_KHR_get_physical_device_properties2"));BrokerDestroyInstance(created,nullptr);
Check(boot_instances.empty() && boot_physical.empty());
    std::printf("PASS: %u actual Vulkan broker/import/provenance/negotiation checks\n",checks);
}
