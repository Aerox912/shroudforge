#include "map/serve_map_overlay.cpp"

#include <array>
#include <cstdio>
#include <cstdlib>

namespace {
unsigned checks{}, capture_enabled{}, capture_disabled{}, graphics_starts{}, graphics_stops{};
bool capture_capability_seen{};
MapRuntime::SnapshotStore frames;

constexpr std::uint8_t package[]{'q','u','e','r','y','-','v','2'};
constexpr std::uint8_t map_name[]{'s','h','r','o','u','d','f','o','r','g','e','.','m','a','p'};
constexpr std::uint8_t overlay_name[]{'s','h','r','o','u','d','f','o','r','g','e','.','o','v','e','r','l','a','y'};
constexpr std::uint8_t unknown_name[]{'s','h','r','o','u','d','f','o','r','g','e','.','u','n','k','n','o','w','n'};
constexpr std::uint64_t generation=177;

void check(bool value,const char* message) {
    ++checks;
    if(!value) { std::fprintf(stderr,"map overlay query check %u failed: %s\n",checks,message); std::exit(1); }
}

template<class T>
std::int32_t query(const std::uint8_t* service,std::uint32_t service_size,std::uint32_t version,
    std::uint32_t capacity,T* output) {
    return KfcRuntimeQueryMapOverlayService(package,sizeof(package),generation,
        service,service_size,version,capacity,output);
}

std::int32_t callback(void*,const SFOverlayFrameV1*,const SFOverlayEncoderV1*) { return 0; }
}

namespace MapRuntime {
SnapshotStore& CapturedFrames() { return frames; }
std::int32_t EnableCapture(std::uint64_t,bool capability_allowed) {
    ++capture_enabled; capture_capability_seen=capability_allowed; return SF_MAP_READY;
}
void DisableCapture(std::uint64_t) { ++capture_disabled; }
void UpdateCaptureSession(std::uint64_t,bool,bool,bool) {}
bool HudKnown(std::uint64_t) { return false; }
bool HudGameplay(std::uint64_t) { return false; }
bool HudInputAllowed(std::uint64_t) { return true; }
}

namespace OverlayRuntime {
bool StartGraphics() { ++graphics_starts; return true; }
void StopGraphics() { ++graphics_stops; }
void MaintainGraphics() {}
SFOverlayStatusV1 GraphicsStatus() { return {sizeof(SFOverlayStatusV1),1,3}; }
}

extern "C" std::uint64_t __cdecl KfcRuntimeWorldSessionId() { return 0; }

