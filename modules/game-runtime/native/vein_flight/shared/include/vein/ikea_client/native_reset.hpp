// SPDX-License-Identifier: MIT
#pragma once
#include "vein/ikea_client/flight_transport.hpp"
namespace vein::ikea_client {
enum class FlightPublishDecision { accepted,inactive,invalid,stale_serial,lifecycle_transition,missing_association,mux_rejected,missing_input };
enum class HostFinishDisposition : std::uint8_t { no_ticket,success,failed_current,failed_stale };
enum class FlightAuthority { selected_peer,current_input };
// Owned values only. Every method is serialized by the adapter's SAME gate as
// FlightMux::write. No native pointer is dereferenced or returned by this state.
// Serial and host tickets are LOCAL observations, never engine generations.
class NativeReset {
 FlightMux& mux_;
 FlightAuthority authority_;
 std::uint64_t serial_;
 std::uintptr_t input_context_=0;
 std::uint32_t input_channel_=0;
 std::uint64_t input_seen_=0;
 std::uintptr_t selected_manager_=0;
 std::uintptr_t host_manager_=0;
 std::uint16_t selected_key_=0;
 std::uint32_t hosts_=0,retires_=0;
 bool attempted_=false,active_=false,exhausted_=false;
 void reset() noexcept;
public:
 explicit NativeReset(FlightMux& mux,std::uint64_t initial_serial=1,FlightAuthority authority=FlightAuthority::selected_peer) noexcept;
 // Call only from the pinned packer within the qualified synchronous producer
 // scope. This cache admits publications; it never authorizes a packet write.
 [[nodiscard]] std::uint64_t observe_input(std::uintptr_t context,std::uint32_t full_channel,std::uint64_t now) noexcept;
 [[nodiscard]] bool input_current(std::uintptr_t context,std::uint32_t full_channel,std::uint64_t serial,std::uint64_t now) const noexcept;
 [[nodiscard]] bool open() noexcept;
 void close() noexcept;
 [[nodiscard]] bool available() const noexcept {return active_&&!exhausted_;}
 [[nodiscard]] bool selected_association() const noexcept {return selected_manager_!=0&&selected_key_!=0;}
 [[nodiscard]] std::uint64_t serial() const noexcept {return serial_;}
 [[nodiscard]] std::uint64_t begin_host(std::uintptr_t manager) noexcept;
 HostFinishDisposition finish_host(std::uint64_t ticket,bool success) noexcept;
 [[nodiscard]] bool select(std::uint64_t ticket,std::uintptr_t manager,std::uint16_t full_key) noexcept;
 [[nodiscard]] bool refresh(std::uintptr_t manager,std::uint16_t full_key) noexcept;
 [[nodiscard]] bool begin_retire(std::uintptr_t manager,std::uint16_t full_key,bool roster) noexcept;
 void finish_retire(bool matched) noexcept;
 [[nodiscard]] FlightPublishDecision publish_decision(const EsmVeinFlightIntentV2&,std::uint64_t now) noexcept;
 [[nodiscard]] bool publish(const EsmVeinFlightIntentV2&,std::uint64_t now) noexcept;
 [[nodiscard]] bool publish_v1(const EsmVeinFlightIntentV1&,std::uint64_t now) noexcept;
};
}
