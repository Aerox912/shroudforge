#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include "vein/settings_file.hpp"

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <string>
#include <utility>

namespace vein::settings_file {
namespace {

enum class ReadStatus {
    ok,
    missing,
    too_large,
    access_denied,
    io_error,
    changed,
};

struct ReadResult {
    ReadStatus status = ReadStatus::io_error;
    std::string bytes;
    DWORD attributes = INVALID_FILE_ATTRIBUTES;
};

class Handle {
public:
    Handle() noexcept = default;
    explicit Handle(HANDLE value) noexcept : value_(value) {}

    ~Handle() {
        if (valid()) CloseHandle(value_);
    }

    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;

    Handle(Handle&& other) noexcept : value_(std::exchange(other.value_, INVALID_HANDLE_VALUE)) {}

    Handle& operator=(Handle&& other) noexcept {
        if (this == &other) return *this;
        if (valid()) CloseHandle(value_);
        value_ = std::exchange(other.value_, INVALID_HANDLE_VALUE);
        return *this;
    }

    [[nodiscard]] bool valid() const noexcept { return value_ != INVALID_HANDLE_VALUE; }
    [[nodiscard]] HANDLE get() const noexcept { return value_; }

    bool close() noexcept {
        if (!valid()) return true;
        if (CloseHandle(value_) == FALSE) return false;
        value_ = INVALID_HANDLE_VALUE;
        return true;
    }

private:
    HANDLE value_ = INVALID_HANDLE_VALUE;
};

class StagedFile {
public:
    StagedFile() = default;
    ~StagedFile() {
        file_.close();
        if (!committed_ && !path_.empty()) DeleteFileW(path_.c_str());
    }

    StagedFile(const StagedFile&) = delete;
    StagedFile& operator=(const StagedFile&) = delete;

    [[nodiscard]] const std::filesystem::path& path() const noexcept { return path_; }
    [[nodiscard]] HANDLE handle() const noexcept { return file_.get(); }

