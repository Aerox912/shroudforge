// Invoke the actual native waypoint callback with only private fixture memory.
#include "map/capture_game_map.cpp"
#include <cstdio>
#include <cstdlib>
namespace {
unsigned checks{};std::uintptr_t waypoint{};std::uint32_t local_record[2]{0,73};std::uint8_t empty_pings[0x50]{};
std::uintptr_t denied_begin{},denied_end{};unsigned denied_reads{};
void check(bool value){++checks;if(!value){std::fprintf(stderr,"waypoint callback check %u failed\n",checks);std::exit(1);}}
LONG CALLBACK ObserveRead(EXCEPTION_POINTERS* fault){
    if(fault->ExceptionRecord->ExceptionCode==EXCEPTION_ACCESS_VIOLATION && fault->ExceptionRecord->NumberParameters>=2){
        const auto address=fault->ExceptionRecord->ExceptionInformation[1];if(address>=denied_begin && address<denied_end)++denied_reads;}
    return EXCEPTION_CONTINUE_SEARCH;
}
void __fastcall Initialize(void*,void* output,std::uint32_t size){std::uintptr_t record[6]{};record[1]=reinterpret_cast<std::uintptr_t>(local_record);record[3]=1;record[4]=record[5]=reinterpret_cast<std::uintptr_t>(empty_pings);std::memcpy(output,record,size);}
std::uintptr_t __fastcall World(std::uintptr_t){return 1;}
void __fastcall FixtureLookup(std::uintptr_t* output,std::uintptr_t,std::uintptr_t,std::uint32_t){output[0]=waypoint;}
void Capture(){std::uintptr_t context[2]{1,0};MapRuntime::waypoints_callback(context,nullptr,nullptr,nullptr);}
void AssertWaypoint(){auto lease=MapRuntime::store.Acquire(9,GetTickCount64());const auto& view=lease.frame.waypoints;
    check(view.meta.status==SF_MAP_READY && view.count==1);const auto& marker=view.data[0];
    check(marker.id==73 && marker.kind==10 && marker.icon_candidate_count==0);
    check(marker.position[0]==300 && marker.position[1]==100 && marker.position[2]==400);
}
}
int main(){using namespace MapRuntime;
    iter_init=Initialize;get_world=World;lookup=FixtureLookup;accepting=true;capture_generation=9;
    store.SetAdmission(SF_MAP_FEATURE_MARKERS);store.SetSession(9,true,false,true,GetTickCount64());
    auto* pages=static_cast<std::uint8_t*>(VirtualAlloc(nullptr,8192,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE));check(pages!=nullptr);
    DWORD old{};check(VirtualProtect(pages+4096,4096,PAGE_NOACCESS,&old)!=FALSE);denied_begin=reinterpret_cast<std::uintptr_t>(pages+4096);denied_end=denied_begin+4096;
    waypoint=reinterpret_cast<std::uintptr_t>(pages+4096-32);*reinterpret_cast<std::uint8_t*>(waypoint)=1;
    const std::int64_t xyz[3]{INT64_C(300)*4294967296,INT64_C(100)*4294967296,INT64_C(400)*4294967296};std::memcpy(reinterpret_cast<void*>(waypoint+8),xyz,sizeof(xyz));
    void* handler=AddVectoredExceptionHandler(1,ObserveRead);check(handler!=nullptr);Capture();
    check(denied_reads==0);AssertWaypoint();RemoveVectoredExceptionHandler(handler);VirtualFree(pages,0,MEM_RELEASE);
    // Readable adjacent bytes must not be mislabeled as icon identity either.
    std::uint8_t readable[48]{};readable[0]=1;std::memcpy(readable+8,xyz,sizeof(xyz));std::memset(readable+32,0xa5,16);waypoint=reinterpret_cast<std::uintptr_t>(readable);Capture();AssertWaypoint();
    readable[0]=0;Capture();auto empty=store.Acquire(9,GetTickCount64());check(empty.frame.waypoints.meta.status==SF_MAP_READY && empty.frame.waypoints.count==0);
    accepting=false;std::printf("PASS: %u actual waypoint callback XYZ-only checks\n",checks);
}