int main() {
    alignas(SFMapServiceV1) std::array<std::uint8_t,sizeof(SFMapServiceV1)> map_output;
    alignas(SFOverlayServiceV1) std::array<std::uint8_t,sizeof(SFOverlayServiceV1)> overlay_output;
    map_output.fill(0xA5); overlay_output.fill(0x5A);
    const auto untouched_map=map_output;
    const auto untouched_overlay=overlay_output;

    check(KfcRuntimeQueryMapOverlayService(package,sizeof(package),generation,
        unknown_name,sizeof(unknown_name),3,0,nullptr)==1,
        "null output keeps argument error precedence");
    check(query(unknown_name,sizeof(unknown_name),3,0,overlay_output.data())==4,
        "unknown service name keeps precedence over unknown version and capacity");
    check(overlay_output==untouched_overlay,"unknown service leaves output untouched");
    check(query(map_name,sizeof(map_name),2,0,map_output.data())==2,
        "map service remains version 1 only before capacity validation");
    check(query(overlay_name,sizeof(overlay_name),3,0,overlay_output.data())==2,
        "unknown overlay version is rejected before capacity validation");
    check(query(map_name,sizeof(map_name),SF_MAP_SERVICE_VERSION,sizeof(SFMapServiceV1)-1,
        map_output.data())==5,"map service keeps its exact existing table capacity");
    check(query(overlay_name,sizeof(overlay_name),SF_OVERLAY_SERVICE_VERSION,sizeof(SFOverlayServiceV1)-1,
        overlay_output.data())==5,"overlay version 1 keeps its table capacity");
    check(query(overlay_name,sizeof(overlay_name),SF_OVERLAY_EXTENDED_SERVICE_VERSION,sizeof(SFOverlayServiceV1)-1,
        overlay_output.data())==5,"overlay version 2 uses the existing service table capacity");
    check(map_output==untouched_map && overlay_output==untouched_overlay,
        "rejected queries leave caller buffers unchanged");
    check(capture_enabled==0 && graphics_starts==0,
        "invalid name, version, and capacity do not create an owner or start graphics");

    SFMapServiceV1 map{};
    check(query(map_name,sizeof(map_name),SF_MAP_SERVICE_VERSION,sizeof(map),&map)==0,
        "map version 1 remains available");
    check(map.size==sizeof(map) && map.version==SF_MAP_SERVICE_VERSION && map.context &&
        map.acquire && map.release && map.read_keys,"map version 1 returns the existing complete table");
    check(capture_enabled==1 && capture_capability_seen,
        "the recognized map capability opens one capture owner");

    SFOverlayServiceV1 overlay_v1{};
    check(query(overlay_name,sizeof(overlay_name),SF_OVERLAY_SERVICE_VERSION,sizeof(overlay_v1),&overlay_v1)==0,
        "overlay version 1 remains available");
    check(overlay_v1.size==sizeof(overlay_v1) && overlay_v1.version==SF_OVERLAY_SERVICE_VERSION &&
        overlay_v1.context==map.context && overlay_v1.register_frame && overlay_v1.unregister_frame &&
        overlay_v1.read_status,"overlay version 1 returns the current table for the same owner");

    SFOverlayServiceV1 overlay_v2{};
    check(query(overlay_name,sizeof(overlay_name),SF_OVERLAY_EXTENDED_SERVICE_VERSION,
        sizeof(overlay_v2),&overlay_v2)==0,"the public query dispatch reaches the registry for overlay version 2");
    check(overlay_v2.size==sizeof(overlay_v2) && overlay_v2.version==SF_OVERLAY_EXTENDED_SERVICE_VERSION &&
        overlay_v2.context==overlay_v1.context && overlay_v2.context==map.context &&
        overlay_v2.register_frame && overlay_v2.unregister_frame && overlay_v2.read_status,
        "overlay version 2 returns the existing bounded table layout and owner");
    check(capture_enabled==1 && graphics_starts==2,
        "version 2 reuses the opened capability owner and reaches the overlay start boundary");

    SFOverlayRegistrationV1 registration{};
    check(overlay_v2.register_frame(overlay_v2.context,callback,nullptr,&registration)==0 && registration,
        "version 2 table registration remains functional");
    check(query(overlay_name,sizeof(overlay_name),SF_OVERLAY_SERVICE_VERSION,sizeof(SFOverlayServiceV1),
        overlay_output.data())==6,"an active owner still rejects changing overlay versions");
    check(overlay_output==untouched_overlay,"owner version conflict leaves output untouched");
    check(overlay_v2.unregister_frame(overlay_v2.context,registration)==0,
        "the registered callback can be removed through the same owner");
    check(query(overlay_name,sizeof(overlay_name),SF_OVERLAY_SERVICE_VERSION,sizeof(overlay_v1),&overlay_v1)==0 &&
        overlay_v1.version==SF_OVERLAY_SERVICE_VERSION && overlay_v1.context==overlay_v2.context,
        "an idle owner can return to overlay version 1");

    check(KfcRuntimeRevokeMapOverlayServices(package,sizeof(package),generation)==0,
        "the queried package generation can be revoked");
    check(query(overlay_name,sizeof(overlay_name),SF_OVERLAY_EXTENDED_SERVICE_VERSION,
        sizeof(SFOverlayServiceV1),overlay_output.data())==3,
        "a revoked owner remains closed for overlay version 2");
    check(overlay_output==untouched_overlay,"revoked query leaves output untouched");
    check(KfcRuntimeDrainMapOverlayServices(package,sizeof(package),generation,0)==0 &&
        capture_disabled==1 && graphics_stops==1,
        "draining the revoked owner releases capture and graphics once");

    std::printf("PASS: %u public map-overlay query checks\n",checks);
}