    void set_path(std::filesystem::path path) { path_ = std::move(path); }
    void set_handle(HANDLE handle) noexcept { file_ = Handle(handle); }
    bool close() noexcept { return file_.close(); }
    void mark_committed() noexcept { committed_ = true; }

private:
    std::filesystem::path path_;
    Handle file_;
    bool committed_ = false;
};

[[nodiscard]] Error map_error(DWORD error) noexcept {
    switch (error) {
    case ERROR_ACCESS_DENIED:
    case ERROR_SHARING_VIOLATION:
    case ERROR_LOCK_VIOLATION:
    case ERROR_WRITE_PROTECT:
    case ERROR_PRIVILEGE_NOT_HELD:
        return Error::access_denied;
    default:
        return Error::io_error;
    }
}

[[nodiscard]] bool valid_config_path(const std::filesystem::path& path) {
    return !path.empty() && path.is_absolute() && !path.filename().empty();
}

// Decode no text and rewrite no unrelated bytes. An optional UTF-8 BOM is
// excluded only from key parsing, and retained in the original byte snapshot.
[[nodiscard]] std::string_view settings_body(std::string_view bytes) noexcept {
    return bytes.starts_with("\xef\xbb\xbf") ? bytes.substr(3) : bytes;
}

[[nodiscard]] bool field_space(char value) noexcept {
    return value == ' ' || value == '\t' || value == '\r';
}

// This support phase persists radius only. Unknown entries, comments, leading
// zeros, whitespace, line endings and the final-newline convention survive.
// Known duplicate keys remain invalid through the unchanged settings parser.
[[nodiscard]] std::optional<std::string> patch_radius(
    const std::string& original, const Settings& requested) {
    const auto parsed = parse_settings(settings_body(original));
    if (!parsed.valid || requested.radius_multiplier < min_radius_multiplier ||
        requested.radius_multiplier > max_radius_multiplier) return std::nullopt;
    const auto& current = parsed.settings;
    if (requested.mode != current.mode || requested.activation_key != current.activation_key ||
        requested.radius_key != current.radius_key ||
        requested.toggle_migration_revision != current.toggle_migration_revision)
        return std::nullopt;
    if (requested.radius_multiplier == current.radius_multiplier) return original;

    std::size_t value_at = std::string::npos, value_size = 0;
    std::size_t position = original.size() - settings_body(original).size();
    bool saw_lf = false, saw_crlf = false;
    while (position < original.size()) {
        const auto newline = original.find('\n', position);
        const auto end = newline == std::string::npos ? original.size() : newline;
        if (newline != std::string::npos) {
            if (end > position && original[end - 1] == '\r') saw_crlf = true;
            else saw_lf = true;
        }
        auto left = position, right = end;
        while (left < right && field_space(original[left])) ++left;
        while (right > left && field_space(original[right - 1])) --right;
        if (left < right && original[left] != '#') {
            const auto equals = original.find('=', left);
            if (equals == std::string::npos || equals >= right) return std::nullopt;
            auto key_end = equals;
            while (key_end > left && field_space(original[key_end - 1])) --key_end;
            if (std::string_view(original).substr(left, key_end - left) == "radius") {
                if (value_at != std::string::npos) return std::nullopt;
                value_at = equals + 1;
                while (value_at < right && field_space(original[value_at])) ++value_at;
                value_size = right - value_at;
            }
        }
        position = newline == std::string::npos ? original.size() : newline + 1;
    }

    auto output = original;
    const char digit = static_cast<char>('0' + requested.radius_multiplier);
    if (value_at != std::string::npos) {
        if (!value_size) return std::nullopt;
        auto value = std::string(value_size, '0');
        value.back() = digit;
        output.replace(value_at, value_size, value);
    } else {
        // A missing entry can be appended only when an existing, unique line
        // convention is observable. Do not guess a convention for one-line or
        // mixed-ending input, and do not introduce serialized default fields.
        if (saw_lf == saw_crlf) return std::nullopt;
        for (std::size_t i = 0; i < original.size(); ++i)
            if (original[i] == '\r' &&
                (i + 1 == original.size() || original[i + 1] != '\n'))
                return std::nullopt;
        const auto ending = saw_crlf ? "\r\n" : "\n";
        if (original.ends_with('\n')) output += std::string("radius=") + digit + ending;
        else output += std::string(ending) + "radius=" + digit;
    }
    return output.size() <= max_settings_bytes ? std::optional(std::move(output)) : std::nullopt;
}

[[nodiscard]] bool is_missing_error(DWORD error) noexcept {
    return error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND;
}

[[nodiscard]] bool same_size_and_write_time(const BY_HANDLE_FILE_INFORMATION& left,
                                            const BY_HANDLE_FILE_INFORMATION& right) noexcept {
    return left.nFileSizeHigh == right.nFileSizeHigh &&
           left.nFileSizeLow == right.nFileSizeLow &&
           CompareFileTime(&left.ftLastWriteTime, &right.ftLastWriteTime) == 0;
}

[[nodiscard]] std::filesystem::path sibling_path(const std::filesystem::path& target,
                                                  const wchar_t* suffix) {
    static std::atomic<std::uint64_t> sequence{0};

    auto directory = target.parent_path();
    if (directory.empty()) directory = L".";

    const std::wstring name =
        L".vein-settings-" + std::to_wstring(GetCurrentProcessId()) + L"-" +
        std::to_wstring(GetTickCount64()) + L"-" +
        std::to_wstring(sequence.fetch_add(1, std::memory_order_relaxed)) + suffix;
    return directory / name;
}

[[nodiscard]] ReadResult read_file(const std::filesystem::path& path) {
    Handle file(CreateFileW(path.c_str(), GENERIC_READ,
                            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                            OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN,
                            nullptr));
    if (!file.valid()) {
        const DWORD error = GetLastError();
        if (is_missing_error(error)) return {ReadStatus::missing, {}, INVALID_FILE_ATTRIBUTES};
        return {map_error(error) == Error::access_denied ? ReadStatus::access_denied
                                                         : ReadStatus::io_error,
                {}, INVALID_FILE_ATTRIBUTES};
    }

    BY_HANDLE_FILE_INFORMATION before{};
    if (GetFileInformationByHandle(file.get(), &before) == FALSE) {
        return {map_error(GetLastError()) == Error::access_denied ? ReadStatus::access_denied
                                                                  : ReadStatus::io_error,
                {}, INVALID_FILE_ATTRIBUTES};
    }

    const std::uint64_t size =
        (static_cast<std::uint64_t>(before.nFileSizeHigh) << 32U) | before.nFileSizeLow;
    if (size > max_settings_bytes) {
        return {ReadStatus::too_large, {}, before.dwFileAttributes};
    }

    std::string bytes(static_cast<std::size_t>(size), '\0');
    std::size_t offset = 0;
    while (offset < bytes.size()) {
        const DWORD request = static_cast<DWORD>(bytes.size() - offset);
        DWORD read = 0;
        if (ReadFile(file.get(), bytes.data() + offset, request, &read, nullptr) == FALSE) {
            return {map_error(GetLastError()) == Error::access_denied ? ReadStatus::access_denied
                                                                      : ReadStatus::io_error,
                    {}, before.dwFileAttributes};
        }
        if (read == 0) return {ReadStatus::changed, {}, before.dwFileAttributes};
        offset += read;
    }

    char extra = '\0';
    DWORD extra_read = 0;
    if (ReadFile(file.get(), &extra, 1, &extra_read, nullptr) == FALSE) {
        return {map_error(GetLastError()) == Error::access_denied ? ReadStatus::access_denied
                                                                  : ReadStatus::io_error,
                {}, before.dwFileAttributes};
    }
    if (extra_read != 0) return {ReadStatus::changed, {}, before.dwFileAttributes};

    BY_HANDLE_FILE_INFORMATION after{};
    if (GetFileInformationByHandle(file.get(), &after) == FALSE) {
        return {map_error(GetLastError()) == Error::access_denied ? ReadStatus::access_denied
                                                                  : ReadStatus::io_error,
                {}, before.dwFileAttributes};
    }
    if (!same_size_and_write_time(before, after)) {
        return {ReadStatus::changed, {}, after.dwFileAttributes};
    }

    return {ReadStatus::ok, std::move(bytes), after.dwFileAttributes};
}

[[nodiscard]] Error read_error(ReadStatus status) noexcept {
    switch (status) {
    case ReadStatus::access_denied: return Error::access_denied;
    case ReadStatus::too_large: return Error::too_large;
    case ReadStatus::changed: return Error::conflict;
    case ReadStatus::ok:
    case ReadStatus::missing:
        return Error::none;
    case ReadStatus::io_error:
        return Error::io_error;
    }
    return Error::io_error;
}

[[nodiscard]] Error check_snapshot(const std::filesystem::path& path,
                                  bool expected_to_exist,
                                  const std::string& expected_bytes) {
    const ReadResult current = read_file(path);
    if (current.status == ReadStatus::missing) {
        return expected_to_exist ? Error::conflict : Error::none;
    }
    if (current.status == ReadStatus::too_large || current.status == ReadStatus::changed) {
        return Error::conflict;
    }
    if (current.status != ReadStatus::ok) return read_error(current.status);

    if (!expected_to_exist || current.bytes != expected_bytes) return Error::conflict;
    if ((current.attributes & FILE_ATTRIBUTE_READONLY) != 0) return Error::access_denied;
    return Error::none;
}

[[nodiscard]] Error create_staged_file(const std::filesystem::path& target,
                                       StagedFile& staged) {
    for (unsigned attempt = 0; attempt < 32; ++attempt) {
        const auto candidate = sibling_path(target, L".tmp");
        HANDLE file = CreateFileW(candidate.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW,
                                  FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_SEQUENTIAL_SCAN, nullptr);
        if (file == INVALID_HANDLE_VALUE) {
            const DWORD error = GetLastError();
            if (error == ERROR_FILE_EXISTS || error == ERROR_ALREADY_EXISTS) continue;
            return map_error(error);
        }
        staged.set_path(candidate);
        staged.set_handle(file);
        return Error::none;
    }
    return Error::io_error;
}

[[nodiscard]] Error write_and_flush(StagedFile& staged, const std::string& bytes) noexcept {
    if (bytes.size() > MAXDWORD) return Error::too_large;

    DWORD written = 0;
    if (WriteFile(staged.handle(), bytes.data(), static_cast<DWORD>(bytes.size()), &written,
                  nullptr) == FALSE) {
        return map_error(GetLastError());
    }
    if (written != bytes.size()) return Error::io_error;
    if (FlushFileBuffers(staged.handle()) == FALSE) return map_error(GetLastError());
    if (!staged.close()) return map_error(GetLastError());
    return Error::none;
}

} // namespace

struct SettingsFile::Impl {
    explicit Impl(std::filesystem::path input_path, FileOperations input_operations)
        : path(std::move(input_path)), operations(input_operations) {}

