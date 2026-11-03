#include <windows.h>
#include <cstdio>
#include <atomic>
#define WriteExecutableBytes ComponentFixtureWriteExecutableBytes
#define ReplaceExecutableBytes ComponentFixtureReplaceExecutableBytes
#include "map/capture_game_map.cpp"
#include "fixtures/foreign_owner_fixture.h"
namespace TestHud {
unsigned forwards{},opens{},reads{},paths{};bool rightForward{};
void Dispatch(void(*fn)());
HANDLE WINAPI Open(LPCWSTR a,DWORD b,DWORD c,LPSECURITY_ATTRIBUTES d,DWORD e,DWORD f,HANDLE g){++opens;return ::CreateFileW(a,b,c,d,e,f,g);}
BOOL WINAPI Read(HANDLE a,void*b,DWORD c,DWORD*d,LPOVERLAPPED e){++reads;return ::ReadFile(a,b,c,d,e);}
DWORD WINAPI Path(HMODULE a,LPWSTR b,DWORD c){++paths;return ForeignPath(a,b,c);}
}
#define CreateFileW TestHud::Open
#define ReadFile TestHud::Read
#define GetModuleFileNameW TestHud::Path
#define GetModuleHandleExW TestHud::Module
#define FreeLibrary TestHud::Release
#define VirtualQuery FixturePageMetadata::Query
#define K32QueryWorkingSetEx FixturePageMetadata::Working
// Intercept only the existing forwarding call, never execute native/game bytes.
#define original() TestHud::Dispatch(original)
#include "map/hud_visibility.cpp"
#undef original
#undef CreateFileW
#undef ReadFile
#undef GetModuleFileNameW
#undef GetModuleHandleExW
#undef FreeLibrary
#undef VirtualQuery
#undef K32QueryWorkingSetEx
#include "fixtures/hud_rows.h"
namespace EngineHooks {
void* AllocateNearCode(std::uintptr_t,std::size_t){return nullptr;}
bool WriteExecutableBytes(std::uintptr_t,const void*,std::size_t,const void*){return false;}
bool ReplaceExecutableBytes(std::uintptr_t,const void*,const void*,std::size_t){return false;}
}
#undef WriteExecutableBytes
#undef ReplaceExecutableBytes
namespace TestHud {void Dispatch(void(*fn)()){++forwards;rightForward=reinterpret_cast<std::uintptr_t>(fn)==MapRuntime::image_base+0xc60310;}}
using namespace MapRuntime;
template<class T>bool ColdOwnership(T& value){if constexpr(requires{OwnershipSample(value,true,PublicCreativeCompat::ProofOrigin::AdmissionUpdate,true);})return OwnershipSample(value,true,PublicCreativeCompat::ProofOrigin::AdmissionUpdate,true);else return OwnershipSample(value,true,PublicCreativeCompat::ProofOrigin::AdmissionUpdate);}
int main(){using namespace MapRuntime;unsigned checks{},failed{};auto check=[&](bool b,const char*s){++checks;if(!b){++failed;std::printf("FAIL %s\n",s);}};
 auto* image=static_cast<std::uint8_t*>(VirtualAlloc(nullptr,0x2a40000,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE));if(!image)return 2;
 image_base=reinterpret_cast<std::uintptr_t>(image);hud_image=image_base;hud_size=0x2a40000;
 KfcRuntimeCompatibility::EnshroudedClient::image_sha256="0403981fd2e8ad18bd8c5698dff03c8d505b5a46677832c496db9c645513f2a6";
 for(const auto& row:HudRows){std::memcpy(image+row.rva,row.bytes.data(),row.bytes.size());if(!FixturePageMetadata::RegisterCode(image+row.rva,row.bytes.size()))return 2;}
 check(FixturePageMetadata::RealNx(image,0x2a40000),"real VirtualQuery: entire synthetic game image is physically RW/NX");
 Ownership sample{};check(OwnershipSample(sample,false,PublicCreativeCompat::ProofOrigin::AdmissionUpdate),"actual current HUD guard/renderer route admitted");
 FixturePageMetadata::enabled=false;check(!OwnershipSample(sample,false,PublicCreativeCompat::ProofOrigin::CallbackPre),"physical RW code data is refused when metadata model is disabled");FixturePageMetadata::enabled=true;
 DWORD oldProtection{};VirtualProtect(image+0xc60000,4096,PAGE_NOACCESS,&oldProtection);check(!OwnershipSample(sample,false,PublicCreativeCompat::ProofOrigin::CallbackPre),"real noaccess code-data page is never upgraded by metadata model");VirtualProtect(image+0xc60000,4096,PAGE_READWRITE,&oldProtection);
 VirtualProtect(image+0xc60000,4096,PAGE_READWRITE|PAGE_GUARD,&oldProtection);check(!OwnershipSample(sample,false,PublicCreativeCompat::ProofOrigin::CallbackPre),"real guard code-data page remains refused");VirtualProtect(image+0xc60000,4096,PAGE_READWRITE,&oldProtection);
 for(const auto& row:HudRows){image[row.rva+row.bytes.size()-1]^=1;check(!OwnershipSample(sample,false,PublicCreativeCompat::ProofOrigin::CallbackPre),"every actual HUD pin mutation refused");image[row.rva+row.bytes.size()-1]^=1;}
 std::uintptr_t context{},holder{},state{},play{};std::uint8_t objects[0x3000]{};context=reinterpret_cast<std::uintptr_t>(objects);holder=context+0x200;state=context+0x400;play=context+0x600;
 std::memcpy(image+0x28404a8,&context,8);std::memcpy(objects+0xd0,&holder,8);std::memcpy(reinterpret_cast<void*>(holder),&state,8);std::memcpy(reinterpret_cast<void*>(state+0x38),&play,8);*reinterpret_cast<std::uint8_t*>(play+0x1cd8)=1;
 std::array<std::uintptr_t,4> chain{};std::uint8_t screen{};check(Screen(screen,chain)&&screen==1&&chain[0]==context,"actual current screen/global layout");
 InstalledCapture call;call.target=image_base+0xbd154a;call.original.assign(image+0xbd154a,image+0xbd154a+5);call.replacement=call.original;call.replacement[1]^=1;
 auto* relay=static_cast<std::uint8_t*>(VirtualAlloc(nullptr,4096,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE));if(!relay)return 2;
 call.allocation=reinterpret_cast<std::uintptr_t>(relay);call.relay={0xff,0x25,0,0,0,0};auto callback=reinterpret_cast<std::uintptr_t>(&HudRender);for(unsigned i=0;i<8;++i)call.relay.push_back(std::uint8_t(callback>>(8*i)));std::memcpy(relay,call.relay.data(),call.relay.size());std::memcpy(image+0xbd154a,call.replacement.data(),5);call.installed=true;hooks.push_back(call);
 DWORD relayProtection{};check(VirtualProtect(relay,4096,PAGE_EXECUTE_READ,&relayProtection)&&FixturePageMetadata::RealRx(relay),"owned emitted HUD relay filled RW then physically RX; never invoked");
 hud_active=true;hud_epoch=9;HudRender();check(TestHud::forwards==1&&TestHud::rightForward,"actual wrapper original forwarded once via inert boundary");check(HudGameplay(GetTickCount64()),"actual wrapper publishes only after stable ownership/screen");
 image[0xbd154a+5]^=1;HudRender();check(TestHud::forwards==2&&!HudKnown(GetTickCount64()),"adjacent second CALL mutation refuses but forwards once");image[0xbd154a+5]^=1;
 DisableHudVisibility();HudRender();check(TestHud::forwards==3&&!HudKnown(GetTickCount64()),"disabled fetched wrapper still forwards once and publishes nothing");
 check(TestHud::opens==0&&TestHud::reads==0&&TestHud::paths==0,"actual native owner/CallbackPre/Post paths perform zero file work");
 check(TestHud::BuildForeign(image_base+0xc60310,PublicCreativeCompat::Original),"exact upstream data mapped only as inert foreign fixtures");
 check(std::all_of(std::begin(TestHud::foreign),std::end(TestHud::foreign),[](const auto& m){return FixturePageMetadata::RealNx(m.bytes,m.size);}),"real VirtualQuery: all F125/T117/C311 PE data remains physically RW/NX");
 {
  PublicCreativeCompat::AdmissionEpochScope cold;Ownership foreign{};
  check(ColdOwnership(foreign),"actual FTC311 T11 graph fully approved at explicit cold boundary");
 }
 check(std::all_of(std::begin(TestHud::foreign),std::end(TestHud::foreign),[](const auto& m){return m.references==1;}),"at most3 continuous provider leases retained after observations release");
 check(TestHud::leaseCapturesWhileBothObservationsHeld==3,"each cold lease acquired while both Observation references still held");
 auto opens=TestHud::opens,reads=TestHud::reads,paths=TestHud::paths;auto stamp=PublicCreativeCompat::CurrentIdentityStamp();
 hud_prepared=true;DisableHudVisibility();check(PublicCreativeCompat::IdentityStampValid(stamp),"current logical stop preserves approval only under continuous leases");
 check(EnableHudVisibility(image_base,0x2a40000),"actual current re-enable uses warm leased approvals");
 for(unsigned i=0;i<3;++i){unsigned refs=0;for(const auto& m:TestHud::foreign)refs+=m.acquires;Ownership warm{};check(OwnershipSample(warm,true,PublicCreativeCompat::ProofOrigin::CallbackPre),"current repeated warm graph remains fully verified");unsigned after=0;for(const auto& m:TestHud::foreign)after+=m.acquires;check(after-refs==6,"warm whole observations add only their existing temporary refs, no new leases");}
 check(TestHud::opens==opens&&TestHud::reads==reads&&TestHud::paths==paths&&PublicCreativeCompat::IdentityStampValid(stamp),"disable/re-enable/repeated callbacks mint no file work or epoch");
 auto& provider=TestHud::foreign[0];provider.bytes[0xe260]^=1;Ownership changed{};
 check(!OwnershipSample(changed,true,PublicCreativeCompat::ProofOrigin::CallbackPre),"body mutation still refuses despite continuous lifetime lease");provider.bytes[0xe260]^=1;
 check(!OwnershipSample(changed,true,PublicCreativeCompat::ProofOrigin::CallbackPost),"invalidated body cannot silently regain warm admission");
 check(TestHud::opens==opens&&TestHud::reads==reads&&TestHud::paths==paths,"mismatch/invalidation cannot acquire cold retry work");
 check(std::all_of(std::begin(TestHud::foreign),std::end(TestHud::foreign),[](const auto& m){return m.references==1&&m.acquires-m.releases==1;}),"exactly3 process leases retained without unbounded reference growth");
 check(FixturePageMetadata::RealNx(image,0x2a40000)&&std::all_of(std::begin(TestHud::foreign),std::end(TestHud::foreign),[](const auto& m){return FixturePageMetadata::RealNx(m.bytes,m.size);}),"all synthetic/foreign data remains NX after complete reader/lifecycle test");
 std::printf("PCC executable-page metadata modeled=%u; real owned-reader metadata unmodified\n",FixturePageMetadata::modeled);
 std::printf("%u/%u current-map HUD reader checks\n",checks-failed,checks);return failed?1:0;
}
