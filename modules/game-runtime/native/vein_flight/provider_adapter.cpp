// SPDX-License-Identifier: MIT
// Explicit opt-in OS cursor policy; this is not a native menu/chat predicate.
#include "provider_adapter.hpp"
#include "a6_consumer.hpp"
#include "vein/ikea_client/flight_transport.hpp"
#include "vein/ikea_client/native_reset.hpp"
#include "vein/ikea_client/native_reset_profile.hpp"
#include "vein/ikea_client/native_input_profile.hpp"
#include "vein/ikea_client/native_input_session.hpp"
#include "vein/ikea_client/hook_transaction.hpp"
#include "vein/ikea_client/profile.hpp"
#include "vein/ikea_client/original_client_guard.hpp"

#include <vector> // Required before the pinned upstream host header.
#include <windows.h>
#include <MinHook.h>
#include <atomic>
#include <filesystem>
#include <iterator>
#include <memory>
#include <mutex>
#include <string_view>
#include <intrin.h>
namespace {
using namespace vein::ikea_client;

using InputProducer=std::uint64_t(__fastcall*)(void*,void*,std::uint32_t,void*,std::uint32_t,std::uint8_t);
using Packer=std::uint64_t(__fastcall*)(void*,void*,const void*,std::uint32_t);
using HostAttempt=std::uint64_t(__fastcall*)(void*,std::uint8_t);
using PeerAdd=std::uint64_t(__fastcall*)(void*,const void*,std::uint8_t);
using PeerLookup=void*(__fastcall*)(void*,std::uint16_t);
using PeerRemove=std::uint64_t(__fastcall*)(void*,std::uint16_t);
using RosterReset=std::uint64_t(__fastcall*)(void*);
struct Runtime {
 std::mutex gate;
 std::unique_ptr<VeinFlight::A6Consumer> coordinator;
 Packer original=nullptr;
 FlightMux flight_mux;
 NativeReset native_reset{flight_mux,1,FlightAuthority::current_input};
 VeinInputSession vein_input_session;
 InputProducer input_original=nullptr;
 HostAttempt host_original=nullptr;
 PeerAdd add_original=nullptr;
 PeerLookup lookup_original=nullptr;
 PeerRemove remove_original=nullptr,retire_original=nullptr;
 RosterReset roster_original=nullptr;
 HookTransaction hooks;
 std::uintptr_t game_base=0;
 std::uint32_t game_size=0;
 bool flight_transport_enabled=false,vein_policy_allowed=false;
 DWORD tls=TLS_OUT_OF_INDEXES;
 HANDLE wake_event=nullptr;
 HANDLE provider_claim=nullptr;
 bool attempted=false,ready=false,enabled=false;
 std::uint64_t epoch=0,save_generation=0;
 std::atomic<bool> fault{false};
 std::atomic<bool> vein_fault{false};
};
// Resident ownership is intentional: the evidenced host never calls Unload and
// does not provide callback quiescence. No destructor, trampoline removal or hot reload.
Runtime& state(){static auto* runtime=new Runtime;return *runtime;}
bool signal_worker() noexcept {
 auto& runtime=state();const DWORD last_error=GetLastError();
 if(!runtime.wake_event&&!runtime.vein_policy_allowed)return true;
 const bool signalled=runtime.wake_event&&SetEvent(runtime.wake_event)!=FALSE;
 if(!signalled)runtime.vein_fault.store(true);
 SetLastError(last_error);return signalled;
}
void fault_worker() noexcept {
 auto& runtime=state();
 if(!runtime.fault.exchange(true))(void)signal_worker();
}
void fault_vein_worker() noexcept {state().vein_fault.store(true);}
void reset_vein_consumer(Runtime& r,std::uint64_t epoch) noexcept {
 try{if(r.coordinator&&epoch)r.coordinator->reset_native_session(epoch);}
 catch(...){fault_vein_worker();}
}
std::optional<vein::Settings> vein_settings(Runtime& r) noexcept {
 try{if(r.coordinator&&!r.vein_fault.load())return r.coordinator->settings();}
 catch(...){fault_vein_worker();}return {};
}
void commit_vein_packet(Runtime& r,std::uint64_t epoch,std::uint64_t generation,PacketTarget target,const InputObservation& context,bool hold,std::uint64_t now) noexcept {
 try{if(r.coordinator&&!r.vein_fault.load())(void)r.coordinator->on_sampled_packet(epoch,generation,target,context,hold,now);}
 catch(...){fault_vein_worker();}
}
bool create_worker_event() noexcept {
 auto& runtime=state();if(runtime.wake_event)return true;
 const DWORD last_error=GetLastError();runtime.wake_event=CreateEventW(nullptr,FALSE,FALSE,nullptr);
 const bool created=runtime.wake_event!=nullptr;SetLastError(last_error);return created;
}
bool claim_provider_owner(HANDLE& claim) noexcept {
 // A resident process-local claim also covers simultaneous loads of different
 // copies of this module. Retained until process exit, including failed setup.
 // An existing/precreated object refuses admission, never authenticates a peer.
 const DWORD error=GetLastError();wchar_t name[96]{};
 if(claim||swprintf_s(name,L"Local\\ShroudweaveVeinFlightResetProviderV1-%lu",GetCurrentProcessId())<0){SetLastError(error);return false;}
 HANDLE candidate=CreateMutexW(nullptr,FALSE,name);const DWORD created_error=GetLastError();
 if(!candidate){SetLastError(error);return false;}
 if(created_error==ERROR_ALREADY_EXISTS){
  if(!CloseHandle(candidate)){claim=candidate;fault_worker();}
  SetLastError(error);return false;
 }
 claim=candidate;SetLastError(error);return true;
}
void diagnostic(const VeinFlight::ProviderConfig* config,const char* message) noexcept {
 try{if(config&&config->log)config->log(message);}catch(...){}
}
bool readable(const void* address,std::size_t bytes,HMODULE owner=nullptr) noexcept {
 auto cursor=reinterpret_cast<std::uintptr_t>(address);
 if(!cursor||bytes>UINTPTR_MAX-cursor)return false;
 const auto end=cursor+bytes;
 while(cursor<end){MEMORY_BASIC_INFORMATION info{};
  if(VirtualQuery(reinterpret_cast<const void*>(cursor),&info,sizeof(info))!=sizeof(info)||info.State!=MEM_COMMIT||(owner&&info.AllocationBase!=owner)||(info.Protect&(PAGE_GUARD|PAGE_NOACCESS)))return false;
  const auto protection=info.Protect&0xff;
  if(protection!=PAGE_READONLY&&protection!=PAGE_READWRITE&&protection!=PAGE_WRITECOPY&&protection!=PAGE_EXECUTE_READ&&protection!=PAGE_EXECUTE_READWRITE&&protection!=PAGE_EXECUTE_WRITECOPY)return false;
  const auto next=reinterpret_cast<std::uintptr_t>(info.BaseAddress)+info.RegionSize;
  if(next<=cursor)return false;cursor=next;
 }
 return true;
}
bool game_profile(HMODULE game) noexcept {
 if(!game||!readable(game,0x1000,game))return false;
 wchar_t path[32768]{};const DWORD length=GetModuleFileNameW(game,path,32768);
 if(!length||length>=32768||_wcsicmp(std::filesystem::path(path).filename().c_str(),L"enshrouded.exe"))return false;
 const auto* base=reinterpret_cast<const std::byte*>(game);
 const auto* dos=reinterpret_cast<const IMAGE_DOS_HEADER*>(base);
 if(dos->e_magic!=IMAGE_DOS_SIGNATURE||dos->e_lfanew<0x40||dos->e_lfanew>0x800)return false;
 const auto* nt=reinterpret_cast<const IMAGE_NT_HEADERS64*>(base+dos->e_lfanew);
 const auto size=nt->OptionalHeader.SizeOfImage;
 if(size<0x1000||size>0x40000000)return false;
 for(const auto& guard:current_guards)if(guard.rva>size||guard.size>size-guard.rva||!readable(base+guard.rva,guard.size,game))return false;
 if(!validate_mapped_profile({base,size}))return false;
 for(const auto& guard:input_layout_guards)if(guard.rva>size||guard.size>size-guard.rva||!readable(base+guard.rva,guard.size,game))return false;
 for(const auto& guard:input_layout_pointer_guards)if(guard.rva>size||sizeof(std::uint64_t)>size-guard.rva||!readable(base+guard.rva,sizeof(std::uint64_t),game))return false;
 if(!validate_mapped_layout({base,size},reinterpret_cast<std::uintptr_t>(game)))return false;
 if(needs_native_input_lifecycle_hooks(state().vein_policy_allowed,state().flight_transport_enabled)){
  for(const auto& guard:native_reset_guards)if(guard.rva>size||guard.size>size-guard.rva||!readable(base+guard.rva,guard.size,game))return false;
  if(!validate_mapped_profile({base,size},native_reset_guards))return false;
  for(const auto& guard:native_input_guards)if(guard.rva>size||guard.size>size-guard.rva||!readable(base+guard.rva,guard.size,game))return false;
  if(!validate_mapped_profile({base,size},native_input_guards))return false;
 }
 state().game_size=size;
 return true;
}
bool original_client_conflict() {
 if(GetModuleHandleW(L"XHL-Vein-Mining.dll"))return true;
 wchar_t path[32768]{};const auto length=GetModuleFileNameW(nullptr,path,32768);
 if(!length||length>=32768)return true;
 return !canonical_original_client_absent(std::filesystem::path(path).parent_path());
}
InputObservation observe_context() noexcept {
 InputObservation input;
 const HWND foreground=GetForegroundWindow();DWORD process=0;
 if(!foreground||!GetWindowThreadProcessId(foreground,&process))return input;
 if(process!=GetCurrentProcessId()){input.focus=Focus::other;return input;}
 CURSORINFO cursor{sizeof(CURSORINFO)};
 if(!GetCursorInfo(&cursor))return input;
 input.focus=Focus::gameplay;
 // Explicit test policy only. This is an OS observation, not a proven native
 // chat/menu capture predicate. Suppressed/visible/unknown cursor blocks.
 input.capture=cursor.flags==0?Capture::clear:Capture::captured;
 return input;
}
bool key_down(std::uint16_t key) noexcept {return (GetAsyncKeyState(key)&0x8000)!=0;}
InputObservation observe(const vein::Settings& settings) noexcept {
 auto input=observe_context();
 if(input.focus!=Focus::gameplay||input.capture==Capture::unknown)return input;
 const auto read_key=[&](std::uint16_t key){if(GetAsyncKeyState(key)&0x8000)input.keys[key/64]|=std::uint64_t{1}<<(key%64);};
 read_key(settings.activation_key);
 if(settings.radius_key!=settings.activation_key)read_key(settings.radius_key);
 return input;
}
// VirtualQuery is a preflight, not a lifetime pin. Stock's caller must retain
// exclusive ownership and lifetime of its output for this synchronous callback.
// Check the full end before computing any offset, including near UINTPTR_MAX.
bool writable_range(std::uintptr_t cursor,std::size_t bytes) noexcept {
 if(!cursor||bytes>UINTPTR_MAX-cursor)return false;
 const auto end=cursor+bytes;
 while(cursor<end){
  MEMORY_BASIC_INFORMATION info{};
  if(VirtualQuery(reinterpret_cast<const void*>(cursor),&info,sizeof(info))!=sizeof(info)||info.State!=MEM_COMMIT||(info.Protect&(PAGE_GUARD|PAGE_NOACCESS)))return false;
  const auto protection=info.Protect&0xff;
  if(protection!=PAGE_READWRITE&&protection!=PAGE_WRITECOPY&&protection!=PAGE_EXECUTE_READWRITE&&protection!=PAGE_EXECUTE_WRITECOPY)return false;
  const auto base=reinterpret_cast<std::uintptr_t>(info.BaseAddress);
  if(info.RegionSize>UINTPTR_MAX-base)return false;
  const auto next=base+info.RegionSize;if(next<=cursor)return false;cursor=next;
 }
 return true;
}
struct NativePacket {void* output;bool access_fault=false;};
int memory_fault_filter(DWORD code) noexcept {
 return code==EXCEPTION_ACCESS_VIOLATION||code==EXCEPTION_IN_PAGE_ERROR?EXCEPTION_EXECUTE_HANDLER:EXCEPTION_CONTINUE_SEARCH;
}
// Leaf SEH boundary: raw access only, no C++ RAII or lock acquisition, and never
// wraps stock. A race after preflight may leave payload/aggregate writes behind;
// presence is published last. A caught fault closes the resident sender.
PacketStatus native_commit(void* context,std::uint32_t magic,std::uint32_t encoded) noexcept {
 auto* packet=static_cast<NativePacket*>(context);
 const auto base=reinterpret_cast<std::uintptr_t>(packet->output);
 if(!base||packet_bytes>UINTPTR_MAX-base)return PacketStatus::unavailable;
 if(!writable_range(base+0x140,1)||!writable_range(base+presence_offset,1)||!writable_range(base+magic_offset,8))return PacketStatus::unavailable;
 __try{
  auto* presence=reinterpret_cast<volatile unsigned char*>(base+presence_offset);
  if(*presence!=0)return PacketStatus::occupied;
  // Byte stores avoid unaligned typed pointers. Volatile sequencing keeps the
  // publication marker after every payload/aggregate store, even under /O2.
  auto* payload=reinterpret_cast<volatile unsigned char*>(base+magic_offset);
  for(unsigned i=0;i<4;++i)payload[i]=static_cast<unsigned char>(magic>>(8*i));
  for(unsigned i=0;i<4;++i)payload[i+4]=static_cast<unsigned char>(encoded>>(8*i));
  auto* aggregate=reinterpret_cast<volatile unsigned char*>(base+0x140);
  *aggregate=static_cast<unsigned char>(*aggregate|1);
  *presence=1;
  return PacketStatus::written;
 }__except(memory_fault_filter(GetExceptionCode())){
  packet->access_fault=true;return PacketStatus::unavailable;
 }
}
// Foreign publications are copied in a leaf SEH scope before any lock is taken.
// VirtualQuery is a preflight only; no pointer from another DLL is retained.
bool copy_flight_publication(const EsmVeinFlightIntentV1* source,EsmVeinFlightIntentV1* value) noexcept {
 if(!source||!value||!readable(source,sizeof(*value)))return false;
 __try{*value=*source;return true;}
 __except(memory_fault_filter(GetExceptionCode())){return false;}
}
std::uint32_t publish_flight_intent(const EsmVeinFlightIntentV1* source) noexcept {
 const DWORD entry_error=GetLastError();std::uint32_t accepted=0;EsmVeinFlightIntentV1 value{};
 if(copy_flight_publication(source,&value)){
  try{
   auto& runtime=state();std::lock_guard lock(runtime.gate);
   if(runtime.ready&&runtime.flight_transport_enabled&&!runtime.fault.load()&&!GetModuleHandleW(L"XHL-Vein-Mining.dll"))
    accepted=runtime.native_reset.publish_v1(value,GetTickCount64())?1u:0u;
  }catch(...){fault_worker();}
 }
 SetLastError(entry_error);return accepted;
}
#include "shared/src/native_lifecycle.inc"
#include "shared/src/native_input.inc"
std::uint64_t __fastcall pack(void* output,void* input,const void* mask,std::uint32_t size_count) noexcept {
 const auto caller=reinterpret_cast<std::uintptr_t>(_ReturnAddress());
 const DWORD entry_error=GetLastError();auto& runtime=state();
 auto* input_scope=current_input_scope;std::uint64_t input_serial=0;
 // Reentry is owned before the first gate and stock; nested entries only forward.
 const auto nested=TlsGetValue(runtime.tls);const auto tls_error=GetLastError();
 if(nested||tls_error!=ERROR_SUCCESS){
  if(tls_error!=ERROR_SUCCESS)fault_worker();
  SetLastError(entry_error);return runtime.original(output,input,mask,size_count);
 }
 if(!TlsSetValue(runtime.tls,reinterpret_cast<void*>(1))){
  fault_worker();SetLastError(entry_error);return runtime.original(output,input,mask,size_count);
 }
 struct ReentryOwner {
  Runtime& runtime;
  ~ReentryOwner(){const DWORD error=GetLastError();if(!TlsSetValue(runtime.tls,nullptr))fault_worker();SetLastError(error);}
 } owner{runtime};
 std::uint64_t entry_epoch=0;PacketFrame flight_frame{};bool entry_flight=false,entry_vein_input=false;VeinInputTicket vein_ticket{};
 {
  std::lock_guard lock(runtime.gate);
  if(runtime.enabled&&!runtime.fault.load()&&!runtime.vein_fault.load())entry_epoch=runtime.epoch;
  const bool qualified_input=qualified_input_packet(input_scope,caller,runtime.game_base);
  VeinInputUpdate vein_update=VeinInputUpdate::invalid;
  if(entry_epoch&&qualified_input)
   vein_update=runtime.vein_input_session.observe_input(input_scope->context,input_scope->channel,GetTickCount64(),vein_ticket);
  entry_vein_input=vein_update!=VeinInputUpdate::invalid;
  if(vein_update==VeinInputUpdate::changed_session)reset_vein_consumer(runtime,entry_epoch);
  entry_flight=runtime.ready&&runtime.flight_transport_enabled&&!runtime.fault.load()&&qualified_input;
  if(entry_flight){
   input_serial=runtime.native_reset.observe_input(input_scope->context,input_scope->channel,GetTickCount64());
   entry_flight=input_serial!=0;
   if(entry_flight)flight_frame=runtime.flight_mux.begin_frame();
  }
 }
 // Old calls cannot inherit a later activation or newly published Flight On.
 SetLastError(entry_error);
 const auto result=runtime.original(output,input,mask,size_count);
 struct RestoreLastError{DWORD value=GetLastError();~RestoreLastError(){SetLastError(value);}} restore_error;
 try{
  std::lock_guard lock(runtime.gate);
  const bool vein_active=entry_vein_input&&entry_epoch&&runtime.enabled&&runtime.epoch==entry_epoch&&!runtime.fault.load()&&!runtime.vein_fault.load()&&
   current_input_scope==input_scope&&qualified_input_packet(input_scope,caller,runtime.game_base)&&runtime.vein_input_session.is_current(vein_ticket);
  const bool flight_active=entry_flight&&runtime.ready&&runtime.flight_transport_enabled&&!runtime.fault.load()&&
   current_input_scope==input_scope&&qualified_input_packet(input_scope,caller,runtime.game_base)&&
   runtime.native_reset.input_current(input_scope->context,input_scope->channel,input_serial,GetTickCount64());
  if((vein_active||flight_active)&&!GetModuleHandleW(L"XHL-Vein-Mining.dll")){
   const auto settings=vein_active?vein_settings(runtime):std::optional<vein::Settings>{};
   const auto flight_tick=flight_active?GetTickCount64():0;
   const bool need_context=bool(settings)||(flight_active&&runtime.flight_mux.needs_on_context(flight_frame,flight_tick));
   InputObservation context;if(need_context)context=observe_context();
   const bool clear=context.focus==Focus::gameplay&&context.capture==Capture::clear;
   const bool hold_activation=settings&&clear&&settings->mode==vein::Mode::Hold&&key_down(settings->activation_key);
   // Retain packet-time freshness after any OS observations, as the Vein path did.
   const auto now=need_context?GetTickCount64():flight_tick;
   NativePacket packet{output};const PacketTarget target{&packet,native_commit};
   if(flight_active){
    MuxPacket shared(runtime.flight_mux,target,flight_frame,now,clear);
    if(settings)commit_vein_packet(runtime,entry_epoch,vein_ticket.generation,shared.vein_target(),context,hold_activation,now);
    (void)shared.finish();
   }else if(settings){
    commit_vein_packet(runtime,entry_epoch,vein_ticket.generation,target,context,hold_activation,now);
   }
   if(packet.access_fault)fault_worker();
  }
 }catch(...){fault_worker();}
 return result;
}

void management_cycle(std::uint64_t& flushed,bool& reported_fault) noexcept {
 auto& runtime=state();VeinFlight::A6Consumer* coordinator=nullptr;std::uint64_t generation=0;
 try{
  {
   std::lock_guard lock(runtime.gate);
   if(runtime.enabled&&!runtime.fault.load()&&!runtime.vein_fault.load()){
    const auto settings=runtime.coordinator->settings();
    VeinInputTicket ticket{};
    if(settings&&runtime.vein_input_session.current_for_worker(GetTickCount64(),ticket)){
     const auto input=observe(*settings);const auto completed_tick=GetTickCount64();
     if(runtime.coordinator->observe_worker_input(runtime.epoch,ticket.generation,input,completed_tick).radius_changed)++runtime.save_generation;
    }
   }
   generation=runtime.save_generation;coordinator=runtime.coordinator.get();
  }
  // Flush only changed settings generations; the active worker polls controls.
  if(coordinator&&generation!=flushed){flushed=generation;const auto error=coordinator->flush_settings();
   if(error!=vein::settings_file::Error::none)OutputDebugStringA("Shroudweave Vein Mining: changed radius could not be saved; no idle retry; existing file preserved.\n");}
  if(runtime.fault.load()&&!reported_fault){reported_fault=true;OutputDebugStringA("Shroudweave Vein Mining: sender closed after internal fault; restart required.\n");}
 }catch(...){fault_vein_worker();}
}
DWORD management_wait_timeout() noexcept {
 auto& runtime=state();std::lock_guard lock(runtime.gate);
 return runtime.enabled&&!runtime.fault.load()&&!runtime.vein_fault.load()?8:INFINITE;
}
DWORD management_wait_once() noexcept {
 auto& runtime=state();const DWORD result=WaitForSingleObject(runtime.wake_event,management_wait_timeout());
 if(result==WAIT_OBJECT_0||result==WAIT_TIMEOUT)return result;
 fault_vein_worker();return WAIT_FAILED;
}
bool management_iteration(std::uint64_t& flushed,bool& reported_fault) noexcept {
 management_cycle(flushed,reported_fault);
 if(management_wait_once()!=WAIT_FAILED)return true;
 management_cycle(flushed,reported_fault);return false;
}
DWORD WINAPI management(void*) noexcept {
 std::uint64_t flushed=0;bool reported_fault=false;
 while(management_iteration(flushed,reported_fault)){}
 return 0;
}
bool prepare(const VeinFlight::ProviderConfig* context){
 auto& runtime=state();
 const auto fail=[&](const char* message){diagnostic(context,message);return false;};
 if(runtime.attempted)return runtime.ready;
 runtime.attempted=true;
 if(!context||!context->is_client||context->is_server||!context->dedicated_guest_qualified){
  diagnostic(context,"Shroudweave Vein Mining: explicit allowUnverifiedCursorGate opt-in required.");return false;
 }
 runtime.vein_policy_allowed=context->allow_unverified_cursor_gate;
 runtime.flight_transport_enabled=context->flight_transport_enabled;
 if(!runtime.vein_policy_allowed&&!runtime.flight_transport_enabled){
  diagnostic(context,"Shroudweave Vein Mining: explicit feature transport opt-in required.");return false;
 }
 if(original_client_conflict()){
  diagnostic(context,"Shroudweave Vein Mining: original Vein client DLL/discovery conflicts; INI disabled alone is insufficient.");return false;
 }
 const auto game=GetModuleHandleW(nullptr);
 if(!game_profile(game)){diagnostic(context,"Shroudweave Vein Mining: current native code/layout guard failed; sender closed.");return false;}
 HMODULE owner=nullptr;
 if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,reinterpret_cast<LPCWSTR>(&pack),&owner))return fail("Shroudweave Vein Mining: callback owner pin failed; sender closed.");
 if(!claim_provider_owner(runtime.provider_claim))return fail("Shroudweave Vein Mining: resident provider ownership unavailable; sender refused.");
 if(runtime.vein_policy_allowed){
  bool initialized=false;
  try{
   runtime.coordinator=std::make_unique<VeinFlight::A6Consumer>();
   initialized=context->vein_settings_path.is_absolute()&&bool(runtime.coordinator->initialize(context->vein_settings_path));
  }catch(...){runtime.vein_fault.store(true);}
  if(!initialized){
   runtime.coordinator.reset();runtime.vein_policy_allowed=false;
   diagnostic(context,"Shroudweave Vein Mining: canonical settings rejected; Vein disabled, file preserved.");
   if(!runtime.flight_transport_enabled)return false;
  }
 }
 // TLS must be valid before creating/enabling any callback. It remains resident
 // with the pinned owner, including initialization failures after this point.
 runtime.tls=TlsAlloc();if(runtime.tls==TLS_OUT_OF_INDEXES||!TlsSetValue(runtime.tls,nullptr))return fail("Shroudweave Vein Mining: pre-start TLS validation failed; sender closed.");
 const auto initialized=MH_Initialize();if(initialized!=MH_OK&&initialized!=MH_ERROR_ALREADY_INITIALIZED)return fail("Shroudweave Vein Mining: hook engine initialization failed; sender closed.");
 runtime.game_base=reinterpret_cast<std::uintptr_t>(game);
 const auto target=[&](std::uint32_t rva){return reinterpret_cast<void*>(runtime.game_base+rva);};
 if(!runtime.hooks.add(target(packer_rva),reinterpret_cast<void*>(&pack)))return fail("Shroudweave Vein Mining: hook plan rejected.");
 if(needs_native_input_lifecycle_hooks(runtime.vein_policy_allowed,runtime.flight_transport_enabled)){
  if(!runtime.hooks.add(target(host_attempt_rva),reinterpret_cast<void*>(&native_host))||
     !runtime.hooks.add(target(peer_add_rva),reinterpret_cast<void*>(&native_add))||
     !runtime.hooks.add(target(peer_lookup_rva),reinterpret_cast<void*>(&native_lookup))||
     !runtime.hooks.add(target(peer_remove_rva),reinterpret_cast<void*>(&native_remove))||
     !runtime.hooks.add(target(peer_retire_rva),reinterpret_cast<void*>(&native_retire))||
     !runtime.hooks.add(target(roster_reset_rva),reinterpret_cast<void*>(&native_roster))||
     !runtime.hooks.add(target(input_producer_rva),reinterpret_cast<void*>(&native_input)))return fail("Shroudweave Vein Mining: native lifecycle plan rejected.");
 }
 const auto operations=hook_operations();
 if(!runtime.hooks.create(operations))return fail("Shroudweave Vein Mining: composed hook preparation failed; resident disabled state retained.");
 runtime.original=reinterpret_cast<Packer>(runtime.hooks.entry(0).original);
 if(needs_native_input_lifecycle_hooks(runtime.vein_policy_allowed,runtime.flight_transport_enabled)){
  for(std::size_t i=1;i<8;++i)if(!leaf_trampoline(runtime.hooks.entry(i))){close_native_admission(nullptr);return fail("Shroudweave Vein Mining: native leaf trampoline shape differs; shared native input/lifecycle hook refused.");}
  runtime.host_original=reinterpret_cast<HostAttempt>(runtime.hooks.entry(1).original);
  runtime.add_original=reinterpret_cast<PeerAdd>(runtime.hooks.entry(2).original);
  runtime.lookup_original=reinterpret_cast<PeerLookup>(runtime.hooks.entry(3).original);
  runtime.remove_original=reinterpret_cast<PeerRemove>(runtime.hooks.entry(4).original);
  runtime.retire_original=reinterpret_cast<PeerRemove>(runtime.hooks.entry(5).original);
  runtime.roster_original=reinterpret_cast<RosterReset>(runtime.hooks.entry(6).original);
  runtime.input_original=reinterpret_cast<InputProducer>(runtime.hooks.entry(7).original);
 }
 // A6 controls alone need this worker. Flight has its own resident publisher
 // and must not depend on A6 event/thread creation or management health.
 if(runtime.vein_policy_allowed){
  bool worker_ready=create_worker_event();
  HANDLE thread=worker_ready?CreateThread(nullptr,0,management,nullptr,0,nullptr):nullptr;
  worker_ready=thread&&CloseHandle(thread);
  if(!worker_ready){
   runtime.vein_fault.store(true);
   diagnostic(context,"Shroudweave Vein Mining: management worker unavailable; Vein disabled.");
   if(!runtime.flight_transport_enabled)return false;
  }
 }
 // CreateHook prepared code only. Recheck the still-unmodified native profile
 // immediately before the one composed enable transaction.
 if(!game_profile(game)){close_native_admission(nullptr);return fail("Shroudweave Vein Mining: native profile changed during preparation; sender refused.");}
 if(!runtime.hooks.activate(operations)){
  runtime.fault.store(true);
  return fail(runtime.hooks.uncertain()?"Shroudweave Vein Mining: hook cancellation uncertain; admission closed, callbacks pinned, restart required.":"Shroudweave Vein Mining: hook enable failed; cancelled with resident originals retained.");
 }
 if(runtime.flight_transport_enabled&&!runtime.native_reset.open()){
  const bool cancelled=runtime.hooks.cancel(operations);runtime.fault.store(true);
  return fail(cancelled?"Shroudweave Vein Mining: native reset admission refused; cancelled.":"Shroudweave Vein Mining: native reset refused and cancellation uncertain; callbacks pinned.");
 }
 runtime.ready=true;
 diagnostic(context,"Shroudweave Vein Mining: current client hook prepared; compatible XHL server required; OS cursor policy is not native menu/chat detection.");return true;
}
} // private native hook state