    std::filesystem::path path;
    FileOperations operations;
    bool loaded = false;
    bool file_existed = false;
    std::string original_bytes;
};

SettingsFile::SettingsFile(std::filesystem::path path, FileOperations operations)
    : impl_(std::make_unique<Impl>(std::move(path), operations)) {}

SettingsFile::~SettingsFile() = default;
SettingsFile::SettingsFile(SettingsFile&&) noexcept = default;
SettingsFile& SettingsFile::operator=(SettingsFile&&) noexcept = default;

LoadResult SettingsFile::load() {
    if (!impl_) return {Error::invalid_path, std::nullopt, false};

    impl_->loaded = false;
    impl_->file_existed = false;
    impl_->original_bytes.clear();

    if (!valid_config_path(impl_->path)) {
        return {Error::invalid_path, std::nullopt, false};
    }

    ReadResult read = read_file(impl_->path);
    if (read.status == ReadStatus::missing) {
        impl_->loaded = true;
        return {Error::none, Settings{}, false};
    }
    if (read.status != ReadStatus::ok) {
        const Error error = read_error(read.status);
        return {error, std::nullopt, read.status == ReadStatus::too_large ||
                                         read.status == ReadStatus::changed};
    }

    const SettingsParseResult parsed = parse_settings(settings_body(read.bytes));
    if (!parsed.valid) return {Error::invalid_settings, std::nullopt, true};

    impl_->loaded = true;
    impl_->file_existed = true;
    impl_->original_bytes = std::move(read.bytes);
    return {Error::none, parsed.settings, true};
}

Error SettingsFile::save(const Settings& settings) {
    if (!impl_) return Error::invalid_path;
    if (!valid_config_path(impl_->path)) return Error::invalid_path;
    if (!impl_->loaded) return Error::not_loaded;

    const auto serialized = patch_radius(impl_->original_bytes, settings);
    if (!serialized) return Error::invalid_settings;

    Error status = check_snapshot(impl_->path, impl_->file_existed, impl_->original_bytes);
    if (status != Error::none) return status;

    StagedFile staged;
    status = create_staged_file(impl_->path, staged);
    if (status != Error::none) return status;

    status = write_and_flush(staged, *serialized);
    if (status != Error::none) return status;

    status = check_snapshot(impl_->path, impl_->file_existed, impl_->original_bytes);
    if (status != Error::none) return status;

    bool committed = false;
    DWORD replacement_error = ERROR_SUCCESS;
    if (impl_->file_existed) {
        if (impl_->operations.replace_existing) {
            const auto result = impl_->operations.replace_existing(
                impl_->operations.context, impl_->path, staged.path(), nullptr);
            committed = result.committed;
            replacement_error = result.system_error;
        } else {
            committed = ReplaceFileW(impl_->path.c_str(), staged.path().c_str(),
                                     nullptr, 0, nullptr, nullptr) != FALSE;
            if (!committed) replacement_error = GetLastError();
        }
    } else {
        committed = MoveFileExW(staged.path().c_str(), impl_->path.c_str(),
                                MOVEFILE_WRITE_THROUGH) != FALSE;
        if (!committed) replacement_error = GetLastError();
    }
    // A failed OS replacement can have an ambiguous outcome. Do not restore,
    // retry or assert preservation. The consumer latches publication off.
    if (!committed) return map_error(replacement_error);
    staged.mark_committed();
    const ReadResult after = read_file(impl_->path);
    if (after.status != ReadStatus::ok)
        return after.status == ReadStatus::missing ? Error::conflict : read_error(after.status);
    if (after.bytes != *serialized) return Error::conflict;
    impl_->file_existed = true;
    impl_->original_bytes = *serialized;
    return Error::none;
}

} // namespace vein::settings_file
