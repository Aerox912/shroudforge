// SPDX-License-Identifier: MIT
#include "vein/ikea_client/coordinator.hpp"
#include <cstring>
#include <utility>
namespace vein::ikea_client {
namespace {
void store(std::span<std::byte> packet,std::size_t offset,std::uint32_t value) noexcept {std::memcpy(packet.data()+offset,&value,sizeof(value));}
bool same_settings(const Settings& a,const Settings& b) noexcept {
 return a.mode==b.mode&&a.activation_key==b.activation_key&&a.radius_key==b.radius_key&&a.radius_multiplier==b.radius_multiplier&&a.toggle_migration_revision==b.toggle_migration_revision;
}
}
PacketWriter::PacketWriter(std::uint32_t last_serial) noexcept:serial_(last_serial&nonce_mask){}
PacketStatus PacketWriter::write(std::span<std::byte> packet,bool active,unsigned radius) noexcept {
 // Trusted portable callers retain their bounded-span contract.
 const PacketTarget target{&packet,[](void* context,std::uint32_t magic,std::uint32_t encoded) noexcept {
  const auto p=*static_cast<std::span<std::byte>*>(context);
  if(p.size()<packet_bytes)return PacketStatus::short_buffer;
  if(p[presence_offset]!=std::byte{0})return PacketStatus::occupied;
  store(p,magic_offset,magic);store(p,encoded_offset,encoded);
  p[0x140]|=std::byte{1};p[presence_offset]=std::byte{1};
  return PacketStatus::written;
 }};
 return write(target,active,radius);
}
PacketStatus PacketWriter::write(PacketTarget target,bool active,unsigned radius) noexcept {
 if(!active)return PacketStatus::inactive;
 if(radius<1||radius>3)return PacketStatus::invalid_radius;
 if(!target.commit)return PacketStatus::unavailable;
 auto next=(serial_+1)&nonce_mask;if(!next)next=1;
 const auto status=target.commit(target.context,namespace_magic,tag|((radius-1)<<22)|next);
 if(status==PacketStatus::written)serial_=next;
 return status;
}
std::uint32_t PacketWriter::last_serial() const noexcept{return serial_;}
Coordinator::Coordinator(std::filesystem::path path,settings_file::FileOperations operations):file_(std::move(path),operations){}
settings_file::LoadResult Coordinator::initialize(){
 std::lock_guard file_guard(file_mutex_);
 {
  std::lock_guard state_guard(state_mutex_);
  if(loaded_)return {settings_file::Error::none,settings_,file_existed_};
 }
 auto result=file_.load();
 if(result){
  std::lock_guard state_guard(state_mutex_);
  settings_=*result.settings;controls_=Controls(settings_.mode,settings_.radius_multiplier);
  activation_=0;sampled_valid_=false;sampled_epoch_=0;sampled_session_generation_=0;native_session_generation_=0;
  have_last_sample_tick_=false;loaded_=true;dirty_=false;file_existed_=result.file_existed;
 }
 return result;
}
bool Coordinator::begin_activation(std::uint64_t epoch){
 std::lock_guard guard(state_mutex_);
 if(!loaded_||!epoch)return false;
 if(activation_!=epoch){controls_.reset_session();activation_=epoch;notices_={};notice_toggle_state_=false;native_session_generation_=0;}
 sampled_valid_=false;sampled_epoch_=0;sampled_session_generation_=0;
 // Serial history deliberately survives activation changes. The server owns its
 // actual retirement; there is no acknowledgement or Off/revoke packet here.
 return true;
}
void Coordinator::retire_activation(std::uint64_t epoch){
 std::lock_guard guard(state_mutex_);
 if(activation_==epoch){activation_=0;controls_.reset_session();sampled_valid_=false;sampled_epoch_=0;sampled_session_generation_=0;native_session_generation_=0;notices_={};notice_toggle_state_=false;}
}
void Coordinator::reset_native_session_locked() noexcept {
 controls_.reset_session();
 sampled_input_={};sampled_result_={};sampled_valid_=false;sampled_epoch_=0;sampled_session_generation_=0;sampled_tick_ms_=0;
 native_session_generation_=0;notices_={};notice_toggle_state_=false;
 // last_sample_tick_ms_ is a process-wide monotonic high-water mark. Do not
 // lower it when a native context/channel/session is retired.
}
void Coordinator::reset_native_session(std::uint64_t epoch){
 std::lock_guard guard(state_mutex_);
 if(loaded_&&activation_&&activation_==epoch)reset_native_session_locked();
}
PacketResult Coordinator::on_packet(std::uint64_t epoch,std::span<std::byte> packet,const InputObservation& input){
 std::lock_guard guard(state_mutex_);
 if(!loaded_||!activation_||activation_!=epoch)return {};
 const auto result=observe_locked(input);
 return {writer_.write(packet,result.active,result.radius_multiplier),result};
}
PacketResult Coordinator::on_packet(std::uint64_t epoch,PacketTarget target,const InputObservation& input){
 std::lock_guard guard(state_mutex_);
 if(!loaded_||!activation_||activation_!=epoch)return {};
 const auto result=observe_locked(input);
 return {writer_.write(target,result.active,result.radius_multiplier),result};
}
InputResult Coordinator::observe_locked(const InputObservation& input){
 const InputSample sample{input.key_down(settings_.activation_key),input.key_down(settings_.radius_key),input.focus==Focus::gameplay,input.capture!=Capture::clear};
 const auto result=controls_.sample(sample);
 if(result.radius_changed){settings_.radius_multiplier=result.radius_multiplier;dirty_=true;notices_.radius=result.radius_multiplier;}
 if(settings_.mode==Mode::Toggle&&sample.gameplay_focused&&!sample.input_captured&&result.active!=notice_toggle_state_){notice_toggle_state_=result.active;notices_.toggle=result.active;}
 return result;
}
InputResult Coordinator::observe_input(std::uint64_t epoch,const InputObservation& input){
 std::lock_guard guard(state_mutex_);
 if(!loaded_||!activation_||activation_!=epoch)return {};
 return observe_locked(input);
}
InputResult Coordinator::observe_worker_input(std::uint64_t epoch,const InputObservation& input,std::uint64_t completed_tick_ms){
 return observe_worker_input(epoch,0,input,completed_tick_ms);
}
InputResult Coordinator::observe_worker_input(std::uint64_t epoch,std::uint64_t native_session_generation,const InputObservation& input,std::uint64_t completed_tick_ms){
 std::lock_guard guard(state_mutex_);
 if(!loaded_||!activation_||activation_!=epoch)return {};
 if(native_session_generation_!=native_session_generation){
  reset_native_session_locked();
  native_session_generation_=native_session_generation;
 }
 // A backwards monotonic clock reading poisons this sample. Retain the prior
 // high-water mark so a later valid reading can recover without accepting a
 // sample from the interval in which time moved backwards. Treat its input as
 // blocked too, tracking physical levels while disarming possible key edges.
 if(have_last_sample_tick_&&completed_tick_ms<last_sample_tick_ms_){
  auto blocked=input;blocked.focus=Focus::unknown;blocked.capture=Capture::unknown;(void)observe_locked(blocked);
  sampled_valid_=false;sampled_epoch_=0;sampled_session_generation_=native_session_generation_;return {false,false,settings_.radius_multiplier};
 }
 const auto result=observe_locked(input);
 last_sample_tick_ms_=completed_tick_ms;have_last_sample_tick_=true;
 sampled_input_=input;sampled_result_=result;sampled_epoch_=epoch;sampled_session_generation_=native_session_generation;sampled_tick_ms_=completed_tick_ms;sampled_valid_=true;
 return result;
}
PacketResult Coordinator::on_sampled_packet(std::uint64_t epoch,PacketTarget target,const InputObservation& context,bool hold_activation_key_down,std::uint64_t now_tick_ms){
 return on_sampled_packet(epoch,0,target,context,hold_activation_key_down,now_tick_ms);
}
PacketResult Coordinator::on_sampled_packet(std::uint64_t epoch,std::uint64_t native_session_generation,PacketTarget target,const InputObservation& context,bool hold_activation_key_down,std::uint64_t now_tick_ms){
 std::lock_guard guard(state_mutex_);
 if(!loaded_||!activation_||activation_!=epoch)return {};
 if(native_session_generation_!=native_session_generation||sampled_session_generation_!=native_session_generation){
  reset_native_session_locked();
  const InputResult controls{false,false,settings_.radius_multiplier};
  return {writer_.write(target,false,controls.radius_multiplier),controls};
 }
 const bool fresh_context=context.focus==Focus::gameplay&&context.capture==Capture::clear;
 const bool fresh_sample=sampled_valid_&&sampled_epoch_==epoch&&now_tick_ms>=sampled_tick_ms_&&
  now_tick_ms-sampled_tick_ms_<=sampled_input_max_age_ms&&sampled_input_.focus==Focus::gameplay&&sampled_input_.capture==Capture::clear;
 if(!fresh_context||!fresh_sample){
  // Disarm worker-owned edges using the last observed physical key state.
  // A packet-time context failure or expired sample must not manufacture an
  // edge when focus returns while a key remains held.
  auto blocked=sampled_input_;blocked.focus=context.focus;blocked.capture=context.capture;
  if(fresh_context)blocked.capture=Capture::captured;
  (void)observe_locked(blocked);sampled_valid_=false;sampled_epoch_=0;sampled_session_generation_=native_session_generation_;
  const InputResult controls{false,false,settings_.radius_multiplier};
  return {writer_.write(target,false,controls.radius_multiplier),controls};
 }
 const bool active=settings_.mode==Mode::Hold?hold_activation_key_down:sampled_result_.active;
 const InputResult controls{active,false,settings_.radius_multiplier};
 return {writer_.write(target,active,controls.radius_multiplier),controls};
}
NoticeEvents Coordinator::take_notices(std::uint64_t epoch){
 std::lock_guard guard(state_mutex_);
 if(!loaded_||!activation_||activation_!=epoch)return {};
 return std::exchange(notices_,{});
}
settings_file::Error Coordinator::flush_settings(){
 std::lock_guard file_guard(file_mutex_);
 Settings snapshot;
 {
  std::lock_guard state_guard(state_mutex_);
  if(!loaded_)return settings_file::Error::not_loaded;
  if(!dirty_)return settings_file::Error::none;
  snapshot=settings_;
 }
 // Keep file I/O outside the mutex used by packet callbacks. A new radius-key edge
 // during this write remains dirty unless it cycles back to the saved values.
 const auto error=file_.save(snapshot);
 if(error==settings_file::Error::none){
  std::lock_guard state_guard(state_mutex_);
  if(same_settings(settings_,snapshot))dirty_=false;
 }
 return error;
}
std::optional<Settings> Coordinator::settings() const {std::lock_guard guard(state_mutex_);return loaded_?std::optional(settings_):std::nullopt;}
bool Coordinator::settings_pending() const {std::lock_guard guard(state_mutex_);return dirty_;}
std::uint32_t Coordinator::last_serial() const {std::lock_guard guard(state_mutex_);return writer_.last_serial();}
}
