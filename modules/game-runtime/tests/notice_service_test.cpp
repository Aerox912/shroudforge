#include "map/serve_map_overlay.cpp"
#include "graphics/rasterize_notice.h"
#include <cstdio>
#include <cstdlib>
#include <thread>
#include <future>
#include <cstring>

namespace {unsigned checks{},enabled{},disabled{},started{},stopped{};std::uint32_t status=0;
MapRuntime::SnapshotStore fixture;
void check(bool value){++checks;if(!value){std::fprintf(stderr,"notice check %u failed\n",checks);std::exit(1);}}
std::int32_t query(const char* id,std::uint64_t generation,SFNoticeServiceV1& table) {
    constexpr char name[]="shroudforge.notices";
    return KfcRuntimeQueryMapOverlayService(reinterpret_cast<const std::uint8_t*>(id),std::uint32_t(std::strlen(id)),generation,
        reinterpret_cast<const std::uint8_t*>(name),sizeof(name)-1,1,sizeof(table),&table);
}
std::int32_t show(SFNoticeServiceV1& table,const char* text) {return table.show(table.context,reinterpret_cast<const std::uint8_t*>(text),std::uint32_t(std::strlen(text)));}
}
namespace MapRuntime {
SnapshotStore& CapturedFrames(){return fixture;}
std::int32_t EnableCapture(std::uint64_t,bool){++enabled;return SF_MAP_READY;}
void DisableCapture(std::uint64_t){++disabled;}
void UpdateCaptureSession(std::uint64_t,bool,bool,bool){}
bool HudKnown(std::uint64_t){return false;}bool HudGameplay(std::uint64_t){return false;}bool HudInputAllowed(std::uint64_t){return true;}
}
namespace OverlayRuntime {
bool StartGraphics(){++started;return true;}void StopGraphics(){++stopped;}void MaintainGraphics(){}
SFOverlayStatusV1 GraphicsStatus(){return {sizeof(SFOverlayStatusV1),1,status};}
}
extern "C" std::uint64_t __cdecl KfcRuntimeWorldSessionId(){return 0;}
int main() {
    static_assert(sizeof(SFNoticeServiceV1)==24);
    SFNoticeServiceV1 first{},second{};check(query("first",1,first)==0);check(query("second",1,second)==0);
    check(enabled==0 && started==2); // Graphics demand never installs capture hooks.
    check(first.version==1 && first.size==24);
    SFNoticeServiceV1 sentinel{};sentinel.version=123;
    constexpr char service_name[]="shroudforge.notices";const auto* service_bytes=reinterpret_cast<const std::uint8_t*>(service_name);
    const auto* first_bytes=reinterpret_cast<const std::uint8_t*>("first");
    check(KfcRuntimeQueryMapOverlayService(first_bytes,5,1,service_bytes,sizeof(service_name)-1,1,sizeof(sentinel)-1,&sentinel)==5 && sentinel.version==123);
    check(KfcRuntimeQueryMapOverlayService(first_bytes,5,1,service_bytes,sizeof(service_name)-1,2,sizeof(sentinel),&sentinel)==2 && sentinel.version==123);
    status=3;check(show(first,"blocked")==3);status=0;
    check(show(first,"")==1);check(show(first,"line\nbreak")==1);check(show(first,"bad\xFF")==1);
    std::string long_text(257,'x');check(show(first,long_text.c_str())==1);
    std::vector<std::uint8_t> pixels;
    const char unicode[]="Flug: an \xC3\xBC";
    check(OverlayRuntime::RasterizeNotice(reinterpret_cast<const std::uint8_t*>(unicode),sizeof(unicode)-1,pixels)==0);
    check(pixels.size()==640*72*4);bool glyph{};for(std::size_t i=0;i<pixels.size();i+=4){check(pixels[i+3]==255);glyph|=pixels[i]>24;}check(glyph);
    check(show(first,"First message")==0);
    struct Probe {unsigned calls{};std::uint64_t owner{},revision{};std::vector<std::uint8_t> pixels;std::promise<void>* entered{};std::shared_future<void> resume;} probe;
    SFOverlayEncoderV1 encoder{sizeof(encoder),1,&probe,0,1,nullptr,
        [](void* p,std::uint64_t owner,const SFOverlayTextureCommandV1* command,std::uint32_t count)->std::int32_t {
            auto& v=*static_cast<Probe*>(p);++v.calls;v.owner=owner;v.revision=command->texture.revision;
            check(count==1 && command->texture.resource_id==UINT64_MAX && (owner>>63)==1 && command->x==80 && command->y==24);
            v.pixels.assign(command->texture.rgba,command->texture.rgba+command->texture.rgba_bytes);
            if(v.entered){v.entered->set_value();v.resume.wait();check(v.pixels==std::vector<std::uint8_t>(command->texture.rgba,command->texture.rgba+command->texture.rgba_bytes));}
            return 0;
        },nullptr};
    const auto before=GetTickCount64();
    check(OverlayRuntime::Services().Dispatch(800,600,before,1,encoder));check(probe.calls==1);
    const auto first_owner=probe.owner,first_revision=probe.revision;
    check(show(second,"Second message")==0);
    OverlayRuntime::Services().Dispatch(800,600,GetTickCount64(),1,encoder);
    check(probe.calls==2 && probe.owner!=first_owner && probe.revision>first_revision);
    const auto second_revision=probe.revision;
    check(show(second,"Replacement")==0);
    OverlayRuntime::Services().Dispatch(800,600,GetTickCount64(),1,encoder);check(probe.calls==3 && probe.revision>second_revision);
    int foreign{};std::thread wrong([&]{foreign=show(second,"Wrong thread");});wrong.join();check(foreign==8);
    OverlayRuntime::Services().Dispatch(800,600,GetTickCount64()+3001,1,encoder);check(probe.calls==3);
    OverlayRuntime::Services().Dispatch(800,600,GetTickCount64()+3002,1,encoder);check(probe.calls==3); // No resurrection.
    check(show(second,"Retained render")==0);
    std::promise<void> entered,resume;probe.entered=&entered;probe.resume=resume.get_future().share();
    std::thread render([&]{OverlayRuntime::Services().Dispatch(800,600,GetTickCount64(),1,encoder);});entered.get_future().wait();
    const auto* id=reinterpret_cast<const std::uint8_t*>("second");
    check(KfcRuntimeRevokeMapOverlayServices(id,6,1)==0);
    check(show(second,"Revoked")==3);
    check(KfcRuntimeDrainMapOverlayServices(id,6,1,0)==6);
    check(show(first,"Replacement during old render")==0);
    resume.set_value();render.join();probe.entered=nullptr;
    check(KfcRuntimeDrainMapOverlayServices(id,6,1,0)==0 && stopped==0 && disabled==0);
    check(query("second",1,second)==3);
    id=reinterpret_cast<const std::uint8_t*>("first");check(KfcRuntimeRevokeMapOverlayServices(id,5,1)==0);
    check(KfcRuntimeDrainMapOverlayServices(id,5,1,0)==0 && stopped==1 && disabled==0);
    OverlayRuntime::Services().Dispatch(800,600,GetTickCount64(),1,encoder);check(probe.calls==4);
    std::printf("PASS: %u notice UTF-8/raster/owner/replacement/expiry/thread/revoke/drain checks\n",checks);
}
