#pragma once
#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>

namespace SemanticAttribution::Native {
// Physical sites belong only to the exact archived server profile. They are
// not logical ledger events and confer no execution or memory authority.
enum class CapturePhase : std::uint8_t { Before, After, Both };
enum class RelocationKind : std::uint8_t { None, OriginalStack, RipRelative };
struct Instruction final {
    std::array<std::uint8_t, 15> bytes{};
    std::uint8_t size{};
    RelocationKind relocation{};
    std::uint8_t displacement_offset{};
    std::uint8_t displacement_size{};
    std::uint8_t modrm_offset{};
    std::int32_t displacement{};
    std::uint32_t rip_target_rva{};
};
struct Site final {
    std::uint32_t rva{};
    std::string_view name;
    CapturePhase phase{};
    std::array<std::uint8_t, 16> expected{};
    std::uint8_t expected_size{};
    std::array<Instruction, 4> instructions{};
    std::uint8_t instruction_count{};
    std::uint32_t native_function_begin{};
    std::uint32_t native_function_end{};
    std::uint32_t native_unwind_rva{};
};
struct CodeRange final { std::uint32_t rva{}; std::uint8_t size{}; std::array<std::uint8_t,16> expected{}; };
struct SemanticDefinition final { std::uint32_t hash{}; std::uint64_t key{}; std::string_view name; };
struct RegistrationDefinition final {
    std::uint32_t descriptor_rva{},callback_rva{},flags_one{},flags_two{};
};
struct Profile final {
    std::string_view file_sha256;
    std::uint32_t image_size{};
    std::uint32_t timestamp{};
    std::span<const Site> sites;
    std::span<const CodeRange> unchanged_operations;
    std::span<const SemanticDefinition> semantic_catalog;
    std::span<const RegistrationDefinition> registration_catalog;
};
const Profile& ArchivedServerProfile() noexcept;
const Site* FindSite(std::uint32_t rva) noexcept;
// Relocates only the reviewed instruction classes. Original native RSP is the
// relay's RSP+8 because the original site enters with a balanced CALL.
bool RelocateOriginal(const Site&, std::uintptr_t image_base,
                      std::uintptr_t output_pc, std::span<std::uint8_t> output,
                      std::size_t& written) noexcept;
struct RelayPlan final {
    static constexpr std::size_t SlotSize=0x200;
    static constexpr std::size_t CommonPointerOffset=0x100;
    static constexpr std::size_t UnwindOffset=0x110;
    std::array<std::uint8_t,SlotSize> bytes{};
    std::uint32_t code_size{};
    std::uint32_t before_return{};
    std::uint32_t after_return{};
};
bool BuildRelay(const Site&, std::uintptr_t image_base,
                std::uintptr_t relay_base, std::uintptr_t common_thunk,
                RelayPlan&) noexcept;
bool EncodeSiteCall(const Site&, std::uintptr_t image_base,
                    std::uintptr_t relay_base,
                    std::span<std::uint8_t> replacement) noexcept;
} // namespace SemanticAttribution::Native
