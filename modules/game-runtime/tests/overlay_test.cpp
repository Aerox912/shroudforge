#include "graphics/render_overlay.h"
#include <cstdio>
#include <cstdlib>
#include <future>
#include <thread>
#include <string>
#include "graphics/present_transaction.h"

// Isolate the registry from a real GPU. A separate fixture exercises renderer
// transitions; this dependency reports unavailable rather than fake readiness.
namespace OverlayRuntime { SFOverlayStatusV1 GraphicsStatus() { return {sizeof(SFOverlayStatusV1),1,3}; } }

namespace {
unsigned checks{};
void check(bool value) { ++checks; if(!value){std::fprintf(stderr,"overlay check %u failed\n",checks);std::exit(1);} }
std::promise<void> entered, resume;
std::shared_future<void> resumed;
int callbacks{};
std::int32_t callback(void*,const SFOverlayFrameV1* frame,const SFOverlayEncoderV1*) {
    ++callbacks; check(frame->width==800 && frame->map_frame->session_generation==9);
    entered.set_value(); resumed.wait(); return 0;
}
}
int main() {
    MapRuntime::SnapshotStore frames; frames.SetAdmission(SF_MAP_FEATURE_PLAYER);
    frames.SetSession(9,true,false,true,100);
    OverlayRuntime::Registry registry(frames);
    auto first=registry.Open("first",1), second=registry.Open("second",1);
    SFMapServiceV1 map{}; SFOverlayServiceV1 overlay{}, other{};
    check(registry.Query(first,"shroudforge.map",1,sizeof(map),&map)==0);
    check(registry.Query(first,"shroudforge.overlay",1,sizeof(overlay),&overlay)==0);
    check(registry.Query(second,"shroudforge.overlay",1,sizeof(other),&other)==0);
    SFOverlayServiceV1 sentinel{}; sentinel.version=123;
    check(registry.Query(first,"shroudforge.overlay",1,sizeof(sentinel)-1,&sentinel)==5 && sentinel.version==123);
    check(registry.Query(first,"shroudforge.overlay",3,sizeof(sentinel),&sentinel)==2);
    SFMapFrameV1 frame{}; SFMapFrameLeaseV1 lease{};
    check(map.acquire(map.context,9,sizeof(frame)-1,&frame,&lease)==5 && lease==0);
    check(map.acquire(map.context,9,sizeof(frame),&frame,&lease)==0 && lease && frame.session_generation==9);
    check(registry.Revoke(second)==0 && registry.Drain(second,0)==0);
    SFOverlayRegistrationV1 registration{};
    resumed=resume.get_future().share();
    check(overlay.register_frame(overlay.context,callback,nullptr,&registration)==0 && registration);
    SFOverlayRegistrationV1 duplicate{};
    check(overlay.register_frame(overlay.context,callback,nullptr,&duplicate)==1);
    check(other.unregister_frame(other.context,registration)==1);
    SFOverlayEncoderV1 encoder{};
    std::thread render([&]{registry.Dispatch(800,600,100,1,encoder);});
    entered.get_future().wait();
    check(overlay.unregister_frame(overlay.context,registration)==6);
    check(registry.Revoke(first)==0);
    check(registry.Drain(first,0)==6);
    resume.set_value(); render.join();
    check(registry.Drain(first,0)==6); // The borrowed frame lease also fences owner retirement.
    check(map.release(map.context,lease)==0);
    check(map.release(map.context,lease)==1);
    check(registry.Drain(first,0)==0);
    registry.Dispatch(800,600,101,1,encoder); check(callbacks==1);
    check(map.acquire(map.context,9,sizeof(frame),&frame,&lease)==3);
    auto third=registry.Open("third",1);SFOverlayServiceV1 bound{};
    check(registry.Query(third,"shroudforge.overlay",1,sizeof(bound),&bound)==0);
    struct Probe {SFOverlayEncoderV1 saved{};int backend_calls{};bool valid{},wrong{},foreign{};} probe;
    SFOverlayEncoderV1 backend{sizeof(backend),1,&probe,0,2,
        [](void* p,std::uint64_t,const SFOverlayRectangleV1*,std::uint32_t){++static_cast<Probe*>(p)->backend_calls;return 0;},nullptr,nullptr};
    auto bound_callback=[](void* p,const SFOverlayFrameV1*,const SFOverlayEncoderV1* e)->std::int32_t {
        auto& v=*static_cast<Probe*>(p);v.saved=*e;
        v.valid=e->draw_rectangles(e->context,e->owner_id,nullptr,0)==0;
        v.wrong=e->draw_rectangles(e->context,e->owner_id+1,nullptr,0)==3;
        std::thread other_thread([&]{v.foreign=e->draw_rectangles(e->context,e->owner_id,nullptr,0)==3;});other_thread.join();return 0;
    };
    check(bound.register_frame(bound.context,bound_callback,&probe,&registration)==0);
    registry.Dispatch(800,600,102,2,backend);
    check(probe.valid && probe.wrong && probe.foreign && probe.backend_calls==1);
    check(probe.saved.draw_rectangles(probe.saved.context,third,nullptr,0)==3 && probe.backend_calls==1);
    auto fourth=registry.Open("extended",1);SFOverlayServiceV1 extendedService{};
    check(registry.Query(fourth,"shroudforge.overlay",2,sizeof(extendedService),&extendedService)==0 && extendedService.version==2);
    struct ExtendedProbe {SFOverlayEncoderV2 saved{};int calls{},backendCalls{};bool wrong{},foreign{},valid{};} v2;
    SFOverlayEncoderV2 extendedBackend{};
    extendedBackend.base={sizeof(extendedBackend),2,&v2,0,2,nullptr,nullptr,nullptr};
    extendedBackend.draw_textures_v2=[](void* p,std::uint64_t,const SFOverlayTextureCommandV2*,std::uint32_t){++static_cast<ExtendedProbe*>(p)->backendCalls;return 0;};
    extendedBackend.draw_terrain_v2=[](void* p,std::uint64_t,const SFOverlayTerrainV2*){++static_cast<ExtendedProbe*>(p)->backendCalls;return 0;};
    auto extendedCallback=[](void* p,const SFOverlayFrameV1* f,const SFOverlayEncoderV1* e)->std::int32_t {
        auto& v=*static_cast<ExtendedProbe*>(p);++v.calls;
        check(f->version==2 && e->version==2 && e->size>=sizeof(SFOverlayEncoderV2));
        const auto& x=*reinterpret_cast<const SFOverlayEncoderV2*>(e);v.saved=x;
        v.valid=x.draw_textures_v2(e->context,e->owner_id,nullptr,0)==0;
        v.wrong=x.draw_textures_v2(e->context,e->owner_id+1,nullptr,0)==3;
        std::thread foreign([&]{v.foreign=x.draw_terrain_v2(e->context,e->owner_id,nullptr)==3;});foreign.join();return 0;
    };
    check(extendedService.register_frame(extendedService.context,extendedCallback,&v2,&registration)==0);
    check(registry.Query(fourth,"shroudforge.overlay",1,sizeof(sentinel),&sentinel)==6);
    registry.Dispatch(800,600,104,2,extendedBackend.base);
    check(v2.calls==1 && v2.valid && v2.wrong && v2.foreign && v2.backendCalls==1);
    check(v2.saved.draw_textures_v2(v2.saved.base.context,fourth,nullptr,0)==3 && v2.backendCalls==1);
    registry.Dispatch(800,600,105,2,backend);check(v2.calls==1); // Missing operations fail closed without a callback.
    check(registry.Revoke(fourth)==0 && registry.Drain(fourth,0)==0);
    registry.Dispatch(800,600,105,2,extendedBackend.base);check(v2.calls==1);
    struct Gpu { int fence{},reset{},submit{}; bool record=true; std::string calls; };
    const auto steps=[](Gpu& gpu) { return OverlayRuntime::PresentSteps{&gpu,
        [](void* p){auto& g=*static_cast<Gpu*>(p);g.calls+='F';return g.fence;},
        [](void* p){auto& g=*static_cast<Gpu*>(p);g.calls+='R';return g.record;},
        [](void* p){auto& g=*static_cast<Gpu*>(p);g.calls+='X';return g.reset;},
        [](void* p){auto& g=*static_cast<Gpu*>(p);g.calls+='S';return g.submit;}}; };
    using Result=OverlayRuntime::PresentResult;
    Gpu gpu; gpu.fence=1;
    check(OverlayRuntime::SubmitPresent(steps(gpu))==Result::Busy && gpu.calls=="F");
    gpu={};gpu.record=false;
    check(OverlayRuntime::SubmitPresent(steps(gpu))==Result::Failed && gpu.calls=="FR");
    gpu={};gpu.reset=-1;
    check(OverlayRuntime::SubmitPresent(steps(gpu))==Result::Failed && gpu.calls=="FRX");
    gpu={};gpu.submit=-1;
    check(OverlayRuntime::SubmitPresent(steps(gpu))==Result::Failed && gpu.calls=="FRXS");
    gpu={};gpu.fence=-4;
    check(OverlayRuntime::SubmitPresent(steps(gpu))==Result::DeviceLost && gpu.calls=="F");
    gpu={};gpu.submit=-4;
    check(OverlayRuntime::SubmitPresent(steps(gpu))==Result::DeviceLost && gpu.calls=="FRXS");
    gpu={};
    check(OverlayRuntime::SubmitPresent(steps(gpu))==Result::Submitted && gpu.calls=="FRXS");
    std::printf("PASS: %u native overlay owner/lease/callback checks\n",checks);
}
