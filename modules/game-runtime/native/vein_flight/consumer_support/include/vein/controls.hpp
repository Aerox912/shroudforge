#pragma once

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

namespace vein {

inline constexpr std::uint16_t default_activation_key = 192;
inline constexpr std::uint16_t default_toggle_activation_key = 114;
inline constexpr std::uint16_t default_radius_key = 123;
inline constexpr unsigned min_radius_multiplier = 1;
inline constexpr unsigned max_radius_multiplier = 3;
inline constexpr unsigned default_radius_multiplier = 2;
inline constexpr std::size_t max_settings_bytes = 1024;

enum class Mode : std::uint8_t {
    Hold,
    Toggle,
    Always,
};

struct InputSample {
    bool activation_key_down = false;
    bool radius_key_down = false;
    bool gameplay_focused = false;
    bool input_captured = false;
};

struct InputResult {
    bool active = false;
    bool radius_changed = false;
    unsigned radius_multiplier = default_radius_multiplier;
};

class Controls {
public:
    explicit Controls(Mode mode = Mode::Hold,
                      unsigned radius_multiplier = default_radius_multiplier) noexcept;

    InputResult sample(InputSample input) noexcept;
    void reset_session() noexcept;
    void set_mode(Mode mode) noexcept;

    [[nodiscard]] Mode mode() const noexcept;
    [[nodiscard]] unsigned radius() const noexcept;
    bool set_radius(unsigned multiplier) noexcept;

private:
    Mode mode_;
    unsigned radius_multiplier_;
    bool toggle_active_ = false;
    bool activation_was_down_ = false;
    bool activation_armed_ = false;
    bool radius_was_down_ = false;
    bool radius_armed_ = false;
};

struct Settings {
    Mode mode = Mode::Hold;
    std::uint16_t activation_key = mode == Mode::Toggle ? default_toggle_activation_key : default_activation_key;
    unsigned radius_multiplier = default_radius_multiplier;
    std::uint32_t toggle_migration_revision = 0;
    std::uint16_t radius_key = default_radius_key;
};

struct SettingsParseResult {
    Settings settings{};
    bool valid = false;
};

[[nodiscard]] SettingsParseResult parse_settings(std::string_view text) noexcept;
[[nodiscard]] std::optional<std::string> serialize_settings(const Settings& settings);
// Revision zero is invalid and is a no-op. A higher nonzero revision updates
// bookkeeping while preserving the user's mode and settings. Revisions are
// assigned directly, so migration cannot wrap or increment.
[[nodiscard]] Settings migrate_settings(Settings settings, std::uint32_t target_revision) noexcept;

} // namespace vein
