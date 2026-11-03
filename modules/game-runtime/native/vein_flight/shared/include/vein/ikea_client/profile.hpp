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
inline constexpr std::uint32_t packer_rva=0x3ab930;
inline constexpr std::uint32_t input_producer_rva=0x3a8890;
inline constexpr std::uint32_t host_attempt_rva=0x109dba0;
inline constexpr std::uint32_t peer_add_rva=0x1090210;
inline constexpr std::uint32_t peer_lookup_rva=0x10950d0;
inline constexpr std::uint32_t peer_remove_rva=0x1092330;
inline constexpr std::uint32_t peer_retire_rva=0x1092510;
inline constexpr std::uint32_t roster_reset_rva=0x10920d0;
inline constexpr std::uint32_t input_packet_caller_rva=0x3a90ac;
inline constexpr std::uint32_t input_transport_caller_rva=0x863cf7;
inline constexpr std::uint32_t input_transport_second_caller_rva=0x87a755;
inline constexpr std::uint32_t host_add_caller_rva=0x109dd40;
inline constexpr std::uint32_t add_lookup_caller_rva=0x10902ce;
inline constexpr std::uint32_t refresh_lookup_caller_rva=0x1095e02;
inline constexpr std::array current_guards{
 Guard{0x3ab930,0x8d1,"e6da934a3aa6adae435ec341cf0671b7f563b96e66bede550f98b7c85c514b85"},
 Guard{0x3a9078,0x10e,"00abb67e547d6c5574fafb3a1d88d9f7bd33b47cad778cb26a67c9dcf5408515"},
 Guard{0x287c0b4,0xc,"9033c947bf1abc7ffab673347b3450d5bf05460bbc417df275eee3fa0aab42ea",false},
 Guard{0x1eb8930,0x14,"35e41fcc6b8100dd284d13835e6d72cd48d074c71457aaf53548eb6627b5e251",false},
};
// These digests cover complete hook code and its observed call/layout contract.
// File SHA, build number, timestamp and image size are provenance, not admission.
[[nodiscard]] bool validate_mapped_profile(std::span<const std::byte> image,std::span<const Guard> guards=current_guards) noexcept;
// Zero load_base uses the PE preferred base for inert file fixtures. Runtime
// callers supply the actual module base, allowing ASLR without hashing VAs.
[[nodiscard]] bool validate_mapped_layout(std::span<const std::byte> image,std::uintptr_t load_base=0) noexcept;
}
