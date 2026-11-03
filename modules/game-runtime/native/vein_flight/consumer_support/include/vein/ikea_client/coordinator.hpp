// SPDX-License-Identifier: MIT
#pragma once
#include "vein/controls.hpp"
#include "vein/settings_file.hpp"
#include <array>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <mutex>
#include <optional>
#include <span>
namespace vein::ikea_client {
inline constexpr std::size_t presence_offset=0x434, magic_offset=0x438, encoded_offset=0x43c, packet_bytes=0x440;
inline constexpr std::uint32_t namespace_magic=0x434c4858, tag=0xa6000000, nonce_mask=0x003fffff;
inline constexpr std::uint64_t sampled_input_max_age_ms=32;
enum class PacketStatus { written, inactive, occupied, short_buffer, invalid_radius, unavailable };
// A synchronous commit runs under coordinator serialization. It must return
// written only after publishing the optional-field presence marker. A failing
// commit never consumes a serial. The native adapter owns foreign-memory safety.
struct PacketTarget {
 void* context;
 PacketStatus (*commit)(void*,std::uint32_t,std::uint32_t) noexcept;
};
class PacketWriter {
public:
 explicit PacketWriter(std::uint32_t last_serial=0) noexcept;
 [[nodiscard]] PacketStatus write(std::span<std::byte> packet,bool active,unsigned radius) noexcept;
 [[nodiscard]] PacketStatus write(PacketTarget target,bool active,unsigned radius) noexcept;
 [[nodiscard]] std::uint32_t last_serial() const noexcept;
private: std::uint32_t serial_;
};
enum class Focus { unknown, gameplay, other };
enum class Capture { unknown, clear, captured };
struct InputObservation {
 std::array<std::uint64_t,4> keys{};
 Focus focus=Focus::unknown;
 Capture capture=Capture::unknown;
 [[nodiscard]] bool key_down(std::uint16_t key) const noexcept {return key<256 && (keys[key/64]&(std::uint64_t{1}<<(key%64)))!=0;}
};
struct NoticeEvents {std::optional<bool> toggle;std::optional<unsigned> radius;};
struct PacketResult {PacketStatus status=PacketStatus::unavailable;InputResult controls{};};
// Epoch is a module activation scope, never a native connection or protocol ID.
// The server owns sender identity/retirement; no acknowledgement is manufactured.
class Coordinator {
public:
 explicit Coordinator(std::filesystem::path absolute_module_config, settings_file::FileOperations operations = {});
 [[nodiscard]] settings_file::LoadResult initialize();
 [[nodiscard]] bool begin_activation(std::uint64_t epoch);
 void retire_activation(std::uint64_t epoch);
 void reset_native_session(std::uint64_t epoch);
 [[nodiscard]] InputResult observe_input(std::uint64_t epoch,const InputObservation& input);
 // The management worker records a complete sample and its monotonic completion tick.
 [[nodiscard]] InputResult observe_worker_input(std::uint64_t epoch,const InputObservation& input,std::uint64_t completed_tick_ms);
 [[nodiscard]] InputResult observe_worker_input(std::uint64_t epoch,std::uint64_t native_session_generation,const InputObservation& input,std::uint64_t completed_tick_ms);
 [[nodiscard]] PacketResult on_packet(std::uint64_t epoch,std::span<std::byte> packet,const InputObservation& input);
 [[nodiscard]] PacketResult on_packet(std::uint64_t epoch,PacketTarget target,const InputObservation& input);
 // Native packet path: context is observed per callback; Hold also supplies a fresh activation-key state.
 [[nodiscard]] PacketResult on_sampled_packet(std::uint64_t epoch,PacketTarget target,const InputObservation& context,bool hold_activation_key_down,std::uint64_t now_tick_ms);
 [[nodiscard]] PacketResult on_sampled_packet(std::uint64_t epoch,std::uint64_t native_session_generation,PacketTarget target,const InputObservation& context,bool hold_activation_key_down,std::uint64_t now_tick_ms);
 // Management-thread operation; never call from the native packet callback.
 [[nodiscard]] settings_file::Error flush_settings();
 // Host management thread drains bounded latest events; no GUI in callbacks.
 [[nodiscard]] NoticeEvents take_notices(std::uint64_t epoch);
 [[nodiscard]] std::optional<Settings> settings() const;
 [[nodiscard]] bool settings_pending() const;
 [[nodiscard]] std::uint32_t last_serial() const;
private:
 InputResult observe_locked(const InputObservation& input);
 void reset_native_session_locked() noexcept;
 mutable std::mutex state_mutex_;
 std::mutex file_mutex_;
 settings_file::SettingsFile file_;
 Settings settings_{};
 Controls controls_{};
 PacketWriter writer_{};
 InputObservation sampled_input_{};
 InputResult sampled_result_{};
 std::uint64_t sampled_epoch_=0,sampled_session_generation_=0,native_session_generation_=0,sampled_tick_ms_=0,last_sample_tick_ms_=0;
 bool sampled_valid_=false,have_last_sample_tick_=false;
 std::uint64_t activation_=0;
 NoticeEvents notices_{};
 bool notice_toggle_state_=false;
 bool loaded_=false,dirty_=false,file_existed_=false;
};
}
