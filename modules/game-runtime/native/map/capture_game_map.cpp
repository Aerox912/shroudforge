// Narrow native capture mechanisms adapted from pinned ESM Minimap test.4.
// MIT, Copyright (c) 2024 Miguel Oppermann. See SOURCE-NOTICE.txt.
#include "map/capture_game_map.h"
#include "map/hud_visibility.h"
#include "profiles/game_profile.h"
#include "engine-hooks/allocate_near_code.h"
#include "engine-hooks/encode_relative_jump.h"
#include "engine-hooks/write_executable_bytes.h"
#include <windows.h>
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstring>
#include <limits>
#include <unordered_set>

namespace MapRuntime {
struct ImageReader {
    std::span<const std::uint8_t> bytes;
    bool Match(std::size_t rva, const void* expected, std::size_t size) const {
        if (rva > bytes.size() || size > bytes.size()-rva || size > 1024) return false;
        std::uint8_t actual[1024]{};
        return ReadMemory(reinterpret_cast<std::uintptr_t>(bytes.data()+rva),actual,size) && !std::memcmp(actual,expected,size);
    }
    bool Named(std::size_t rva, const char* name, std::size_t function) const {
        std::uintptr_t descriptor[3]{}; char actual[96]{}; const auto length=std::strlen(name)+1;
        const auto base=reinterpret_cast<std::uintptr_t>(bytes.data());
        return rva <= bytes.size() && sizeof(descriptor) <= bytes.size()-rva && length <= sizeof(actual) &&
            ReadMemory(base+rva,descriptor,sizeof(descriptor)) && descriptor[0] >= base &&
            descriptor[0]-base <= bytes.size() && length <= bytes.size()-(descriptor[0]-base) &&
            descriptor[1] == length-1 && descriptor[2] == base+function &&
            ReadMemory(descriptor[0],actual,length) && !std::memcmp(actual,name,length);
    }
};
}
#include "map/test4_guard_tables.h"