namespace VeinFlight {
bool prepare_provider(const ProviderConfig& config) noexcept {
 try {auto& r=state();std::lock_guard lock(r.gate);return prepare(&config);}catch(...){fault_worker();return false;}
}
bool set_vein_enabled(bool enabled) noexcept {
 try {auto& r=state();std::lock_guard lock(r.gate);
  if(!r.ready||!r.vein_policy_allowed||!r.coordinator||r.fault.load())return false;
  if(!enabled){r.enabled=false;r.vein_input_session.invalidate();r.coordinator->retire_activation(r.epoch);(void)signal_worker();return true;}
  if(r.vein_fault.load())return false;
  if(r.enabled)return true;
  if(r.epoch==UINT64_MAX){fault_vein_worker();return false;}
  ++r.epoch;r.vein_input_session.invalidate();r.enabled=r.coordinator->begin_activation(r.epoch);
  if(r.enabled){r.coordinator->reset_native_session(r.epoch);(void)signal_worker();}
  return r.enabled;
 }catch(...){fault_vein_worker();return false;}
}
void tick_provider() noexcept { /* Native worker owns fresh input and persistence. */ }
void stop_provider() noexcept {
 try {auto& r=state();std::lock_guard lock(r.gate);
  r.enabled=false;r.vein_input_session.invalidate();
  if(r.coordinator)r.coordinator->retire_activation(r.epoch);
  close_native_admission(nullptr);r.fault.store(true);(void)signal_worker();
  // Resident originals/trampolines/module remain pinned. No hot unload.
 }catch(...){fault_worker();}
}
bool provider_ready() noexcept {
 try {auto& r=state();std::lock_guard lock(r.gate);return r.ready&&!r.fault.load();}catch(...){return false;}
}
vein::ikea_client::NoticeEvents take_vein_notices() noexcept {
 try {auto& r=state();std::lock_guard lock(r.gate);
  if(r.enabled&&r.coordinator&&!r.fault.load()&&!r.vein_fault.load())return r.coordinator->take_notices(r.epoch);
 }catch(...){fault_vein_worker();}return {};
}
} // namespace VeinFlight
extern "C" bool ESM_VEIN_FLIGHT_CALL ESM_VeinGetFlightResetSerialV1(std::uint64_t* value) {return get_reset_serial(value);}
extern "C" bool ESM_VEIN_FLIGHT_CALL ESM_VeinPublishFlightIntentV2(const EsmVeinFlightIntentV2* value) {return publish_flight_v2(value);}
