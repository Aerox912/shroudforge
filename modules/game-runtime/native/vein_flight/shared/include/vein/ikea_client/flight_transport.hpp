// SPDX-License-Identifier: MIT
#pragma once
#include "vein/ikea_client/coordinator.hpp"
#include "vein/ikea_client/flight_intent_abi.h"
namespace vein::ikea_client {
enum class PacketFamily { none,vein,flight };
struct PacketFrame { std::uint64_t publication_revision=0; bool attempted=false; };
struct VeinWord { std::uint32_t magic,encoded; };
struct MuxResult {
    PacketStatus status=PacketStatus::inactive;
    PacketFamily family=PacketFamily::none;
    bool on_context_candidate=false,on_due=false,off_pending=false,flight_attempted=false,flight_on=false;
};
// Caller-serialized value state. No producer or caller pointer is retained.
// Wire identity: F6-VEIN-WIRE-V1.json SHA256 30e79e080faa2f64c788954494bc4fc9e8751e4486513ff09d6e255da7b5cbaf.
class FlightMux {
    std::uint64_t epoch_=0,revision_=0,published_at_=0,clock_high_=0,last_on_write_=0;
    std::uint32_t serial_=0,off_remaining_=0;
    bool clock_seen_=false,on_=false,retired_=false,have_on_write_=false;
    bool flight_turn_=true,off_priority_=false;
    void retire_on() noexcept;
    [[nodiscard]] bool advance_time(std::uint64_t now_ms) noexcept;
    void expire(std::uint64_t now_ms) noexcept;
public:
    // Reset owned publication state, preserving the existing connection's wire
    // sequence, monotonic publisher epoch, clock and A6 arbitration state.
    void native_reset() noexcept;
    [[nodiscard]] bool publish(const EsmVeinFlightIntentV1&,std::uint64_t now_ms) noexcept;
    [[nodiscard]] PacketFrame begin_frame() const noexcept;
    [[nodiscard]] bool needs_on_context(const PacketFrame&,std::uint64_t now_ms) const noexcept;
    [[nodiscard]] MuxResult write(PacketFrame&,PacketTarget,const VeinWord*,std::uint64_t now_ms,bool allow_on) noexcept;
};
class MuxPacket {
    FlightMux& mux_;
    PacketTarget output_;
    PacketFrame frame_;
    std::uint64_t now_;
    bool allow_on_,finished_=false;
    MuxResult result_{};
    static PacketStatus commit_vein(void*,std::uint32_t,std::uint32_t) noexcept;
public:
    MuxPacket(FlightMux& mux,PacketTarget output,PacketFrame frame,std::uint64_t now,bool allow_on) noexcept
      :mux_(mux),output_(output),frame_(frame),now_(now),allow_on_(allow_on){}
    [[nodiscard]] PacketTarget vein_target() noexcept {return {this,commit_vein};}
    [[nodiscard]] MuxResult finish() noexcept;
};
}
