#define WriteExecutableBytes ComponentFixtureWriteExecutableBytes
#define ReplaceExecutableBytes ComponentFixtureReplaceExecutableBytes
#include "map/capture_game_map.cpp"
#include <cstdio>
namespace EngineHooks {
void* AllocateNearCode(std::uintptr_t,std::size_t){return nullptr;}
bool WriteExecutableBytes(std::uintptr_t,const void*,std::size_t,const void*){return false;}
bool ReplaceExecutableBytes(std::uintptr_t,const void*,const void*,std::size_t){return false;}
}
#undef WriteExecutableBytes
#undef ReplaceExecutableBytes
namespace {
unsigned checks{},failed{},initCalls{},nextCalls{};std::uint32_t lastSize{};
std::uintptr_t* inputContext{};bool privateArgs{},highBits{};
alignas(16) std::uint8_t query[0x900]{},p3[0x410]{},p8[0x90]{};
std::uint32_t local[2]{0,7},entityRows[4]{7};
MapRuntime::NativeTransform player{},camera{};
std::uintptr_t worldState{};unsigned nextMode{},weatherCalls{},fogCalls{},metaReads{},metaBytes{};
bool metadata(std::uintptr_t at,void* out,std::size_t size){++metaReads;metaBytes+=static_cast<unsigned>(size);return MapRuntime::ReadMemory(at,out,size);}
void Check(bool b,const char* text){++checks;if(!b){++failed;std::printf("FAIL %s\n",text);}}
template<class T>void Put(size_t offset,T v){std::memcpy(query+offset,&v,sizeof(v));}
void __fastcall Init(void* ctx,void* record,std::uint32_t size){
 ++initCalls;lastSize=size;privateArgs=ctx!=inputContext && std::memcmp(ctx,inputContext,16)==0;
 highBits=(*static_cast<std::uint64_t*>(static_cast<void*>(static_cast<std::uint8_t*>(ctx)+8))>>32)==0xaabbccdd;
 std::memset(record,0,size);
 if(size==80){MapRuntime::NativeCameraRecord r{};r.local_player=reinterpret_cast<std::uintptr_t>(local);r.player_transform=reinterpret_cast<std::uintptr_t>(&player);r.camera_transform=reinterpret_cast<std::uintptr_t>(&camera);std::memcpy(record,&r,sizeof(r));}
 else if(size==16)static_cast<std::uintptr_t*>(record)[1]=worldState;
 else if(size>=24)static_cast<std::uintptr_t*>(record)[size==152?2:1]=reinterpret_cast<std::uintptr_t>(local);
}
bool __fastcall Next(void* ctx,void*,std::uint32_t size){++nextCalls;lastSize=size;auto* cursor=reinterpret_cast<std::uint32_t*>(static_cast<std::uint8_t*>(ctx)+8);if(nextMode!=1)++*cursor;if(nextMode==2)static_cast<std::uint64_t*>(ctx)[0]+=8;highBits=highBits && (static_cast<std::uint64_t*>(ctx)[1]>>32)==0xaabbccdd;return nextMode!=3;}
float* __fastcall Weather(float* out,void*,const void*,std::int64_t){++weatherCalls;return out;}
void __fastcall Fog(void*,const float*,float){++fogCalls;}
void Reset(unsigned capacity,bool valid){
 std::memset(query,0,sizeof(query));std::memset(p3,0,sizeof(p3));std::memset(p8,0,sizeof(p8));
 Put(0xc0,reinterpret_cast<std::uintptr_t>(p3));Put(0xc8,reinterpret_cast<std::uintptr_t>(p8));
 query[0x4b0]=valid?std::uint8_t(capacity/8-1):255;Put<std::uint64_t>(0x838,1);
 Put(0x830,reinterpret_cast<std::uintptr_t>(entityRows));
 initCalls=nextCalls=0;privateArgs=highBits=false;
 MapRuntime::last_remote=MapRuntime::last_clock=MapRuntime::last_world=0;
}
}
int main(){using namespace MapRuntime;namespace Profile=KfcRuntimeCompatibility::EnshroudedClient;
 Profile::image_sha256="0403981fd2e8ad18bd8c5698dff03c8d505b5a46677832c496db9c645513f2a6";
 iter_init=Init;iter_next=Next;accepting=true;capture_generation=9;store.SetAdmission(SF_MAP_FEATURE_PLAYER);store.SetSession(9,true,false,true,GetTickCount64());
 player.quaternion[3]=camera.quaternion[3]=1;player.xyz[0]=camera.xyz[0]=INT64_C(300)*4294967296;player.xyz[1]=camera.xyz[1]=INT64_C(100)*4294967296;player.xyz[2]=camera.xyz[2]=INT64_C(400)*4294967296;
 using Callback=void(__fastcall*)(void*,void*,void*,void*);
 struct Case{Callback callback;unsigned capacity;};
 for(const auto& c:{Case{camera_callback,80},Case{remote_callback,152},Case{waypoints_callback,48},Case{clock_callback,40},Case{world_callback,16}}){
  Reset(c.capacity,false);std::uintptr_t ctx[]{reinterpret_cast<std::uintptr_t>(query),0xaabbccdd00000000};inputContext=ctx;c.callback(ctx,nullptr,nullptr,nullptr);Check(initCalls==0 && nextCalls==0,"bad count refuses every private fixed-buffer native call");
  Reset(c.capacity,true);c.callback(ctx,nullptr,nullptr,nullptr);Check(initCalls==1 && lastSize==c.capacity && privateArgs && highBits,"each callback uses exact capacity and private full context");Check(ctx[1]==0xaabbccdd00000000,"original callback context unchanged");
 }
 auto frame=store.Acquire(9,GetTickCount64()).frame;Check(frame.player.meta.status==SF_MAP_READY && frame.player.pose.position[0]==300,"camera selects current entity array +830 and publishes bounded pose");
 Reset(80,true);std::uintptr_t guarded[]{reinterpret_cast<std::uintptr_t>(query),0xaabbccdd00000000};inputContext=guarded;std::uint8_t scratch[160]{};
 metaReads=metaBytes=0;auto snapshot=NativeIteratorSnapshot::ReadPlan(guarded,scratch,80,metadata);Check(snapshot.valid&&metaReads==8&&metaBytes==64,"selected callback snapshot has fixed8 reads/64 bytes");
 initCalls=0;Check(!initialize(guarded,scratch,153)&&initCalls==0,"unqualified buffer above152 refuses before Init");
 Check(!initialize(guarded,query+0x100,80)&&initCalls==0,"query/destination alias refuses before Init");
 Put<std::uint64_t>(0x838,UINT64_C(0x100000000));Check(!initialize(guarded,scratch,80)&&initCalls==0,"qword row count beyond DWORD termination refuses");
 Reset(80,true);Put<std::uintptr_t>(0xc0,0);Check(!initialize(guarded,scratch,80)&&initCalls==0,"unknown indirect metadata refuses before native calls");
 for(unsigned mode:{0u,1u,2u,3u}){
  Reset(80,true);guarded[0]=reinterpret_cast<std::uintptr_t>(query);guarded[1]=0xaabbccdd00000000;NativeIteratorExtent::IteratorExtentPlan plan{};Check(initialize(guarded,scratch,80,&plan),"actual planner prepared normal private call");
  nextMode=mode;const bool result=advance(guarded,scratch,80,plan,256);Check(result==(mode==0)&&nextCalls==1,"actual Next requires DWORD progress/query identity/native true");
  if(mode==0){Check((guarded[1]>>32)==0xaabbccdd,"actual DWORD advance preserves high32");Check(!advance(guarded,scratch,80,plan,256)&&nextCalls==1,"actual exhausted plan makes zero extra Next calls");}
 }nextMode=0;
 std::vector<std::uint8_t> state(0x400000),newRows(0xa0),oldRows(0xa0);worldState=reinterpret_cast<std::uintptr_t>(state.data());
 const std::int64_t xyz[]{INT64_C(300)*4294967296,INT64_C(100)*4294967296,INT64_C(400)*4294967296};std::memcpy(newRows.data()+0x10,xyz,sizeof(xyz));std::memcpy(oldRows.data()+0x10,xyz,sizeof(xyz));
 std::uint32_t newKind=42,oldKind=17;std::memcpy(newRows.data()+0x30,&newKind,4);std::memcpy(oldRows.data()+0x30,&oldKind,4);
 auto list=[&](size_t offset,const auto& rows){auto ptr=reinterpret_cast<std::uintptr_t>(rows.data());std::uint64_t count=1;std::memcpy(state.data()+offset,&ptr,8);std::memcpy(state.data()+offset+8,&count,8);};list(0x3e6a78,newRows);list(0x3bea50,oldRows);
 store.SetAdmission(SF_MAP_FEATURE_WORLD|SF_MAP_FEATURE_DEATHS);store.SetSession(9,true,false,true,GetTickCount64());deaths_enabled=true;Reset(16,true);guarded[0]=reinterpret_cast<std::uintptr_t>(query);guarded[1]=0xaabbccdd00000000;inputContext=guarded;world_callback(guarded,nullptr,nullptr,nullptr);
 auto deathLease=store.Acquire(9,GetTickCount64());auto deaths=deathLease.frame.deaths;Check(deaths.count==1&&deaths.data[0].kind==42,"actual new-profile death selection ignores old cross-profile list");
 Check(deathLease.frame.world_markers.count==1&&deathLease.frame.world_markers.data[0].kind==17,"actual current secondary normal list remains ordinary world markers");
 Profile::image_sha256="d64966e23b1a458ee6f147bcbceac0b577fa7e39a212f371287d8cc119817882";
 Reset(16,false);std::uintptr_t old[]{reinterpret_cast<std::uintptr_t>(query),0xaabbccdd00000000};inputContext=old;world_callback(old,nullptr,nullptr,nullptr);Check(initCalls==1,"old profile private Init behavior preserved");
 deathLease=store.Acquire(9,GetTickCount64());deaths=deathLease.frame.deaths;Check(deaths.count==1&&deaths.data[0].kind==17,"actual legacy death selection remains separate");
 accepting=false;weather_sample=Weather;fog_update=Fog;float weights[]{1,0,0,0},position[3]{};Check(weather_callback(weights,nullptr,position,123)==weights&&weatherCalls==1,"inactive weather original args/result forwarded once");fog_callback(nullptr,position,4);Check(fogCalls==1,"inactive fog original forwarded once");
 std::printf("%u/%u current-map capture checks\n",checks-failed,checks);return failed?1:0;
}
