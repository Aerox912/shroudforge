#include "vein/controls.hpp"

#include <charconv>
#include <system_error>

namespace vein {
namespace {

bool valid_mode(Mode mode) noexcept {
    return mode == Mode::Hold || mode == Mode::Toggle || mode == Mode::Always;
}

bool valid_radius(unsigned multiplier) noexcept {
    return multiplier >= min_radius_multiplier && multiplier <= max_radius_multiplier;
}

bool valid_key(unsigned key) noexcept {
    return key >= 1 && key <= 254;
}

std::string_view mode_name(Mode mode) noexcept {
    switch (mode) {
    case Mode::Hold: return "hold";
    case Mode::Toggle: return "toggle";
    case Mode::Always: return "always";
    }
    return {};
}

std::string_view trim(std::string_view value) noexcept {
    while (!value.empty() && (value.front() == ' ' || value.front() == '\t' || value.front() == '\r')) {
        value.remove_prefix(1);
    }
    while (!value.empty() && (value.back() == ' ' || value.back() == '\t' || value.back() == '\r')) {
        value.remove_suffix(1);
    }
    return value;
}

bool parse_number(std::string_view value, std::uint32_t& number) noexcept {
    if (value.empty()) return false;
    std::uint32_t parsed = 0;
    const auto result = std::from_chars(value.data(), value.data() + value.size(), parsed, 10);
    if (result.ec != std::errc{} || result.ptr != value.data() + value.size()) return false;
    number = parsed;
    return true;
}

void append_number(std::string& output, std::uint32_t value) {
    char buffer[10]{};
    const auto result = std::to_chars(buffer, buffer + sizeof(buffer), value, 10);
    output.append(buffer, result.ptr);
}

} // namespace

Controls::Controls(Mode mode, unsigned radius_multiplier) noexcept
    : mode_(valid_mode(mode) ? mode : Mode::Hold),
      radius_multiplier_(valid_radius(radius_multiplier) ? radius_multiplier : default_radius_multiplier) {}

InputResult Controls::sample(InputSample input) noexcept {
    const bool eligible = input.gameplay_focused && !input.input_captured;
    bool active = false;
    bool radius_changed = false;

    if (!eligible) {
        activation_armed_ = false;
        radius_armed_ = false;
    } else {
        switch (mode_) {
        case Mode::Hold:
            active = input.activation_key_down;
            break;
        case Mode::Toggle:
            if (!input.activation_key_down) activation_armed_ = true;
            if (activation_armed_ && input.activation_key_down && !activation_was_down_) {
                toggle_active_ = !toggle_active_;
            }
            active = toggle_active_;
            break;
        case Mode::Always:
            active = true;
            break;
        }

        if (!input.radius_key_down) radius_armed_ = true;
        if (radius_armed_ && input.radius_key_down && !radius_was_down_) {
            radius_multiplier_ = radius_multiplier_ % max_radius_multiplier + min_radius_multiplier;
            radius_changed = true;
        }
    }

    // Tracking physical state while blocked prevents focus/capture changes from
    // manufacturing a fresh press. A release in eligible gameplay rearms edges.
    activation_was_down_ = input.activation_key_down;
    radius_was_down_ = input.radius_key_down;
    return {active, radius_changed, radius_multiplier_};
}

void Controls::reset_session() noexcept {
    toggle_active_ = false;
    activation_was_down_ = false;
    activation_armed_ = false;
    radius_was_down_ = false;
    radius_armed_ = false;
}

void Controls::set_mode(Mode mode) noexcept {
    if (!valid_mode(mode) || mode == mode_) return;
    mode_ = mode;
    toggle_active_ = false;
    activation_armed_ = false;
}

Mode Controls::mode() const noexcept { return mode_; }
unsigned Controls::radius() const noexcept { return radius_multiplier_; }

bool Controls::set_radius(unsigned multiplier) noexcept {
    if (!valid_radius(multiplier)) return false;
    radius_multiplier_ = multiplier;
    return true;
}

SettingsParseResult parse_settings(std::string_view text) noexcept {
    if (text.size() > max_settings_bytes) return {};
    for (const char character : text) {
        if (character == '\0') return {};
    }

    Settings settings{};
    bool saw_version = false;
    bool saw_mode = false;
    bool saw_key = false;
    bool saw_radius = false;
    bool saw_radius_key = false;
    bool saw_migration_revision = false;

    std::size_t position = 0;
    while (position < text.size()) {
        const std::size_t newline = text.find('\n', position);
        const std::size_t end = newline == std::string_view::npos ? text.size() : newline;
        const auto line = trim(text.substr(position, end - position));
        position = newline == std::string_view::npos ? text.size() : newline + 1;

        if (line.empty() || line.front() == '#') continue;
        const std::size_t equals = line.find('=');
        if (equals == std::string_view::npos) return {};
        const auto key = trim(line.substr(0, equals));
        const auto value = trim(line.substr(equals + 1));
        if (key.empty()) return {};

        if (key == "version") {
            std::uint32_t version = 0;
            if (saw_version || !parse_number(value, version) || version != 1) return {};
            saw_version = true;
        } else if (key == "mode") {
            if (saw_mode) return {};
            if (value == "hold") settings.mode = Mode::Hold;
            else if (value == "toggle") settings.mode = Mode::Toggle;
            else if (value == "always") settings.mode = Mode::Always;
            else return {};
            saw_mode = true;
        } else if (key == "key") {
            std::uint32_t parsed = 0;
            if (saw_key || !parse_number(value, parsed) || parsed < 1 || parsed > 254) return {};
            settings.activation_key = static_cast<std::uint16_t>(parsed);
            saw_key = true;
        } else if (key == "radius") {
            std::uint32_t parsed = 0;
            if (saw_radius || !parse_number(value, parsed) || !valid_radius(parsed)) return {};
            settings.radius_multiplier = parsed;
            saw_radius = true;
        } else if (key == "radius_key") {
            std::uint32_t parsed = 0;
            if (saw_radius_key || !parse_number(value, parsed) || !valid_key(parsed)) return {};
            settings.radius_key = static_cast<std::uint16_t>(parsed);
            saw_radius_key = true;
        } else if (key == "toggle_migration_revision") {
            std::uint32_t revision = 0;
            if (saw_migration_revision || !parse_number(value, revision)) return {};
            settings.toggle_migration_revision = revision;
            saw_migration_revision = true;
        }
        // Unknown keys are ignored for forward compatibility. Known duplicate
        // keys and unknown settings versions remain errors above.
    }

    if (!saw_key && settings.mode == Mode::Toggle) settings.activation_key = default_toggle_activation_key;

    return {settings, true};
}

std::optional<std::string> serialize_settings(const Settings& settings) {
    if (!valid_mode(settings.mode) || settings.activation_key < 1 || settings.activation_key > 254 ||
        !valid_radius(settings.radius_multiplier) || !valid_key(settings.radius_key)) {
        return std::nullopt;
    }

    std::string output;
    output.reserve(96);
    output += "version=1\nmode=";
    output += mode_name(settings.mode);
    output += "\nkey=";
    append_number(output, settings.activation_key);
    output += "\nradius=";
    append_number(output, settings.radius_multiplier);
    output += "\ntoggle_migration_revision=";
    append_number(output, settings.toggle_migration_revision);
    output += "\nradius_key=";
    append_number(output, settings.radius_key);
    output += '\n';
    return output;
}

Settings migrate_settings(Settings settings, std::uint32_t target_revision) noexcept {
    if (target_revision != 0 && target_revision > settings.toggle_migration_revision) {
        settings.toggle_migration_revision = target_revision;
    }
    return settings;
}

} // namespace vein
