// SPDX-License-Identifier: MIT
#include "vein/ikea_client/flight_transport.hpp"
#include <limits>
namespace vein::ikea_client {
namespace {
constexpr std::uint32_t flight_tag=0xa7000000,on_mask=0x00800000;
constexpr std::uint64_t on_interval_ms=100,publication_lease_ms=500;
constexpr std::uint32_t off_publications=3;
}
void FlightMux::retire_on() noexcept {
    if(!on_)return;
    on_=false;retired_=true;off_remaining_=off_publications;off_priority_=true;
    have_on_write_=false;
    if(revision_!=(std::numeric_limits<std::uint64_t>::max)())++revision_;
}
void FlightMux::native_reset() noexcept {
    on_=false;retired_=true;off_remaining_=off_publications;off_priority_=true;
    have_on_write_=false;
    if(revision_!=(std::numeric_limits<std::uint64_t>::max)())++revision_;
}
bool FlightMux::advance_time(std::uint64_t now) noexcept {
    if(clock_seen_&&now<clock_high_){retire_on();return false;}
    clock_seen_=true;clock_high_=now;return true;
}
void FlightMux::expire(std::uint64_t now) noexcept {
    if(on_&&now>=published_at_&&now-published_at_>=publication_lease_ms)retire_on();
}
bool FlightMux::publish(const EsmVeinFlightIntentV1& p,std::uint64_t now) noexcept {
    if(p.struct_bytes!=24||p.abi_version!=1||!p.publisher_epoch||p.requested_on>1||p.reserved)return false;
    if(!advance_time(now))return false;
    expire(now);
    const bool requested=p.requested_on!=0;
    if(p.publisher_epoch<epoch_)return false;
    if(p.publisher_epoch==epoch_){
        if(retired_||requested!=on_)return false;
        published_at_=now;return true;
    }
    if(revision_==(std::numeric_limits<std::uint64_t>::max)()){retire_on();return false;}
    epoch_=p.publisher_epoch;published_at_=now;on_=requested;retired_=false;++revision_;
    have_on_write_=false;
    if(on_){off_remaining_=0;off_priority_=false;flight_turn_=true;}
    else {off_remaining_=off_publications;off_priority_=true;}
    return true;
}
bool FlightMux::needs_on_context(const PacketFrame& frame,std::uint64_t now) const noexcept {
    return on_&&now>=published_at_&&now-published_at_<publication_lease_ms&&
        (!clock_seen_||now>=clock_high_)&&frame.publication_revision==revision_&&
        (!have_on_write_||(now>=last_on_write_&&now-last_on_write_>=on_interval_ms));
}
PacketFrame FlightMux::begin_frame() const noexcept {return {revision_,false};}
MuxResult FlightMux::write(PacketFrame& frame,PacketTarget output,const VeinWord* vein,std::uint64_t now,bool allow_on) noexcept {
    if(frame.attempted)return {};
    const bool time_valid=advance_time(now);
    if(time_valid)expire(now);
    const bool on_candidate=time_valid&&needs_on_context(frame,now);
    const bool due_on=allow_on&&on_candidate;
    const bool off_pending=off_remaining_!=0;
    const bool flight_pending=off_pending||due_on;
    if(!flight_pending&&!vein)return {PacketStatus::inactive,PacketFamily::none,on_candidate,due_on,off_pending,false,false};
    const bool choose_flight=flight_pending&&(!vein||off_priority_||flight_turn_);
    frame.attempted=true;
    const auto family=choose_flight?PacketFamily::flight:PacketFamily::vein;
    const bool flight_on=choose_flight&&off_remaining_==0&&on_;
    if(!output.commit)return {PacketStatus::unavailable,family,on_candidate,due_on,off_pending,choose_flight,flight_on};
    if(!choose_flight){
        const auto status=output.commit(output.context,vein->magic,vein->encoded);
        if(status==PacketStatus::written)flight_turn_=true;
        return {status,family,on_candidate,due_on,off_pending,false,false};
    }
    auto next=(serial_+1)&nonce_mask;if(!next)next=1;
    const auto status=output.commit(output.context,namespace_magic,flight_tag|(flight_on?on_mask:0u)|next);
    if(status==PacketStatus::written){
        serial_=next;flight_turn_=false;
        if(flight_on){last_on_write_=now;have_on_write_=true;}
        else {--off_remaining_;off_priority_=false;}
    }
    return {status,family,on_candidate,due_on,off_pending,true,flight_on};
}
PacketStatus MuxPacket::commit_vein(void* context,std::uint32_t magic,std::uint32_t encoded) noexcept {
    auto& packet=*static_cast<MuxPacket*>(context);
    const VeinWord vein{magic,encoded};
    packet.result_=packet.mux_.write(packet.frame_,packet.output_,&vein,packet.now_,packet.allow_on_);
    if(packet.result_.family==PacketFamily::vein)return packet.result_.status;
    // Flight now occupies the optional field; A6's writer must not consume its serial.
    return packet.result_.status==PacketStatus::written?PacketStatus::occupied:packet.result_.status;
}
MuxResult MuxPacket::finish() noexcept {
    if(!finished_&&!frame_.attempted)result_=mux_.write(frame_,output_,nullptr,now_,allow_on_);
    finished_=true;return result_;
}
}
