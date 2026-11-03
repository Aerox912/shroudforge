#include "map/capture_game_map.h"
#include "map/capture_game_map.cpp"
#include "map/menu_visibility_1_0.h"
#include "engine-hooks/write_executable_bytes.h"
#include <windows.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <source_location>
#include <thread>

namespace {
unsigned checks{};
void check(bool ok, std::source_location at=std::source_location::current()) {
    ++checks; if (!ok) { std::fprintf(stderr,"map admission/capture check %u failed at %u\n",checks,at.line()); std::exit(1); }
}
void check(bool ok, const char* message, std::source_location at=std::source_location::current()) {
    ++checks; if (!ok) { std::fprintf(stderr,"map admission/capture check %u failed at %u: %s\n",checks,at.line(),message); std::exit(1); }
}
DWORD WINAPI private_loop(void* page) { reinterpret_cast<void(*)()>(page)(); return 0; }
void mixed_admission_statuses() {
    using namespace MapRuntime;
    struct Case { bool clock,weather,world,deaths; };
    for (const auto test : {Case{true,false,true,false}, Case{false,true,true,false},
            Case{true,true,true,true}, Case{false,false,false,false}}) {
        MapImageAdmission admission{}; admission.player=true;
        admission.clock=test.clock; admission.weather=test.weather;
        admission.world=test.world; admission.deaths=test.deaths;
        SnapshotStore store; store.SetAdmission(admission.Features());
        store.SetSession(1,true,false,true,100);
        auto frame=store.Acquire(1,101).frame;
        check(frame.environment.clock_meta.status == (test.clock ? SF_MAP_NOT_CAPTURED : SF_MAP_UNSUPPORTED_BUILD));
        check(frame.environment.weather_meta.status == (test.weather ? SF_MAP_NOT_CAPTURED : SF_MAP_UNSUPPORTED_BUILD));
        check(frame.world_markers.meta.status == (test.world ? SF_MAP_NOT_CAPTURED : SF_MAP_UNSUPPORTED_BUILD));
        check(frame.deaths.meta.status == (test.deaths ? SF_MAP_NOT_CAPTURED : SF_MAP_UNSUPPORTED_BUILD));
        SFMapEnvironmentV1 environment{}; environment.minute_of_day=720; environment.daytime=1;
        check(store.PublishEnvironment(1,environment,true,100) == test.clock);
        check(store.PublishEnvironment(1,environment,false,100) == test.weather);
        check(store.PublishMarkers(Collection::World,1,{},100) == test.world);
        check(store.PublishMarkers(Collection::Deaths,1,{},100) == test.deaths);
        frame=store.Acquire(1,101).frame;
        check(frame.environment.clock_meta.status == (test.clock ? SF_MAP_READY : SF_MAP_UNSUPPORTED_BUILD));
        check(frame.environment.weather_meta.status == (test.weather ? SF_MAP_READY : SF_MAP_UNSUPPORTED_BUILD));
        check(frame.world_markers.meta.status == (test.world ? SF_MAP_READY : SF_MAP_UNSUPPORTED_BUILD));
        check(frame.deaths.meta.status == (test.deaths ? SF_MAP_READY : SF_MAP_UNSUPPORTED_BUILD));
        check(frame.world_markers.count==0 && frame.deaths.count==0);
    }
}

void camera_entry_hook_ownership() {
    using namespace MapRuntime;
    const auto previous_base = image_base;
    auto previous_hooks = std::move(hooks);
    std::vector<std::uint8_t> image(CameraEntryRva + CameraEntrySize);
    std::vector<std::uint8_t> live_relay(EntryRelaySize);
    image_base = reinterpret_cast<std::uintptr_t>(image.data());
    hooks.clear();

    std::array<std::uint8_t, CameraEntrySize> replacement{};
    image[CameraEntryRva] = 0xe9;
    check(!ReadOwnedCameraEntryReplacement(CameraEntryOriginal, replacement),
        "an unowned E9 camera entry is not treated as the Minimap hook");

    auto install_owned_record = [&] {
        hooks.clear();
        InstalledCapture hook;
        hook.target = image_base + CameraEntryRva;
        hook.allocation = reinterpret_cast<std::uintptr_t>(live_relay.data());
        hook.original.assign(CameraEntryOriginal.begin(), CameraEntryOriginal.end());
        hook.relay = make_entry_relay(CameraEntryOriginal, reinterpret_cast<void*>(&camera_callback));
        std::int32_t tail{};
        check(EngineHooks::EncodeRelativeJump(hook.allocation + EntryRelaySize - 5,
            hook.target + CameraEntrySize, tail), "the owned relay has a representable return edge");
        std::memcpy(hook.relay.data() + EntryRelayTailOffset + 1, &tail, sizeof(tail));
        std::copy(hook.relay.begin(), hook.relay.end(), live_relay.begin());
        hook.replacement.assign(CameraEntrySize, 0x90);
        hook.replacement[0] = 0xe9;
        std::int32_t entry{};
        check(EngineHooks::EncodeRelativeJump(hook.target, hook.allocation, entry),
            "the owned replacement has a representable relay edge");
        std::memcpy(hook.replacement.data() + 1, &entry, sizeof(entry));
        std::copy(hook.replacement.begin(), hook.replacement.end(), image.begin() + CameraEntryRva);
        hook.installed = true;
        hooks.push_back(std::move(hook));
    };

    install_owned_record();
    check(ReadOwnedCameraEntryReplacement(CameraEntryOriginal, replacement),
        "the recorded installed Minimap camera hook is recognized");
    check(std::equal(replacement.begin(), replacement.end(), image.begin() + CameraEntryRva),
        "the ownership query returns the exact live replacement snapshot");

    auto wrong_original = CameraEntryOriginal;
    wrong_original[0] ^= 1;
    check(!ReadOwnedCameraEntryReplacement(wrong_original, replacement),
        "ownership is denied when the caller's expected preimage differs");

    image[CameraEntryRva + 3] ^= 1;
    check(!ReadOwnedCameraEntryReplacement(CameraEntryOriginal, replacement),
        "tampered live entry bytes invalidate Minimap ownership");
    image[CameraEntryRva + 3] ^= 1;

    live_relay[EntryRelayCallbackOffset] ^= 1;
    check(!ReadOwnedCameraEntryReplacement(CameraEntryOriginal, replacement),
        "a tampered owned relay is not accepted as the camera callback");
    live_relay[EntryRelayCallbackOffset] ^= 1;

    hooks.clear();
    image_base = previous_base;
    hooks = std::move(previous_hooks);
}
}
int main(int argc, char** argv) {
    using namespace MapRuntime;
    mixed_admission_statuses();
    camera_entry_hook_ownership();
    MinimapMenu10::Snapshot hud;check(!hud.AllowsGameplay(100));hud.Capture(1,100,true);
    check(hud.AllowsGameplay(600) && !hud.AllowsGameplay(601) && !hud.AllowsGameplay(99));
    hud.Capture(2,101,true);check(!hud.AllowsGameplay(101));hud.Capture(1,101,false);check(!hud.AllowsGameplay(101));
    std::array<std::uint64_t,32> discovery_words{};discovery_words[0]=UINT64_C(0x1122334455667788);discovery_words[2]=17;
    discovery_words[6]=std::uint64_t(300)*4294967296;discovery_words[7]=std::uint64_t(100)*4294967296;discovery_words[8]=std::uint64_t(400)*4294967296;
    std::array<std::uint8_t,96> visibility{};visibility[0]=2;std::uintptr_t discovery_record[10]{};
    discovery_record[1]=reinterpret_cast<std::uintptr_t>(discovery_words.data());discovery_record[2]=reinterpret_cast<std::uintptr_t>(visibility.data());
    SFMapMarkerV1 discovery{};bool positioned{};DiscoveryIdentity captured_identity{};
    check(ReadDiscoveryMarker(reinterpret_cast<std::uintptr_t>(discovery_record),discovery,positioned,&captured_identity) && positioned);
    check(discovery.id==discovery_words[0] && discovery.kind==17 && discovery.icon_candidate_count==10 && discovery.icon_candidates[1]==0x11223344);
    check(captured_identity.marker==discovery_record[1] && captured_identity.entity==0);
    check((discovery.flags&SF_MAP_MARKER_DISCOVERED)!=0);visibility[0]=0;
    check(ReadDiscoveryMarker(reinterpret_cast<std::uintptr_t>(discovery_record),discovery,positioned) && !(discovery.flags&SF_MAP_MARKER_VISIBLE));
    check(!ReadDiscoveryMarker(1,discovery,positioned));

    check(InspectMapImage({},"",false).status == SF_MAP_CAPABILITY_DENIED);
    check(InspectMapImage({},"",true).status == SF_MAP_UNSUPPORTED_BUILD);
    std::vector<std::uint8_t> damaged(1024);
    check(InspectMapImage(damaged,CurrentClientSha256,true).status == SF_MAP_UNSUPPORTED_BUILD);
    NativeTransform player{{12LL<<32,42LL<<32,14LL<<32},{0,0,0,1}};
    NativeTransform camera=player;
    NativeCameraRecord record{}; record.player_transform=reinterpret_cast<std::uintptr_t>(&player);
    record.camera_transform=reinterpret_cast<std::uintptr_t>(&camera);
    std::uint32_t local[2]{0,2}; record.local_player=reinterpret_cast<std::uintptr_t>(local);
    SFMapPoseV1 p{},c{};
    check(!ReadLocalCamera(record,1,p,c));
    check(ReadLocalCamera(record,2,p,c) && p.position[0]==12 && p.position[2]==14);
    record.camera_transform=1; check(!ReadLocalCamera(record,2,p,c));
    std::vector<std::uint8_t> pingBlock(0x800);
    struct PingList { std::uint8_t before[0x18]{}; std::uint32_t count{1}; std::uint8_t gap[0x14]{};
        std::uintptr_t arena{}; std::uint64_t capacity{1}; std::uint16_t index{1}; } list;
    static_assert(offsetof(PingList,arena)==0x30 && offsetof(PingList,index)==0x40);
    list.arena=reinterpret_cast<std::uintptr_t>(pingBlock.data());
    std::uint32_t sender=3; std::memcpy(pingBlock.data()+16,&sender,4);
    std::memcpy(pingBlock.data()+24,player.xyz,24);
    std::vector<SFMapMarkerV1> markers;
    check(ReadPingEvents(reinterpret_cast<std::uintptr_t>(&list),2,100,markers));
    check(markers.size()==1 && markers[0].id==3 && markers[0].expires_at_ms==8100);
    check(ReadPingEvents(reinterpret_cast<std::uintptr_t>(&list),3,100,markers) && markers.empty());
    list.count=1025; check(!ReadPingEvents(reinterpret_cast<std::uintptr_t>(&list),2,100,markers));
    list.count=60; check(!ReadPingEvents(reinterpret_cast<std::uintptr_t>(&list),2,100,markers));
    list.count=0; list.arena=0; check(ReadPingEvents(reinterpret_cast<std::uintptr_t>(&list),2,100,markers) && markers.empty());

    // The shared patch primitive must reject an occupied instruction on an
    // owned executable page. This never patches a game or loaded module.
    auto* page=static_cast<std::uint8_t*>(VirtualAlloc(nullptr,4096,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE));
    check(page!=nullptr); page[0]=0xeb; page[1]=0xfe; DWORD previous{};
    check(VirtualProtect(page,4096,PAGE_EXECUTE_READ,&previous)!=FALSE);
    HANDLE loop=CreateThread(nullptr,0,private_loop,page,0,nullptr); check(loop!=nullptr);
    Sleep(20);
    const std::uint8_t replacement[2]{0x90,0x90}, original[2]{0xeb,0xfe};
    check(!EngineHooks::WriteExecutableBytes(reinterpret_cast<std::uintptr_t>(page),replacement,2,original));
    check(page[0]==0xeb && page[1]==0xfe);
    // Release the private loop by redirecting only this owned suspended thread
    // to RET; no TerminateThread and no multi-byte live instruction replacement.
    check(SuspendThread(loop)!=DWORD(-1)); CONTEXT context{}; context.ContextFlags=CONTEXT_CONTROL;
    check(GetThreadContext(loop,&context)!=FALSE);
    check(VirtualProtect(page,4096,PAGE_READWRITE,&previous)!=FALSE); page[16]=0xc3;
    check(VirtualProtect(page,4096,PAGE_EXECUTE_READ,&previous)!=FALSE);
    FlushInstructionCache(GetCurrentProcess(),page,4096);
    context.Rip=reinterpret_cast<DWORD64>(page+16); check(SetThreadContext(loop,&context)!=FALSE);
    check(ResumeThread(loop)!=DWORD(-1)); check(WaitForSingleObject(loop,1000)==WAIT_OBJECT_0); CloseHandle(loop);
    check(EngineHooks::ReplaceExecutableBytes(reinterpret_cast<std::uintptr_t>(page),original,replacement,2));
    check(page[0]==0x90 && page[1]==0x90);
    check(!EngineHooks::ReplaceExecutableBytes(reinterpret_cast<std::uintptr_t>(page),original,replacement,2));
    VirtualFree(page,0,MEM_RELEASE);

    if (argc==2) {
        std::ifstream input(argv[1],std::ios::binary|std::ios::ate); check(bool(input));
        std::vector<std::uint8_t> file(std::size_t(input.tellg())); input.seekg(0);
        check(bool(input.read(reinterpret_cast<char*>(file.data()),file.size())));
        const auto* dos=reinterpret_cast<const IMAGE_DOS_HEADER*>(file.data());
        const auto* nt=reinterpret_cast<const IMAGE_NT_HEADERS64*>(file.data()+dos->e_lfanew);
        std::vector<std::uint8_t> image(nt->OptionalHeader.SizeOfImage);
        std::memcpy(image.data(),file.data(),nt->OptionalHeader.SizeOfHeaders);
        const auto* sections=IMAGE_FIRST_SECTION(nt);
        for(unsigned i=0;i<nt->FileHeader.NumberOfSections;++i) {
            const auto& section=sections[i];
            check(std::size_t(section.PointerToRawData)+section.SizeOfRawData<=file.size());
            check(std::size_t(section.VirtualAddress)+section.SizeOfRawData<=image.size());
            std::memcpy(image.data()+section.VirtualAddress,file.data()+section.PointerToRawData,section.SizeOfRawData);
        }
        const auto base=reinterpret_cast<std::uintptr_t>(image.data());
        for(auto rva:{0x1e04490u,0x1e0e400u,0x1e04660u,0x1e08620u,0x1e05560u,0x1e0a960u,
            0x1e063d0u,0x1e09410u,0x1e0f970u,0x1e13e00u,0x1dea720u})
            for(unsigned word:{0u,2u}) { std::uintptr_t value{}; std::memcpy(&value,image.data()+rva+word*8,8);
                value=value-nt->OptionalHeader.ImageBase+base; std::memcpy(image.data()+rva+word*8,&value,8); }
        for(unsigned i=0;i<4;++i) { auto* ptr=image.data()+0x1bbd820+i*0x28; std::uintptr_t value{};
            std::memcpy(&value,ptr,8); value=value-nt->OptionalHeader.ImageBase+base; std::memcpy(ptr,&value,8); }
        check(MinimapMenu10::MatchGuards(image.data(),image.size(),nullptr,true));
        check(!MinimapMenu10::MatchGuards(image.data(),image.size(),nullptr,false));
        std::uint8_t owned_call[5]{};std::memcpy(owned_call,image.data()+0xbc24ba,5);image[0xbc24ba]^=1;
        check(!MinimapMenu10::MatchGuards(image.data(),image.size(),nullptr,true));
        check(MinimapMenu10::MatchGuardsWithRendererPrefix(image.data(),image.size(),nullptr,0,owned_call,5,nullptr,true));
        image[0xbc24bf]^=1;check(!MinimapMenu10::MatchGuardsWithRendererPrefix(image.data(),image.size(),nullptr,0,owned_call,5,nullptr,true));
        image[0xbc24bf]^=1;image[0xbc24ba]^=1;
        auto admission=InspectMapImage(image,CurrentClientSha256,true);
        check(admission.status==SF_MAP_READY && admission.player && admission.markers && admission.fog &&
            admission.clock && admission.weather && admission.world && admission.deaths && admission.discovery && admission.renderer);
        check(InspectMapImage(image,"changed",true).status==SF_MAP_UNSUPPORTED_BUILD);
        image[0x2b2b60]^=1; check(!InspectMapImage(image,CurrentClientSha256,true).player); image[0x2b2b60]^=1;
        image[0x3072a4]^=1; admission=InspectMapImage(image,CurrentClientSha256,true);
        check(!admission.fog && admission.clock && admission.player); image[0x3072a4]^=1;
        image[0x2fdd18]^=1; admission=InspectMapImage(image,CurrentClientSha256,true);
        check(!admission.weather && admission.fog && admission.world);
        SnapshotStore partial; partial.SetAdmission(admission.Features()); partial.SetSession(1,true,false,true,100);
        check(partial.Acquire(1,101).frame.environment.weather_meta.status==SF_MAP_UNSUPPORTED_BUILD);
        check(partial.Acquire(1,101).frame.environment.clock_meta.status==SF_MAP_NOT_CAPTURED);
        image[0x2fdd18]^=1;
        image[0x2b3340]^=1; admission=InspectMapImage(image,CurrentClientSha256,true);
        check(!admission.clock && admission.weather);
        partial.SetAdmission(admission.Features()); partial.SetSession(1,true,false,true,100);
        check(partial.Acquire(1,101).frame.environment.clock_meta.status==SF_MAP_UNSUPPORTED_BUILD);
        check(partial.Acquire(1,101).frame.environment.weather_meta.status==SF_MAP_NOT_CAPTURED);
        image[0x2b3340]^=1;
        image[0x310cb0+0x30]^=1; admission=InspectMapImage(image,CurrentClientSha256,true);
        check(admission.world && !admission.deaths);
        partial.SetAdmission(admission.Features()); partial.SetSession(1,true,false,true,100);
        check(partial.Acquire(1,101).frame.deaths.meta.status==SF_MAP_UNSUPPORTED_BUILD);
        check(partial.Acquire(1,101).frame.world_markers.meta.status==SF_MAP_NOT_CAPTURED);
        image[0x310cb0+0x30]^=1;
        image[0x1bbd820+0x10]^=1; check(!InspectMapImage(image,CurrentClientSha256,true).weather);
    }
    std::printf("PASS: %u native map admission/capture/patch checks%s\n",checks,argc==2 ? " with data-only executable fixture" : " (executable fixture omitted)");
}
