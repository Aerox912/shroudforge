#pragma once
#include <cstdint>
#include <cstddef>
#include <string_view>
namespace MapRuntime::Current0403981f {
inline constexpr char Sha[]="0403981fd2e8ad18bd8c5698dff03c8d505b5a46677832c496db9c645513f2a6";
inline constexpr std::size_t ImageSize=0x2a40000;
inline constexpr std::uint32_t Timestamp=1793351710;
inline constexpr std::uintptr_t Camera=0x2b7820,CameraResume=0x2b7832,EntityArray=0x830;
inline constexpr std::uintptr_t IterInit=0x9c7ac0,IterNext=0x9c26c0,GetWorld=0x9bfac0,Lookup=0x9b2a90;
inline constexpr std::uintptr_t Remote=0x31d2b0,Waypoints=0x313f20,Clock=0x2b8000,World=0x304390;
inline constexpr std::uintptr_t WeatherCall=0x3030c8,WeatherSample=0xaaa410,FogCall=0x30c654,FogUpdate=0x3a26a0,Discovery=0x41c2d2;
inline constexpr std::size_t CustomList=0x396a28,WorldList=0x36ea00,SecondaryWorldList=0x3bea50,DeathList=0x3e6a78;
inline constexpr std::uintptr_t HudCall=0xbd154a,Renderer=0xc60310,UiGlobal=0x28404a8;
struct NamedSystem{std::size_t descriptor;const char* name;std::size_t callback;};
inline constexpr NamedSystem Systems[]={
{0x1E25E00,"player_camera_transform",0x2B7820},
{0x1E2FD80,"player_waypoints_ui",0x313F20},
{0x1E25FD0,"player_daytime_ui",0x2B8000},
{0x1E29FA0,"client_update_daytime",0x307B60},
{0x1E26EE0,"ambient_effects",0x302D50},
{0x1E2C2E0,"fog_of_war",0x30C590},
{0x1E27D50,"clear_map_markers_for_ui",0x304390},
{0x1E2AD90,"custom_map_markers_ui",0x308710},
{0x1E312F0,"replicate_map_markers_for_ui",0x3160D0},
{0x1E35780,"update_ui_data",0x31D2B0},
{0x1E0C060,"knowledge_query_mapmarker_visibility",0x41C070}
};
constexpr bool Selected(std::string_view sha) noexcept{return sha==Sha;}
// Static capability data only. No held preparation or product/runtime acceptance.
}
