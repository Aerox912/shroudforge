#pragma once

#include <cstddef>
#include <cstdint>
#include <vector>

namespace shroudweave::flight {

struct TargetLayout {
    std::uint32_t site_rva{};
    std::uint32_t lower_limit_rva{};
    std::uint32_t upper_limit_rva{};
};

enum class ScanStatus {
    found,
    malformed_image,
    unsupported_architecture,
    invalid_sections,
    no_signature,
    ambiguous_signature,
    invalid_operand,
    invalid_constants,
};

struct ScanResult {
    ScanStatus status{ScanStatus::malformed_image};
    TargetLayout target{};
    std::size_t matches{};
};

ScanResult FindTargetLayout(const std::uint8_t* mapped_image, std::size_t image_size) noexcept;
bool MapPeFile(const std::uint8_t* file_bytes, std::size_t file_size, std::vector<std::uint8_t>& mapped_image) noexcept;
const char* ScanStatusName(ScanStatus status) noexcept;

class ToggleState {
public:
    bool enabled() const noexcept { return enabled_; }
    bool Sample(bool key_down, bool focused) noexcept;
    void Disarm() noexcept { armed_ = false; }

private:
    bool armed_{};
    bool enabled_{};
};

} // namespace shroudweave::flight