namespace MapRuntime {
MapImageAdmission InspectMapImage(std::span<const std::uint8_t> bytes, std::string_view sha, bool permitted) {
    MapImageAdmission result;
    if (!permitted) { result.status=SF_MAP_CAPABILITY_DENIED; return result; }
    if (sha != CurrentClientSha256 || bytes.size() != 44163072) return result;
    IMAGE_DOS_HEADER dos{}; IMAGE_NT_HEADERS64 nt{};
    const auto base=reinterpret_cast<std::uintptr_t>(bytes.data());
    if (!ReadMemory(base,&dos,sizeof(dos)) || dos.e_magic != IMAGE_DOS_SIGNATURE || dos.e_lfanew < 0 ||
        std::size_t(dos.e_lfanew) > bytes.size()-sizeof(nt) || !ReadMemory(base+dos.e_lfanew,&nt,sizeof(nt)) ||
        nt.Signature != IMAGE_NT_SIGNATURE || nt.FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 ||
        nt.OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC || nt.FileHeader.TimeDateStamp != 1792507660 ||
        nt.OptionalHeader.SizeOfImage != bytes.size()) return result;
    const ImageReader image{bytes};
    const bool named=image.Named(0x1e04490,"player_camera_transform",0x2b2b60) &&
        image.Named(0x1e0e400,"player_waypoints_ui",0x30eb10);
    result.player=named && Test4Guards::Camera(image);
    result.renderer=Test4Guards::Renderer(image);
    result.markers=named && image.Named(0x1e13e00,"update_ui_data",0x317e80) && Test4Guards::Markers(image);
    result.clock=image.Named(0x1e04660,"player_daytime_ui",0x2b3340) &&
        image.Named(0x1e08620,"client_update_daytime",0x3027b0) && Test4Guards::Clock(image);
    result.weather=image.Named(0x1e05560,"ambient_effects",0x2fd9a0) && Test4Guards::Weather(image);
    if (result.weather) {
        const char* names[]{"Clear","Rain","Snow","Blizzard"};
        for (std::uint64_t i=0;i<4;++i) {
            std::uintptr_t name{}; std::uint64_t value{}; char actual[9]{};
            const auto entry=base+0x1bbd820+i*0x28, length=std::strlen(names[i])+1;
            if (!ReadMemory(entry,&name,8) || name < base || name-base > bytes.size() || length > bytes.size()-(name-base) ||
                !ReadMemory(entry+0x10,&value,8) || value != i || !ReadMemory(name,actual,length) ||
                std::memcmp(actual,names[i],length)) result.weather=false;
        }
    }
    result.fog=image.Named(0x1e0a960,"fog_of_war",0x3071e0) && Test4Guards::Fog(image);
    result.world=image.Named(0x1e063d0,"clear_map_markers_for_ui",0x2fefe0) &&
        image.Named(0x1e09410,"custom_map_markers_ui",0x303360) &&
        image.Named(0x1e0f970,"replicate_map_markers_for_ui",0x310cb0) && Test4Guards::World(image);
    result.deaths=result.world && Test4Guards::Death(image);
    result.discovery=image.Named(0x1dea720,"knowledge_query_mapmarker_visibility",0x415140) && Test4Guards::Visibility(image);
    if (result.player) result.status=SF_MAP_READY;
    return result;
}
std::uint32_t MapImageAdmission::Features() const {
    return (player ? SF_MAP_FEATURE_PLAYER : 0) | (markers ? SF_MAP_FEATURE_MARKERS : 0) |
        (fog ? SF_MAP_FEATURE_FOG : 0) | ((clock || weather) ? SF_MAP_FEATURE_ENVIRONMENT : 0) |
        (clock ? SF_MAP_FEATURE_CLOCK : 0) | (weather ? SF_MAP_FEATURE_WEATHER : 0) |
        (world ? SF_MAP_FEATURE_WORLD : 0) | (deaths ? SF_MAP_FEATURE_DEATHS : 0) |
        (discovery ? SF_MAP_FEATURE_DISCOVERY : 0);
}
bool ReadLocalCamera(const NativeCameraRecord& record, std::uint32_t entity,
    SFMapPoseV1& player, SFMapPoseV1& camera) {
    std::uint32_t local{}; NativeTransform p{},c{};
    return entity && entity <= 1023 && ReadMemory(record.local_player+4,&local,4) && local == entity &&
        ReadMemory(record.player_transform,&p,sizeof(p)) && ReadMemory(record.camera_transform,&c,sizeof(c)) &&
        DecodeLocalTransforms(p,c,player,camera);
}
bool ReadPingEvents(std::uintptr_t list, std::uint32_t excluded, std::uint64_t now,
    std::vector<SFMapMarkerV1>& result) {
    result.clear(); std::uint32_t count{}; std::uint16_t index{}; std::uintptr_t arena{}; std::uint64_t capacity{};
    if (!list || list > UINTPTR_MAX-0x42 || !ReadMemory(list+0x18,&count,4) || count > 1024) return false;
    if (!count) return true;
    if (!ReadMemory(list+0x30,&arena,8) || !arena || !ReadMemory(list+0x38,&capacity,8) || !capacity ||
        capacity > (UINTPTR_MAX-arena)/0x800 || !ReadMemory(list+0x40,&index,2) || !index) return false;
    std::array<std::uint16_t,22> visited{}; std::size_t blocks{},offset{}; std::uintptr_t block{};
    const auto select=[&]() {
        if (!index || index > capacity || blocks >= visited.size() ||
            std::find(visited.begin(),visited.begin()+blocks,index) != visited.begin()+blocks) return false;
        visited[blocks++]=index; block=arena+std::uintptr_t(index-1)*0x800; offset=0; return true;
    };
    if (!select()) return false;
    std::vector<SFMapMarkerV1> pending;
    for (std::uint32_t i=0;i<count;++i) {
        if (offset+0x28 > 0x7f8 && (!ReadMemory(block+2,&index,2) || !select())) return false;
        std::uint8_t event[0x28]{}; SFMapMarkerV1 ping{}; NativeTransform transform{};
        if (!ReadMemory(block+8+offset,event,sizeof(event))) return false;
        offset+=sizeof(event); std::uint32_t id{}; std::memcpy(&id,event+8,4);
        std::memcpy(transform.xyz,event+0x10,sizeof(transform.xyz)); transform.quaternion[3]=1;
        SFMapPoseV1 pose{},camera{};
        if (!id || id > 1023 || id == excluded || !DecodeLocalTransforms(transform,transform,pose,camera)) continue;
        std::uint64_t identity_words[5]{};std::memcpy(identity_words,event,sizeof(identity_words));SetMarkerIdentity(ping,identity_words);
        ping.id=id; std::copy(std::begin(pose.position),std::end(pose.position),ping.position);
        ping.flags=SF_MAP_MARKER_VISIBLE; ping.expires_at_ms=now > UINT64_MAX-8000 ? UINT64_MAX : now+8000;
        pending.push_back(ping);
    }
    result=std::move(pending); return true;
}

bool ReadDiscoveryMarker(std::uintptr_t address,SFMapMarkerV1& marker,bool& positioned,DiscoveryIdentity* identity) {
    std::uintptr_t record[10]{};std::uint64_t words[5]{};std::uint8_t visibility{};positioned=false;
    if(!ReadMemory(address,record,sizeof(record)) || !ReadMemory(record[1],words,sizeof(words)) || !ReadMemory(record[2],&visibility,1))return false;
    if(identity)*identity={record[1],record[9]};
    marker={};SetMarkerIdentity(marker,words);marker.kind=std::uint32_t(words[2]);
    marker.flags=visibility>=2 ? SF_MAP_MARKER_VISIBLE|SF_MAP_MARKER_DISCOVERED:0;
    if(!visibility)return true;
    std::size_t best=SIZE_MAX;
    const auto scan=[&](std::uintptr_t root,std::size_t bytes) {
        if(root<0x10000 || root>UINT64_C(0x7fffffffffff))return;
        for(std::size_t offset=0;offset+40<=bytes;offset+=8) {
            std::int64_t raw[5]{};if(!ReadMemory(root+offset,raw,sizeof(raw)))continue;
            for(unsigned first=0;first<=2;++first) {
                float xyz[3]{};for(unsigned c=0;c<3;++c)xyz[c]=float(double(raw[first+c])/4294967296.0);
                if(!std::isfinite(xyz[0]) || !std::isfinite(xyz[1]) || !std::isfinite(xyz[2]) ||
                    xyz[0]<=1 || xyz[2]<=1 || xyz[0]>=10304 || xyz[2]>=10304 || xyz[1]<=-2000 || xyz[1]>=5000)continue;
                if(offset<best){best=offset;std::copy(std::begin(xyz),std::end(xyz),marker.position);positioned=true;}break;
            }
        }
    };
    scan(record[1],0x100);scan(record[9],0x180);scan(record[0],0x100);scan(record[2],0x60);return true;
}

namespace {
constexpr std::size_t CameraEntryRva = 0x2b2b60;
constexpr std::size_t CameraEntrySize = 18;
constexpr std::array<std::uint8_t, CameraEntrySize> CameraEntryOriginal = {
    0x40, 0x55, 0x57, 0x48, 0x8d, 0xac, 0x24, 0xb8, 0xfe,
    0xff, 0xff, 0x48, 0x81, 0xec, 0x48, 0x02, 0x00, 0x00,
};
constexpr std::array<std::uint8_t, 26> EntryRelayPrefix = {
    0x48, 0x83, 0xec, 0x48, 0x48, 0x89, 0x4c, 0x24, 0x20,
    0x48, 0x89, 0x54, 0x24, 0x28, 0x4c, 0x89, 0x44, 0x24,
    0x30, 0x4c, 0x89, 0x4c, 0x24, 0x38, 0x48, 0xb8,
};
constexpr std::array<std::uint8_t, 26> EntryRelaySuffix = {
    0xff, 0xd0, 0x48, 0x8b, 0x4c, 0x24, 0x20, 0x48, 0x8b,
    0x54, 0x24, 0x28, 0x4c, 0x8b, 0x44, 0x24, 0x30, 0x4c,
    0x8b, 0x4c, 0x24, 0x38, 0x48, 0x83, 0xc4, 0x48,
};
constexpr std::size_t EntryRelayCallbackOffset = EntryRelayPrefix.size();
constexpr std::size_t EntryRelaySuffixOffset = EntryRelayCallbackOffset + sizeof(std::uintptr_t);
constexpr std::size_t EntryRelayOriginalOffset = EntryRelaySuffixOffset + EntryRelaySuffix.size();
constexpr std::size_t EntryRelayTailOffset = EntryRelayOriginalOffset + CameraEntrySize;
constexpr std::size_t EntryRelaySize = EntryRelayTailOffset + 5;

void __fastcall camera_callback(void* ctx, void*, void*, void*) noexcept;

using IterInit=void(__fastcall*)(void*,void*,std::uint32_t);
using IterNext=bool(__fastcall*)(void*,void*,std::uint32_t);
using GetWorld=std::uintptr_t(__fastcall*)(std::uintptr_t);
using Lookup=void(__fastcall*)(std::uintptr_t*,std::uintptr_t,std::uintptr_t,std::uint32_t);
using WeatherSample=float*(__fastcall*)(float*,void*,const void*,std::int64_t);
using FogUpdate=void(__fastcall*)(void*,const float*,float);
SnapshotStore store;
std::recursive_mutex control_mutex, transient_mutex;
std::atomic<bool> accepting{};
std::atomic<bool> deaths_enabled{};
std::atomic<std::uint64_t> capture_generation{}, last_fog{}, last_remote{}, last_world{}, last_clock{}, last_weather{}, last_discovery{};
std::unordered_set<std::uint64_t> owners;
std::uintptr_t image_base{};
IterInit iter_init{}; IterNext iter_next{}; GetWorld get_world{}; Lookup lookup{};
WeatherSample weather_sample{}; FogUpdate fog_update{};
MapImageAdmission admission;
std::uint64_t source_session{}, next_generation{};
std::vector<SFMapMarkerV1> pings;
struct DiscoveryEntry {DiscoveryIdentity identity;SFMapMarkerV1 value;};
std::vector<DiscoveryEntry> discoveries;
struct InstalledCapture { std::uintptr_t target{}; std::vector<std::uint8_t> original, replacement, relay;std::uintptr_t allocation{}; bool installed{}; };
std::vector<InstalledCapture> hooks;

std::vector<std::uint8_t> make_entry_relay(
    std::span<const std::uint8_t> original, void* callback) {
    std::vector<std::uint8_t> code(EntryRelayPrefix.begin(), EntryRelayPrefix.end());
    const auto handler = reinterpret_cast<std::uintptr_t>(callback);
    for (unsigned i = 0; i < sizeof(handler); ++i) {
        code.push_back(static_cast<std::uint8_t>(handler >> (i * 8)));
    }
    code.insert(code.end(), EntryRelaySuffix.begin(), EntryRelaySuffix.end());
    code.insert(code.end(), original.begin(), original.end());
    code.insert(code.end(), {0xe9, 0, 0, 0, 0});
    return code;
}

bool replacement_matches_allocation(const InstalledCapture& hook) noexcept {
    if (!hook.target || !hook.allocation || hook.replacement.size() != CameraEntrySize) return false;
    std::array<std::uint8_t, CameraEntrySize> expected{};
    expected.fill(0x90);
    expected[0] = 0xe9;
    std::int32_t displacement{};
    if (!EngineHooks::EncodeRelativeJump(hook.target, hook.allocation, displacement)) return false;
    std::memcpy(expected.data() + 1, &displacement, sizeof(displacement));
    return std::equal(expected.begin(), expected.end(), hook.replacement.begin());
}

bool relay_matches_camera_callback(
    const InstalledCapture& hook,
    const std::array<std::uint8_t, CameraEntrySize>& expectedOriginal,
    std::span<const std::uint8_t> liveRelay) noexcept {
    if (hook.relay.size() != EntryRelaySize || liveRelay.size() != EntryRelaySize ||
        std::memcmp(hook.relay.data(), EntryRelayPrefix.data(), EntryRelayPrefix.size()) != 0 ||
        std::memcmp(hook.relay.data() + EntryRelaySuffixOffset,
            EntryRelaySuffix.data(), EntryRelaySuffix.size()) != 0 ||
        std::memcmp(hook.relay.data() + EntryRelayOriginalOffset,
            expectedOriginal.data(), expectedOriginal.size()) != 0 ||
        hook.relay[EntryRelayTailOffset] != 0xe9 ||
        std::memcmp(liveRelay.data(), hook.relay.data(), hook.relay.size()) != 0) {
        return false;
    }

    std::uintptr_t handler{};
    std::memcpy(&handler, hook.relay.data() + EntryRelayCallbackOffset, sizeof(handler));
    if (handler != reinterpret_cast<std::uintptr_t>(&camera_callback)) return false;

    std::int32_t tail_displacement{};
    std::memcpy(&tail_displacement, hook.relay.data() + EntryRelayTailOffset + 1,
        sizeof(tail_displacement));
    if (hook.allocation > UINTPTR_MAX - EntryRelaySize ||
        hook.target > UINTPTR_MAX - expectedOriginal.size()) {
        return false;
    }
    const auto tail_end = hook.allocation + EntryRelaySize;
    const auto expected_return = hook.target + expectedOriginal.size();
    return static_cast<std::int64_t>(tail_end) + tail_displacement ==
            static_cast<std::int64_t>(expected_return);
}

bool camera_entry_record_matches(
    const InstalledCapture& hook,
    std::uintptr_t base,
    const std::array<std::uint8_t, CameraEntrySize>& expectedOriginal,
    const std::array<std::uint8_t, CameraEntrySize>& liveEntry,
    const std::array<std::uint8_t, EntryRelaySize>& liveRelay) noexcept {
    return hook.installed && hook.target == base + CameraEntryRva &&
        hook.original.size() == expectedOriginal.size() &&
        std::equal(expectedOriginal.begin(), expectedOriginal.end(), hook.original.begin()) &&
        replacement_matches_allocation(hook) &&
        std::equal(liveEntry.begin(), liveEntry.end(), hook.replacement.begin()) &&
        relay_matches_camera_callback(hook, expectedOriginal, liveRelay);
}

template<class T> bool read(std::uintptr_t address, T& out) { return ReadMemory(address,&out,sizeof(out)); }
bool initialize(void* context, void* record, std::uint32_t size) {
    __try { iter_init(context,record,size); return true; }
    __except(EXCEPTION_EXECUTE_HANDLER) { return false; }
}
bool advance(void* context, void* record, std::uint32_t size) {
    __try { return iter_next(context,record,size); }
    __except(EXCEPTION_EXECUTE_HANDLER) { return false; }
}
bool component(std::uintptr_t context, std::uintptr_t lookup_context, std::uint32_t id, std::uintptr_t& result) {
    __try {
        const auto world=get_world(context); if (!world) return false;
        std::uintptr_t out[2]{}; lookup(out,world,lookup_context,id); result=out[0]; return result != 0;
    } __except(EXCEPTION_EXECUTE_HANDLER) { return false; }
}
bool throttle(std::atomic<std::uint64_t>& last, std::uint64_t now, unsigned interval) {
    auto previous=last.load(std::memory_order_relaxed);
    return now >= previous && now-previous >= interval && last.compare_exchange_strong(previous,now);
}
void __fastcall camera_callback(void* ctx, void*, void*, void*) noexcept {
    if (!accepting.load(std::memory_order_acquire)) return;
    try {
        const auto generation=capture_generation.load();
        std::uintptr_t context[2]{}; NativeCameraRecord record{};
        if (!read(reinterpret_cast<std::uintptr_t>(ctx),context) || !initialize(context,&record,sizeof(record))) return;
        for (unsigned attempts=0;attempts<256 && advance(context,&record,sizeof(record));++attempts) {
            const auto index=std::uint32_t(context[1]); std::uintptr_t entities{}; std::uint32_t entity{};
            if (!index || index > 256 || !read(context[0]+0x828,entities) ||
                !read(entities+std::uintptr_t(index-1)*0x10,entity)) return;
            SFMapPoseV1 player{},camera{};
            if (ReadLocalCamera(record,entity,player,camera)) {
                store.PublishPlayer(generation,player,camera,GetTickCount64()); return;
            }
        }
    } catch (...) {}
}
void __fastcall remote_callback(void* ctx, void*, void*, void*) noexcept {
    if (!accepting.load(std::memory_order_acquire)) return;
    const auto generation=capture_generation.load(), now=GetTickCount64();
    if (!throttle(last_remote,now,100)) return;
    try {
        std::uintptr_t context[2]{},record[19]{}; std::uint32_t local{};
        if (!read(reinterpret_cast<std::uintptr_t>(ctx),context) || !initialize(context,record,sizeof(record)) ||
            !record[1] || !record[2] || !record[6] || !read(record[2]+4,local) || !local || local > 1023) return;
        std::vector<SFMapMarkerV1> markers;
        for (std::uint32_t id=1;id<=1023 && markers.size()<32;++id) {
            std::uint32_t slot{}; std::uintptr_t address{}; NativeTransform transform{};
            if (id==local || !read(record[1]+std::uintptr_t(id-1)*0xa0,slot) || slot != id ||
                !component(context[0],record[6],id,address) || !read(address,transform)) continue;
            SFMapPoseV1 pose{},camera{};
            if (!DecodeLocalTransforms(transform,transform,pose,camera)) continue;
            SFMapMarkerV1 marker{}; marker.id=id; marker.heading_radians=pose.heading_radians;
            marker.flags=SF_MAP_MARKER_VISIBLE | SF_MAP_MARKER_HEADING;
            std::copy(std::begin(pose.position),std::end(pose.position),marker.position); markers.push_back(marker);
        }
        store.PublishMarkers(Collection::RemotePlayers,generation,std::move(markers),now);
    } catch (...) {}
}
void __fastcall waypoints_callback(void* ctx, void*, void*, void*) noexcept {
    if (!accepting.load(std::memory_order_acquire)) return;
    const auto generation=capture_generation.load(), now=GetTickCount64();
    try {
        std::uintptr_t context[2]{},record[6]{}; std::uint32_t local{};
        if (!read(reinterpret_cast<std::uintptr_t>(ctx),context) || !initialize(context,record,sizeof(record)) ||
            !read(record[1]+4,local) || !local || local>1023) return;
        std::vector<SFMapMarkerV1> input,received;
        const bool valid_received=ReadPingEvents(record[4],local,now,received);
        const bool valid_input=ReadPingEvents(record[5],0,now,input);
        if (valid_received && valid_input) {
            std::lock_guard lock(transient_mutex);
            if (capture_generation.load() == generation) {
                std::erase_if(pings,[&](const auto& ping){ return now >= ping.expires_at_ms; });
                for (const auto& list:{&received,&input}) for (const auto& ping:*list) {
                    const auto existing=std::find_if(pings.begin(),pings.end(),[&](const auto& value){return value.id==ping.id;});
                    if (existing != pings.end()) *existing=ping; else if (pings.size()<32) pings.push_back(ping);
                }
                store.PublishMarkers(Collection::Pings,generation,pings,now);
            }
        }
        std::vector<SFMapMarkerV1> waypoints; std::uintptr_t address{}; std::uint8_t enabled{};
        if (!component(context[0],record[3],local,address) || !read(address,enabled)) return;
        if (enabled) {
            NativeTransform transform{}; SFMapPoseV1 pose{},camera{}; transform.quaternion[3]=1;
            if (!ReadMemory(address+8,transform.xyz,sizeof(transform.xyz)) || !DecodeLocalTransforms(transform,transform,pose,camera)) return;
            SFMapMarkerV1 marker{}; marker.id=local; marker.kind=10; marker.flags=SF_MAP_MARKER_VISIBLE;
            std::copy(std::begin(pose.position),std::end(pose.position),marker.position); waypoints.push_back(marker);
        }
        store.PublishMarkers(Collection::Waypoints,generation,std::move(waypoints),now);
    } catch (...) {}
}
void __fastcall clock_callback(void* ctx, void*, void*, void*) noexcept {
    if (!accepting.load(std::memory_order_acquire)) return;
    const auto generation=capture_generation.load(),now=GetTickCount64();
    if (!throttle(last_clock,now,100)) return;
    try {
        std::uintptr_t context[2]{},record[5]{}; std::int64_t fields[5]{}; SFMapEnvironmentV1 environment{};
        if (read(reinterpret_cast<std::uintptr_t>(ctx),context) && initialize(context,record,sizeof(record)) &&
            read(record[2]+0x28,fields) && DecodeClock(fields,environment)) store.PublishEnvironment(generation,environment,true,now);
    } catch (...) {}
}
float* __fastcall weather_callback(float* output,void* weather,const void* position,std::int64_t time) noexcept {
    const auto generation=capture_generation.load();
    auto* result=weather_sample(output,weather,position,time);
    if (!accepting.load(std::memory_order_acquire)) return result;
    const auto now=GetTickCount64(); if (!throttle(last_weather,now,100)) return result;
    try { float weights[4]{}; SFMapEnvironmentV1 environment{};
        if (read(reinterpret_cast<std::uintptr_t>(result),weights) && DecodeWeather(weights,environment))
            store.PublishEnvironment(generation,environment,false,now);
    } catch (...) {}
    return result;
}
void __fastcall fog_callback(void* fog,const float* position,float radius) noexcept {
    const auto generation=capture_generation.load();
    fog_update(fog,position,radius);
    if (!accepting.load(std::memory_order_acquire)) return;
    const auto now=GetTickCount64(); if (!throttle(last_fog,now,500)) return;
    try { auto snapshot=std::make_shared<FogSnapshot>();
        if (ReadFog(reinterpret_cast<std::uintptr_t>(fog),*snapshot)) store.PublishFog(generation,std::move(snapshot),now);
    } catch (...) {}
}
bool world_array(std::uintptr_t state,std::size_t offset,bool custom,std::vector<SFMapMarkerV1>& markers,unsigned maximum=2048) {
    std::uintptr_t entries{}; std::uint64_t count{};
    if (!state || !read(state+offset,entries) || !read(state+offset+8,count) || count>maximum) return false;
    std::vector<std::uint8_t> bytes(std::size_t(count)*0xa0);
    if (count && !ReadMemory(entries,bytes.data(),bytes.size())) return false;
    return DecodeWorldMarkers(bytes,std::size_t(count),custom,markers);
}
void __fastcall world_callback(void* ctx,void*,void*,void*) noexcept {
    if (!accepting.load(std::memory_order_acquire)) return;
    const auto generation=capture_generation.load(),now=GetTickCount64(); if (!throttle(last_world,now,100)) return;
    try {
        std::uintptr_t context[2]{},record[2]{};
        if (!read(reinterpret_cast<std::uintptr_t>(ctx),context) || !initialize(context,record,sizeof(record))) return;
        std::vector<SFMapMarkerV1> world,custom,deaths;
        if (world_array(record[1],0x396a28,true,custom) && world_array(record[1],0x36ea00,false,world)) {
            world.insert(world.end(),custom.begin(),custom.end()); store.PublishMarkers(Collection::World,generation,std::move(world),now);
        }
        if (deaths_enabled.load() && world_array(record[1],0x3bea50,false,deaths,64))
            store.PublishMarkers(Collection::Deaths,generation,std::move(deaths),now);
    } catch (...) {}
}

void __fastcall discovery_callback(void* record,void*) noexcept {
    if(!accepting.load(std::memory_order_acquire))return;
    const auto generation=capture_generation.load(),now=GetTickCount64();
    try {
        SFMapMarkerV1 marker{};bool positioned{};DiscoveryIdentity identity{};if(!ReadDiscoveryMarker(reinterpret_cast<std::uintptr_t>(record),marker,positioned,&identity))return;
        std::lock_guard lock(transient_mutex);if(generation!=capture_generation.load())return;
        std::erase_if(discoveries,[&](const auto& item){return now>=item.value.expires_at_ms;});
        const auto found=std::find_if(discoveries.begin(),discoveries.end(),[&](const auto& item){return item.identity==identity;});
        if(!(marker.flags&SF_MAP_MARKER_VISIBLE)){if(found!=discoveries.end())discoveries.erase(found);}
        else {
            if(!positioned && found!=discoveries.end()){std::copy(std::begin(found->value.position),std::end(found->value.position),marker.position);positioned=true;}
            if(positioned){marker.expires_at_ms=now+180000;if(found!=discoveries.end())found->value=marker;else if(discoveries.size()<4096)discoveries.push_back({identity,marker});}
        }
        if(throttle(last_discovery,now,100)){std::vector<SFMapMarkerV1> visible;visible.reserve(discoveries.size());for(const auto& item:discoveries)visible.push_back(item.value);store.PublishMarkers(Collection::Discovery,generation,std::move(visible),now);}
    }catch(...){}
}
// Capture uses the existing owner/primitives. Published thunks and the pinned
// callback module remain mapped after restoration, including partial failure.
bool publish_hook(std::uintptr_t target,std::span<const std::uint8_t> original,
    std::vector<std::uint8_t> payload,bool call) {
    if (original.size()<5 || original.size()>32) return false;
    auto* allocation=static_cast<std::uint8_t*>(EngineHooks::AllocateNearCode(target,payload.size()));
    if (!allocation) return false;
    if (!call) {
        std::int32_t back{};
        if (!EngineHooks::EncodeRelativeJump(reinterpret_cast<std::uintptr_t>(allocation)+payload.size()-5,target+original.size(),back)) {
            VirtualFree(allocation,0,MEM_RELEASE); return false;
        }
        std::memcpy(payload.data()+payload.size()-4,&back,4);
    }
    std::memcpy(allocation,payload.data(),payload.size()); DWORD previous{};
    if (!VirtualProtect(allocation,payload.size(),PAGE_EXECUTE_READ,&previous)) { VirtualFree(allocation,0,MEM_RELEASE); return false; }
    FlushInstructionCache(GetCurrentProcess(),allocation,payload.size());
    InstalledCapture hook; hook.allocation=reinterpret_cast<std::uintptr_t>(allocation);hook.relay=payload;hook.target=target; hook.original.assign(original.begin(),original.end());
    hook.replacement.assign(original.size(),0x90); hook.replacement[0]=call ? 0xe8 : 0xe9;
    std::int32_t displacement{};
    if (!EngineHooks::EncodeRelativeJump(target,reinterpret_cast<std::uintptr_t>(allocation),displacement)) {
        VirtualFree(allocation,0,MEM_RELEASE); return false;
    }
    std::memcpy(hook.replacement.data()+1,&displacement,4);
    hooks.push_back(std::move(hook)); auto& stored=hooks.back();
    if (!EngineHooks::ReplaceExecutableBytes(target,stored.original.data(),stored.replacement.data(),stored.original.size())) {
        // The transaction restored the preimage before returning false.
        VirtualFree(allocation,0,MEM_RELEASE); hooks.pop_back(); return false;
    }
    stored.installed=true; return true;
}
bool entry_hook(std::size_t rva,std::span<const std::uint8_t> original,void* callback) {
    return publish_hook(image_base+rva,original,make_entry_relay(original,callback),false);
}
bool call_hook(std::size_t rva,std::size_t original_target,void* callback) {
    std::uint8_t original[5]{}; std::int32_t offset{};
    if (!ReadMemory(image_base+rva,original,5) || original[0]!=0xe8) return false;
    std::memcpy(&offset,original+1,4);
    if (std::int64_t(rva)+5+offset != std::int64_t(original_target)) return false;
    std::vector<std::uint8_t> relay{0xff,0x25,0,0,0,0}; const auto handler=reinterpret_cast<std::uintptr_t>(callback);
    for (unsigned i=0;i<8;++i) relay.push_back(std::uint8_t(handler>>(i*8)));
    return publish_hook(image_base+rva,original,std::move(relay),true);
}
bool discovery_hook() {
    constexpr std::uint8_t original[]{0x41,0xb8,0x50,0,0,0};
    std::vector<std::uint8_t> code{0x48,0x83,0xec,0x50,0x48,0x89,0x4c,0x24,0x20,0x48,0x89,0x54,0x24,0x28,
        0x4c,0x89,0x44,0x24,0x30,0x4c,0x89,0x4c,0x24,0x38,0x48,0xb8};
    const auto handler=reinterpret_cast<std::uintptr_t>(&discovery_callback);for(unsigned i=0;i<8;++i)code.push_back(std::uint8_t(handler>>(i*8)));
    code.insert(code.end(),{0x48,0x8d,0x4d,0xe7,0x4c,0x89,0xf2,0xff,0xd0,0x48,0x8b,0x4c,0x24,0x20,
        0x48,0x8b,0x54,0x24,0x28,0x4c,0x8b,0x44,0x24,0x30,0x4c,0x8b,0x4c,0x24,0x38,0x48,0x83,0xc4,0x50});
    code.insert(code.end(),std::begin(original),std::end(original));code.insert(code.end(),{0xe9,0,0,0,0});
    return publish_hook(image_base+0x4153a2,original,std::move(code),false);
}
void restore_hooks() {
    for (auto it=hooks.rbegin();it!=hooks.rend();++it) if (it->installed &&
        EngineHooks::ReplaceExecutableBytes(it->target,it->replacement.data(),it->original.data(),it->original.size())) it->installed=false;
}
}
SnapshotStore& CapturedFrames() { return store; }
bool RendererAdmitted() {std::lock_guard lock(control_mutex);return accepting.load() && admission.renderer;}
bool InstallSharedEntry(std::size_t rva,std::span<const std::uint8_t> expected,void* callback) {
    std::lock_guard lock(control_mutex);return accepting.load() && callback && entry_hook(rva,expected,callback);
}
bool ReadOwnedCameraEntryReplacement(
    const std::array<std::uint8_t, CameraEntrySize>& expected_original,
    std::array<std::uint8_t, CameraEntrySize>& replacement) {
    std::unique_lock<std::recursive_mutex> lock(control_mutex, std::try_to_lock);
    if (!lock.owns_lock() || !image_base ||
        expected_original != CameraEntryOriginal ||
        image_base > UINTPTR_MAX - CameraEntryRva) {
        return false;
    }

    const auto target = image_base + CameraEntryRva;
    const InstalledCapture* owned{};
    for (const auto& hook : hooks) {
        if (hook.target != target) continue;
        if (owned) return false;
        owned = &hook;
    }
    if (!owned || !owned->installed || owned->relay.size() != EntryRelaySize) return false;

    std::array<std::uint8_t, CameraEntrySize> live_entry{};
    std::array<std::uint8_t, EntryRelaySize> live_relay{};
    if (!ReadMemory(owned->target, live_entry.data(), live_entry.size()) ||
        !ReadMemory(owned->allocation, live_relay.data(), live_relay.size()) ||
        !camera_entry_record_matches(*owned, image_base, expected_original,
            live_entry, live_relay)) {
        return false;
    }

    std::copy(owned->replacement.begin(), owned->replacement.end(), replacement.begin());
    return true;
}
bool InstallSharedCall(std::size_t rva,std::size_t target,void* callback) {
    std::lock_guard lock(control_mutex);return accepting.load() && callback && call_hook(rva,target,callback);
}
bool SharedCallOwned(std::size_t rva,std::size_t target,void* callback) {
    std::lock_guard lock(control_mutex);
    for(const auto& hook:hooks)if(hook.target==image_base+rva && hook.installed && hook.relay.size()==14 && hook.original.size()==5) {
        std::uint8_t bytes[14]{},actual[5]{};std::int32_t relative{};std::uintptr_t handler{};
        std::memcpy(&relative,hook.original.data()+1,4);std::memcpy(&handler,hook.relay.data()+6,8);
        return hook.original[0]==0xe8 && std::int64_t(rva)+5+relative==std::int64_t(target) && handler==reinterpret_cast<std::uintptr_t>(callback) &&
            ReadMemory(hook.target,actual,5) && !std::memcmp(actual,hook.replacement.data(),5) &&
            ReadMemory(hook.allocation,bytes,14) && !std::memcmp(bytes,hook.relay.data(),14);
    }return false;
}
std::int32_t EnableCapture(std::uint64_t owner,bool capability) {
    if (!owner || !capability) return SF_MAP_CAPABILITY_DENIED;
    std::lock_guard control(control_mutex);
    if (owners.contains(owner)) return SF_MAP_INVALID_ARGUMENT;
    if (!owners.empty()) { owners.insert(owner); return SF_MAP_READY; }
    if (std::any_of(hooks.begin(),hooks.end(),[](const auto& hook){return hook.installed;})) return SF_MAP_FAILED;
    namespace Profile=KfcRuntimeCompatibility::EnshroudedClient;
    image_base=reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
    admission=InspectMapImage({reinterpret_cast<const std::uint8_t*>(image_base),Profile::image_size},Profile::image_sha256,capability);
    if (admission.status != SF_MAP_READY) return admission.status;
    HMODULE pinned{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
        reinterpret_cast<LPCWSTR>(&EnableCapture),&pinned)) return SF_MAP_FAILED;
    if (!iter_init) {
        iter_init=reinterpret_cast<IterInit>(image_base+0x9bc1a0); iter_next=reinterpret_cast<IterNext>(image_base+0x9b6e60);
        get_world=reinterpret_cast<GetWorld>(image_base+0x9b43d0); lookup=reinterpret_cast<Lookup>(image_base+0x9a7200);
        weather_sample=reinterpret_cast<WeatherSample>(image_base+0xa9d530); fog_update=reinterpret_cast<FogUpdate>(image_base+0x39c430);
    }
    if (!entry_hook(CameraEntryRva,CameraEntryOriginal,reinterpret_cast<void*>(&camera_callback))) { restore_hooks(); return SF_MAP_FAILED; }
    constexpr std::uint8_t waypoint[]{0x48,0x89,0x5c,0x24,0x18,0x48,0x89,0x6c,0x24,0x20};
    constexpr std::uint8_t remote[]{0x48,0x89,0x4c,0x24,0x08};
    const auto marker_hooks_start=hooks.size();
    admission.markers=admission.markers && entry_hook(0x30eb10,waypoint,reinterpret_cast<void*>(&waypoints_callback)) &&
        entry_hook(0x317e80,remote,reinterpret_cast<void*>(&remote_callback));
    if (!admission.markers) for (auto i=marker_hooks_start;i<hooks.size();++i) {
        auto& hook=hooks[i];
        if (hook.installed && EngineHooks::ReplaceExecutableBytes(hook.target,hook.replacement.data(),hook.original.data(),hook.original.size())) hook.installed=false;
        if (hook.installed) { restore_hooks(); return SF_MAP_FAILED; }
    }
    constexpr std::uint8_t clock[]{0x41,0x56,0x48,0x83,0xec,0x60,0x41,0xb8,0x28,0,0,0,0x48,0x8d,0x54,0x24,0x30};
    admission.clock=admission.clock && entry_hook(0x2b3340,clock,reinterpret_cast<void*>(&clock_callback));
    admission.weather=admission.weather && call_hook(0x2fdd18,0xa9d530,reinterpret_cast<void*>(&weather_callback));
    admission.fog=admission.fog && call_hook(0x3072a4,0x39c430,reinterpret_cast<void*>(&fog_callback));
    constexpr std::uint8_t world[]{0x48,0x83,0xec,0x38,0x41,0xb8,0x10,0,0,0,0x48,0x8d,0x54,0x24,0x20};
    admission.world=admission.world && entry_hook(0x2fefe0,world,reinterpret_cast<void*>(&world_callback));
    admission.deaths=admission.world && admission.deaths;
    deaths_enabled.store(admission.deaths);
    // HUD/Creative ownership and discovery loop publication join the renderer
    // integration stage. Do not advertise a capture that has no installed owner.
    admission.discovery=admission.discovery && discovery_hook();
    capture_generation.store(0); source_session=0; store.SetAdmission(admission.Features());
    owners.insert(owner); accepting.store(true,std::memory_order_release);
    if(EnableHudVisibility(image_base,KfcRuntimeCompatibility::EnshroudedClient::image_size))store.SetAdmission(admission.Features()|SF_MAP_FEATURE_HUD);
    else OutputDebugStringA("[ESM map] HUD unavailable: renderer ownership/call-site guards or safe patch admission failed; gameplay visibility remains closed\n");
    return SF_MAP_READY;
}
void DisableCapture(std::uint64_t owner) {
    std::lock_guard control(control_mutex);
    if (!owners.erase(owner) || !owners.empty()) return;
    DisableHudVisibility();accepting.store(false,std::memory_order_release); capture_generation.store(0); store.Disable();
    restore_hooks(); std::lock_guard transient(transient_mutex); pings.clear(); discoveries.clear(); source_session=0;
}
void UpdateCaptureSession(std::uint64_t session,bool gameplay,bool menu,bool focused) {
    std::lock_guard control(control_mutex);
    if (!accepting.load()) return;
    if (source_session != session) {
        capture_generation.store(0); source_session=session;
        const auto next=session ? ++next_generation : 0;
        store.SetSession(next,gameplay,menu,focused,GetTickCount64());
        std::lock_guard transient(transient_mutex); pings.clear(); discoveries.clear();
        capture_generation.store(next);
        last_fog=last_remote=last_world=last_clock=last_weather=last_discovery=0;
    } else store.SetSession(capture_generation.load(),gameplay,menu,focused,GetTickCount64());
}
}
