// SPDX-License-Identifier: MIT
#pragma once
#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>
namespace vein::ikea_client {
struct Guard {std::uint32_t rva,size;std::string_view sha256;bool executable=true;};
struct PointerGuard {std::uint32_t rva,target_rva;};
// Single current-build contract, rederived from inert old/new native code.
inline constexpr std::uint32_t packer_rva=0x3aba10;
inline constexpr std::uint32_t input_producer_rva=0x3a8970;
inline constexpr std::uint32_t host_attempt_rva=0x109ce50;
inline constexpr std::uint32_t peer_add_rva=0x108f4c0;
inline constexpr std::uint32_t peer_lookup_rva=0x1094380;
inline constexpr std::uint32_t peer_remove_rva=0x10915e0;
inline constexpr std::uint32_t peer_retire_rva=0x10917c0;
inline constexpr std::uint32_t roster_reset_rva=0x1091380;
inline constexpr std::uint32_t input_packet_caller_rva=0x3a918c;
inline constexpr std::uint32_t input_transport_caller_rva=0x863db7;
inline constexpr std::uint32_t input_transport_second_caller_rva=0x87a815;
inline constexpr std::uint32_t host_add_caller_rva=0x109cff0;
inline constexpr std::uint32_t add_lookup_caller_rva=0x108f57e;
inline constexpr std::uint32_t refresh_lookup_caller_rva=0x10950b2;
inline constexpr std::array current_guards{
 Guard{0x3aba10,0x8d1,"3304fb742d852133688b204b765059221491b57b50ad3515498ea8c32e3c3728"},
 Guard{0x3a9158,0x10e,"10801ebcc3d5fe247abc8a9bf1e2c124097391a8ee973af3fc2dcf64d064e0c4"},
 Guard{0x287b108,0xc,"31fd90736e7b34c166eb2856013686e6f6abe972d914ca24738122274a363c69",false},
 Guard{0x1eb79a0,0x14,"35e41fcc6b8100dd284d13835e6d72cd48d074c71457aaf53548eb6627b5e251",false},
};
// These digests cover complete hook code and its observed call/layout contract.
// File SHA, build number, timestamp and image size are provenance, not admission.
[[nodiscard]] bool validate_mapped_profile(std::span<const std::byte> image,std::span<const Guard> guards=current_guards) noexcept;
// Zero load_base uses the PE preferred base for inert file fixtures. Runtime
// callers supply the actual module base, allowing ASLR without hashing VAs.
[[nodiscard]] bool validate_mapped_layout(std::span<const std::byte> image,std::uintptr_t load_base=0) noexcept;
}
