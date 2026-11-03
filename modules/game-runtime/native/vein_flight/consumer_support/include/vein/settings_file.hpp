#pragma once

#include "vein/controls.hpp"

#include <cstdint>
#include <filesystem>
#include <memory>
#include <optional>

namespace vein::settings_file {

// The caller must provide an absolute Windows path to a file owned by this
// module. Relative, drive-relative, and root-relative paths are rejected. This
// helper does not discover, import, or migrate XHL or vmkeys configuration.
enum class Error : std::uint8_t {
    none,
    invalid_path,
    not_loaded,
    too_large,
    invalid_settings,
    access_denied,
    io_error,
    conflict,
};

struct LoadResult {
    Error error = Error::io_error;
    std::optional<Settings> settings;
    bool file_existed = false;

    [[nodiscard]] explicit operator bool() const noexcept {
        return error == Error::none && settings.has_value();
    }
};

// A narrow injectable OS replacement boundary. Production never supplies a
// backup path. The callback reports the actual system error without recovery.
struct ReplaceResult { bool committed = false; std::uint32_t system_error = 0; };
struct FileOperations {
    void* context = nullptr;
    ReplaceResult (*replace_existing)(void*, const std::filesystem::path&,
        const std::filesystem::path&, const std::filesystem::path*) noexcept = nullptr;
};

class SettingsFile {
public:
    explicit SettingsFile(std::filesystem::path path, FileOperations operations = {});
    ~SettingsFile();

    SettingsFile(SettingsFile&&) noexcept;
    SettingsFile& operator=(SettingsFile&&) noexcept;

    SettingsFile(const SettingsFile&) = delete;
    SettingsFile& operator=(const SettingsFile&) = delete;

    // A missing file returns the parser defaults without creating a file.
    // Existing malformed, oversized, or unreadable files return an error.
    [[nodiscard]] LoadResult load();

    // Save requires a prior successful load. Visible changes since that load
    // return conflict. A failed save never falls back to persisting defaults.
    [[nodiscard]] Error save(const Settings& settings);

private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
};

} // namespace vein::settings_file
