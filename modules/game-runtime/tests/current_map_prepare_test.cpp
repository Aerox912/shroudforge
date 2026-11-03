// Actual source admission against inert byte fixtures, never native game code.
#include <windows.h>
#include "fixtures/page_metadata_fixture.h"
namespace PrivateMainImage {inline HMODULE value{};inline HMODULE WINAPI Module(LPCWSTR name){return name?::GetModuleHandleW(name):(value ? value : ::GetModuleHandleW(nullptr));}}
#define GetModuleHandleW PrivateMainImage::Module
#define WriteExecutableBytes ComponentFixtureWriteExecutableBytes
#define ReplaceExecutableBytes ComponentFixtureReplaceExecutableBytes
#include "map/capture_game_map.cpp"
#include "fixtures/client_0403981f_marker_pins.h"
#include "fixtures/client_0403981f_optional_pins.h"
#include "fixtures/named_rows.h"
#include "fixtures/hud_rows.h"
#include "engine-hooks/held_entry_preparation.h"
#include <cstdio>
namespace EngineHooks {
unsigned writes{};
void* fixtureAllocation{};bool fixturePublish{},freshAllocations{};unsigned failAt{};
void* AllocateNearCode(std::uintptr_t,std::size_t){return freshAllocations?VirtualAlloc(nullptr,4096,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE):fixtureAllocation;}
bool WriteExecutableBytes(std::uintptr_t,const void*,std::size_t,const void*){++writes;return false;}
bool ReplaceExecutableBytes(std::uintptr_t at,const void* expected,const void* replacement,std::size_t size){++writes;if(!fixturePublish||writes==failAt||std::memcmp(reinterpret_cast<void*>(at),expected,size))return false;std::memcpy(reinterpret_cast<void*>(at),replacement,size);return true;}
}
#undef GetModuleHandleW
#undef WriteExecutableBytes
#undef ReplaceExecutableBytes
#define GetModuleHandleW PrivateMainImage::Module
#include "graphics/vulkan_overlay.cpp"
#undef GetModuleHandleW
#include "graphics/render_overlay.cpp"
namespace {
unsigned checks{},failed{};
void Check(bool b,const char* s){++checks;if(!b){++failed;std::printf("FAIL %s\n",s);}}
using namespace MapRuntime;
// Absent new APIs are a behavioral false/no-op only in this test adapter.
// Production gets no fake token, method body, receipt, or success fallback.
template<class A,class B>bool Owned(A a,B b){if constexpr(requires{ReadOwnedCameraEntryReplacement(a,b);})return ReadOwnedCameraEntryReplacement(a,b);else return false;}
template<class... A>void LogicalStop(A... a){if constexpr(requires{StopCapture(a...);})StopCapture(a...);}
template<class T>std::int32_t PrepareActual(const T& t){if constexpr(requires{PrepareCapture(t);})return PrepareCapture(t);else return SF_MAP_FAILED;}
template<class H>void QualifyPrivateFixture(H& h,HMODULE owner){if constexpr(requires{h.publicationQualified;h.residentOwner;}){h.publicationQualified=true;h.residentOwner=owner;}}
constexpr char Sha[]="0403981fd2e8ad18bd8c5698dff03c8d505b5a46677832c496db9c645513f2a6";
void FillImage(std::span<std::uint8_t> image){
 auto base=reinterpret_cast<std::uintptr_t>(image.data());
 auto write=[&](size_t rva,const void* data,size_t n){if(rva>image.size()||n>image.size()-rva)return false;std::memcpy(image.data()+rva,data,n);return true;};
 using namespace Fixture0403;
 MatchPins(CameraPins,write);MatchPins(RendererPins,write);MatchMarkers(base,write);
 MatchOptionalClock(base,write);MatchOptionalWeather(base,write);MatchOptionalFog(base,write);
 MatchOptionalWorld(base,write);MatchOptionalDeath(base,write);MatchOptionalVisibility(base,write);
 for(const auto& row:namedRows){const std::uintptr_t tuple[]{base+row.nameRva,std::strlen(row.name),base+row.function};write(row.descriptor,tuple,sizeof(tuple));write(row.nameRva,row.name,std::strlen(row.name)+1);}
 IMAGE_DOS_HEADER dos{};dos.e_magic=IMAGE_DOS_SIGNATURE;dos.e_lfanew=0x80;write(0,&dos,sizeof(dos));
 for(const auto& row:HudRows){write(row.rva,row.bytes.data(),row.bytes.size());if(!FixturePageMetadata::RegisterCode(image.data()+row.rva,row.bytes.size()))std::abort();}
 IMAGE_NT_HEADERS64 nt{};nt.Signature=IMAGE_NT_SIGNATURE;nt.FileHeader.Machine=IMAGE_FILE_MACHINE_AMD64;nt.FileHeader.TimeDateStamp=1793351710;nt.OptionalHeader.Magic=IMAGE_NT_OPTIONAL_HDR64_MAGIC;nt.OptionalHeader.SizeOfImage=static_cast<DWORD>(image.size());write(0x80,&nt,sizeof(nt));
}
std::vector<std::uint8_t> Image(){std::vector<std::uint8_t> image(0x2a40000);FillImage(image);return image;}
HANDLE parkedReady{},parkedRelease{};std::atomic<bool> parked{};DWORD parkedId{};std::int32_t preparationResult=SF_MAP_FAILED;
DWORD WINAPI Park(void*){parked=true;SetEvent(parkedReady);WaitForSingleObject(parkedRelease,5000);parked=false;return 0;}
bool __cdecl Held(std::uint32_t id) noexcept{return id==parkedId&&parked.load();}
bool PrepareUnderRealToken(const EngineHooks::HeldEntryToken& token,void*) noexcept{preparationResult=PrepareActual(token);return preparationResult==SF_MAP_READY;}
int HeldScenario(const char* scenario){
 auto* data=static_cast<std::uint8_t*>(VirtualAlloc(nullptr,0x2a40000,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE));if(!data)return 2;
 FillImage({data,0x2a40000});PrivateMainImage::value=reinterpret_cast<HMODULE>(data);
 auto* ntHeader=reinterpret_cast<IMAGE_NT_HEADERS64*>(data+0x80);ntHeader->OptionalHeader.NumberOfRvaAndSizes=16;
 ntHeader->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT]={0x80000,40};
 auto* imports=reinterpret_cast<IMAGE_IMPORT_DESCRIPTOR*>(data+0x80000);imports[0].OriginalFirstThunk=0x80100;imports[0].Name=0x80200;imports[0].FirstThunk=0x80300;
 reinterpret_cast<IMAGE_THUNK_DATA64*>(data+0x80100)[0].u1.AddressOfData=0x80400;std::memcpy(data+0x80200,"KERNEL32.dll",13);std::memcpy(data+0x80402,"GetProcAddress",15);
 *reinterpret_cast<std::uintptr_t*>(data+0x80300)=reinterpret_cast<std::uintptr_t>(&::GetProcAddress);
 namespace Profile=KfcRuntimeCompatibility::EnshroudedClient;Profile::image_sha256=Sha;Profile::image_size=0x2a40000;Profile::image_timestamp=1793351710;
 Check(FixturePageMetadata::RealNx(data,0x2a40000),"real VirtualQuery: held synthetic game image is physically RW/NX");
 EngineHooks::fixturePublish=true;EngineHooks::freshAllocations=true;
 if(std::strcmp(scenario,"graphics-unprepared")==0){Check(!OverlayRuntime::PrepareGraphicsBoot(true),"actual graphics boot requires cold prepared current renderer receipt");std::printf("%u/%u actual graphics-unprepared checks\n",checks-failed,checks);return failed?1:0;}
 const bool positive=std::strcmp(scenario,"positive")==0,pinRefusal=std::strcmp(scenario,"pin-refusal")==0;
 if(pinRefusal)data[0xe98de0+302]^=1;if(!positive&&!pinRefusal)EngineHooks::failAt=2;
 parkedReady=CreateEventW(nullptr,TRUE,FALSE,nullptr);parkedRelease=CreateEventW(nullptr,TRUE,FALSE,nullptr);auto thread=CreateThread(nullptr,0,Park,nullptr,0,&parkedId);if(!thread)return 2;WaitForSingleObject(parkedReady,1000);
 EngineHooks::HeldEntryProof proof;proof.parked_thread_id=parkedId;proof.is_held=Held;
 const bool result=EngineHooks::RunHeldEntryPreparation(proof,PrepareUnderRealToken,nullptr);
 Check(result==positive && (preparationResult==SF_MAP_READY)==positive,"real typed held token yields exact preparation result");
 Check(!accepting,"preparation never activates collection or fabricates a session");
 if(pinRefusal)Check(EngineHooks::writes==0&&hooks.empty(),"full original camera/helper pin refusal before any publication");
 std::array<std::uint8_t,18> original{},out{};for(const auto& pin:Fixture0403::CameraPins)if(pin.rva==0x2b7820)std::memcpy(original.data(),pin.bytes,18);
 if(positive){
  Check(hooks.size()==9&&std::all_of(hooks.begin(),hooks.end(),[](const auto& h){return h.installed;}),"all required capture/HUD hooks prepared inactive");
  Check(std::all_of(hooks.begin(),hooks.end(),[](const auto& h){return FixturePageMetadata::RealRx(reinterpret_cast<void*>(h.allocation));}),"all owned emitted bridges are physically RX after RW fill");
  Check(Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(out)),"successful real preparation qualifies live owned camera entry");
  data[0x2b7953]^=1;const auto writes=EngineHooks::writes;
  Check(OverlayRuntime::PrepareGraphicsBoot(true),"actual graphics boot consumes prepared renderer after deliberate FP CALL change");
  Check(EnableCapture(123,true)==SF_MAP_READY&&accepting&&capture_generation==0,"activation accepts prior preparation after deliberate FP CALL change, no session invented");
  Check(CapturedFrames().Acquire(0,GetTickCount64()).frame.status==SF_MAP_NO_SESSION,"prepared/activated capabilities still report real session unavailable");
  Check(EngineHooks::writes==writes,"activation performs no new native publication");LogicalStop();
  Check(!accepting&&EngineHooks::writes==writes&&Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(out)),"Stop performs no late write and retains genuine ownership");
 }else{
  Check(EnableCapture(123,true)!=SF_MAP_READY&&!accepting,"failed prepare cannot activate partial candidate");
  Check(!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(out)),"partial preparation never qualifies owned camera readback");
  if(!pinRefusal)Check(!hooks.empty()&&hooks.front().allocation&&hooks.front().installed,"partial published bridge retained inactive");
 }
 SetEvent(parkedRelease);WaitForSingleObject(thread,1000);CloseHandle(thread);CloseHandle(parkedReady);CloseHandle(parkedRelease);
 Check(FixturePageMetadata::RealNx(data,0x2a40000),"held synthetic game data remains NX after all preparation/activation actions");
 std::printf("%u/%u actual-held %s checks\n",checks-failed,checks,scenario);return failed?1:0;
}
}
int main(int argc,char** argv){
 if(argc>1)return HeldScenario(argv[1]);
 auto image=Image();auto inspect=[&]{return MapRuntime::InspectMapImage(image,Sha,true);};auto a=inspect();
 Check(a.status==SF_MAP_READY && a.player && a.markers && a.clock && a.weather && a.fog && a.world && a.deaths && a.discovery && a.renderer,"exact current0403 complete capabilities");
 Check(MapRuntime::InspectMapImage(image,"unknown",true).status==SF_MAP_UNSUPPORTED_BUILD,"unknown hash refused");
 Check(MapRuntime::InspectMapImage(image,Sha,false).status==SF_MAP_CAPABILITY_DENIED,"capability refusal");
 for(size_t rva:{size_t(0x2b7952),size_t(0x2b7fee),size_t(0xe98de0+302),size_t(0x9c7ac0+555),size_t(0x313f20),size_t(0x3a1890),size_t(0x308710)}){
  image[rva]^=1;Check(inspect().status!=SF_MAP_READY,"required full dependency mutation refuses whole candidate");image[rva]^=1;
 }
 image[0x1e2fd80]^=1;Check(inspect().status!=SF_MAP_READY,"relocated table pointer mutation refused");image[0x1e2fd80]^=1;
 image[0x80+offsetof(IMAGE_NT_HEADERS64,FileHeader)+offsetof(IMAGE_FILE_HEADER,TimeDateStamp)]^=1;Check(inspect().status!=SF_MAP_READY,"wrong current metadata refused");
 // Synthetic game data is physically RW/NX; owned emitted bridges alone use RX.
 auto* privateImage=static_cast<std::uint8_t*>(VirtualAlloc(nullptr,0x3000000,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE));
 auto* relay=VirtualAlloc(nullptr,4096,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!privateImage||!relay)return 2;
 MapRuntime::image_base=reinterpret_cast<std::uintptr_t>(privateImage);EngineHooks::fixtureAllocation=relay;EngineHooks::fixturePublish=true;
 KfcRuntimeCompatibility::EnshroudedClient::image_sha256=Sha;KfcRuntimeCompatibility::EnshroudedClient::image_size=0x2a40000;
 std::array<std::uint8_t,18> original{},replacement{};for(const auto& pin:Fixture0403::CameraPins)if(pin.rva==0x2b7820)std::memcpy(original.data(),pin.bytes,18);
 std::memcpy(privateImage+0x2b7820,original.data(),18);
 Check(entry_hook(0x2b7820,original,reinterpret_cast<void*>(&camera_callback)),"actual emitted entry bridge published only into private fixture");
 Check(FixturePageMetadata::RealNx(privateImage,0x3000000)&&FixturePageMetadata::RealRx(relay),"real OS metadata: synthetic game NX and owned bridge RX");
 Check(!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"legacy bool publication is not qualified owned receipt");
 HMODULE owner{};GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,reinterpret_cast<LPCWSTR>(&CapturedFrames),&owner);
 QualifyPrivateFixture(hooks.back(),owner); // inert qualified-writer stand-in, no real held transaction claim
 Check(Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"qualified actual live18/resident emitted bridge readback");
 privateImage[0x2b7820+17]^=1;Check(!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"changed live18 rejected");privateImage[0x2b7820+17]^=1;
 DWORD previous{};VirtualProtect(relay,4096,PAGE_READWRITE,&previous);static_cast<std::uint8_t*>(relay)[8]^=1;VirtualProtect(relay,4096,PAGE_EXECUTE_READ,&previous);Check(!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"changed RX resident bridge rejected");VirtualProtect(relay,4096,PAGE_READWRITE,&previous);static_cast<std::uint8_t*>(relay)[8]^=1;VirtualProtect(relay,4096,PAGE_EXECUTE_READ,&previous);
 VirtualProtect(relay,4096,PAGE_READWRITE,&previous);Check(FixturePageMetadata::RealNx(relay,4096)&&!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"unmodeled actual ReadOwned rejects a physical RW/NX bridge");
 const auto registeredBefore=FixturePageMetadata::used;FixturePageMetadata::RegisterCode(relay,4096);MEMORY_BASIC_INFORMATION modeled{};
 Check(FixturePageMetadata::Query(relay,&modeled,sizeof(modeled))==sizeof(modeled)&&modeled.Protect==PAGE_EXECUTE_READ&&!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"actual ReadOwned remains real-OS-only even when fixture adapter would report RX");FixturePageMetadata::used=registeredBefore;VirtualProtect(relay,4096,PAGE_EXECUTE_READ,&previous);
 VirtualProtect(relay,4096,PAGE_NOACCESS,&previous);Check(!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"unmodeled actual ReadOwned rejects noaccess bridge");VirtualProtect(relay,4096,PAGE_EXECUTE_READ,&previous);
 VirtualProtect(relay,4096,PAGE_EXECUTE_READ|PAGE_GUARD,&previous);Check(!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"unmodeled actual ReadOwned rejects guarded RX bridge");VirtualProtect(relay,4096,PAGE_EXECUTE_READ,&previous);
 for(const DWORD protection:{DWORD(PAGE_EXECUTE_READWRITE),DWORD(PAGE_EXECUTE),DWORD(PAGE_READONLY)}){
  const bool changed=VirtualProtect(relay,4096,protection,&previous)!=FALSE;MEMORY_BASIC_INFORMATION real{};
  Check(changed&&::VirtualQuery(relay,&real,sizeof(real))==sizeof(real)&&(real.Protect&0xff)==protection,"real OS applied owned-bridge protection variant");
  Check(changed&&!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"actual ReadOwned rejects writable/execute-only/nonexecutable protection");
  VirtualProtect(relay,4096,PAGE_EXECUTE_READ,&previous);Check(FixturePageMetadata::RealRx(relay)&&Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"real RX restoration restores qualified ownership");
 }
 for(const DWORD protection:{DWORD(PAGE_EXECUTE_WRITECOPY),DWORD(PAGE_WRITECOPY)}){
  SetLastError(ERROR_SUCCESS);const bool represented=VirtualProtect(relay,4096,protection,&previous)!=FALSE;const DWORD error=GetLastError();
  if(represented){MEMORY_BASIC_INFORMATION real{};Check(::VirtualQuery(relay,&real,sizeof(real))==sizeof(real)&&(real.Protect&0xff)==protection&&!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"actual represented write-copy protection refuses ownership");}
  else Check(FixturePageMetadata::RealRx(relay),"unrepresentable private write-copy leaves real RX unchanged; exact source predicate excludes it");
  std::printf("PRIVATE_WRITECOPY protection=%lu represented=%u error=%lu\n",protection,unsigned(represented),error);
  VirtualProtect(relay,4096,PAGE_EXECUTE_READ,&previous);Check(Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"write-copy trial restores qualified RX ownership");
 }
 Check(!Owned(std::span<const std::uint8_t>(original).first(17),std::span<std::uint8_t>(replacement)),"strict original18 extent");
 Check(!Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement).first(17)),"strict replacement18 extent");
 auto wrong=original;wrong[0]^=1;Check(!Owned(std::span<const std::uint8_t>(wrong),std::span<std::uint8_t>(replacement)),"wrong retained original refused");
 accepting=true;owners.insert(1);auto writes=EngineHooks::writes;LogicalStop();Check(!accepting&&EngineHooks::writes==writes&&hooks.back().installed,"logical Stop disables without late code write or loss of ownership");
 Check(Owned(std::span<const std::uint8_t>(original),std::span<std::uint8_t>(replacement)),"stopped capture retains real camera replacement readback");
 Check(FixturePageMetadata::RealNx(privateImage,0x3000000)&&FixturePageMetadata::RealRx(relay),"final synthetic game NX and owned bridge RX permissions preserved");
 std::printf("%u/%u current-map admission checks\n",checks-failed,checks);return failed?1:0;
}
