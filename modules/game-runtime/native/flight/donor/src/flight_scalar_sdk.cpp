#include "flight_scalar_sdk.hpp"
#include "flight_scalar_sdk_pin.hpp"

#include <bcrypt.h>

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>

namespace shroudweave::flight {
namespace {

constexpr wchar_t kScalarSdkFileName[] = L"shroudweave-native-scalar-sdk.dll";
using detail::kExpectedSdkLength;
using detail::kExpectedSdkSha256;
constexpr DWORD kSafeSearchFlags =
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS;

bool HashExactSdkFile(HANDLE file) noexcept {
    LARGE_INTEGER initialSize{};
    if (file == INVALID_HANDLE_VALUE || !GetFileSizeEx(file, &initialSize) ||
        initialSize.QuadPart != static_cast<LONGLONG>(kExpectedSdkLength)) {
        return false;
    }

    BCRYPT_ALG_HANDLE algorithm{};
    BCRYPT_HASH_HANDLE hash{};
    std::array<UCHAR, 4096> object{};
    std::array<UCHAR, 4096> buffer{};
    std::array<UCHAR, 32> digest{};
    DWORD hashLength{};
    DWORD bytes{};
    bool valid = false;
    if (BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) < 0) {
        goto cleanup;
    }
    if (BCryptGetProperty(algorithm, BCRYPT_HASH_LENGTH,
                          reinterpret_cast<PUCHAR>(&hashLength), sizeof(hashLength),
                          &bytes, 0) < 0 || hashLength != digest.size()) {
        goto cleanup;
    }
    if (BCryptCreateHash(algorithm, &hash, object.data(),
                         static_cast<ULONG>(object.size()), nullptr, 0, 0) < 0) {
        goto cleanup;
    }

    {
        LARGE_INTEGER start{};
        if (!SetFilePointerEx(file, start, nullptr, FILE_BEGIN)) {
            goto cleanup;
        }
        std::uint64_t consumed{};
        while (consumed < kExpectedSdkLength) {
            DWORD requested = static_cast<DWORD>(
                (kExpectedSdkLength - consumed) < buffer.size()
                    ? (kExpectedSdkLength - consumed)
                    : buffer.size());
            DWORD read{};
            if (!ReadFile(file, buffer.data(), requested, &read, nullptr) || read != requested ||
                BCryptHashData(hash, buffer.data(), read, 0) < 0) {
                goto cleanup;
            }
            consumed += read;
        }
        if (BCryptFinishHash(hash, digest.data(), static_cast<ULONG>(digest.size()), 0) < 0) {
            goto cleanup;
        }
        LARGE_INTEGER finalSize{};
        valid = GetFileSizeEx(file, &finalSize) && finalSize.QuadPart == initialSize.QuadPart &&
                std::memcmp(digest.data(), kExpectedSdkSha256.data(), digest.size()) == 0;
    }

cleanup:
    if (hash != nullptr) BCryptDestroyHash(hash);
    if (algorithm != nullptr) BCryptCloseAlgorithmProvider(algorithm, 0);
    return valid;
}

bool IsApiV1(const ShroudweaveNativeScalarApiV1* api) noexcept {
    return api != nullptr && api->cbSize == sizeof(ShroudweaveNativeScalarApiV1) &&
           api->version == SHROUDWEAVE_NATIVE_SCALAR_API_VERSION_V1 &&
           api->capabilityFlags == SHROUDWEAVE_NATIVE_SCALAR_CAP_SYNCHRONOUS_FRAME_V1 &&
           api->reserved[0] == 0 && api->reserved[1] == 0 && api->Register != nullptr &&
           api->Remove != nullptr;
}

bool SiblingPath(const void* callerAddress, std::array<wchar_t, 32'768>& path) noexcept {
    if (callerAddress == nullptr) return false;
    const auto addressValue = reinterpret_cast<std::uintptr_t>(callerAddress);
    HMODULE callerModule{};
    if (GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                               GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                           reinterpret_cast<LPCWSTR>(addressValue), &callerModule) == FALSE) {
        return false;
    }

    std::array<wchar_t, 32'768> callerPath{};
    const DWORD length = GetModuleFileNameW(
        callerModule, callerPath.data(), static_cast<DWORD>(callerPath.size()));
    if (length == 0 || length >= callerPath.size()) return false;

    wchar_t* separator = wcsrchr(callerPath.data(), L'\\');
    wchar_t* slash = wcsrchr(callerPath.data(), L'/');
    if (slash != nullptr && (separator == nullptr || slash > separator)) separator = slash;
    if (separator == nullptr) return false;
    const std::size_t prefixLength = static_cast<std::size_t>(separator + 1 - callerPath.data());
    const std::size_t fileLength = std::size(kScalarSdkFileName) - 1;
    if (prefixLength + fileLength + 1 > path.size()) return false;
    std::memcpy(path.data(), callerPath.data(), prefixLength * sizeof(wchar_t));
    std::memcpy(path.data() + prefixLength, kScalarSdkFileName,
                (fileLength + 1) * sizeof(wchar_t));
    return true;
}

bool ModulePathMatches(HMODULE module, const wchar_t* expected) noexcept {
    std::array<wchar_t, 32'768> actual{};
    const DWORD length = GetModuleFileNameW(
        module, actual.data(), static_cast<DWORD>(actual.size()));
    return length != 0 && length < actual.size() &&
           CompareStringOrdinal(expected, -1, actual.data(), -1, TRUE) == CSTR_EQUAL;
}

} // namespace

bool NativeScalarSdk::LoadForAddress(const void* callerAddress) noexcept {
    if (Ready() || load_attempted_ || callerAddress == nullptr) return Ready();
    load_attempted_ = true;

    std::array<wchar_t, 32'768> path{};
    if (!SiblingPath(callerAddress, path)) {
        load_error_ = GetLastError();
        if (load_error_ == ERROR_SUCCESS) load_error_ = ERROR_BAD_PATHNAME;
        return false;
    }

    // Keep the file open without write/delete sharing until the exact image has
    // been mapped. This binds the hash check and LoadLibraryExW to one file.
    HANDLE file = CreateFileW(path.data(), GENERIC_READ, FILE_SHARE_READ, nullptr,
                              OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN,
                              nullptr);
    if (file == INVALID_HANDLE_VALUE) {
        load_error_ = GetLastError();
        return false;
    }
    if (!HashExactSdkFile(file)) {
        load_error_ = ERROR_CRC;
        CloseHandle(file);
        return false;
    }

    HMODULE module = LoadLibraryExW(path.data(), nullptr, kSafeSearchFlags);
    if (module == nullptr) {
        load_error_ = GetLastError();
        CloseHandle(file);
        return false;
    }
    if (!ModulePathMatches(module, path.data())) {
        load_error_ = ERROR_MOD_NOT_FOUND;
        if (module != nullptr) FreeLibrary(module);
        CloseHandle(file);
        return false;
    }

    const auto getter = reinterpret_cast<ShroudweaveGetNativeScalarApiV1Fn>(
        GetProcAddress(module, "ShroudweaveGetNativeScalarApiV1"));
    const auto* api = getter != nullptr ? getter() : nullptr;
    if (!IsApiV1(api)) {
        load_error_ = getter == nullptr ? GetLastError() : ERROR_REVISION_MISMATCH;
        FreeLibrary(module);
        CloseHandle(file);
        return false;
    }

    using GetterV2 = const ShroudweaveNativeScalarApiV2*(SHROUDWEAVE_NATIVE_SCALAR_CALL*)();
    const auto getterV2 = reinterpret_cast<GetterV2>(
        GetProcAddress(module, "ShroudweaveGetNativeScalarApiV2"));
    const auto* apiV2 = getterV2 ? getterV2() : nullptr;
    if (apiV2 && apiV2->cbSize==sizeof(*apiV2) && apiV2->version==2 &&
        apiV2->capabilityFlags==3 && apiV2->reserved[0]==0 && apiV2->reserved[1]==0 &&
        apiV2->Register && apiV2->Remove && apiV2->Remove==api->Remove) apiV2_=apiV2;

    // Once admitted, the DLL and its immutable relay are process-lifetime.
    // Keep this file handle to prevent replacing the admitted sibling on disk.
    module_ = module;
    api_ = api;
    sdk_file_ = file;
    return true;
}

bool NativeScalarSdk::Ready() const noexcept {
    return module_ != nullptr && IsApiV1(api_);
}

bool NativeScalarSdk::Registered() const noexcept {
    return handle_ != 0;
}

bool NativeScalarSdk::RegistrationAccepted() const noexcept {
    return registration_accepted_;
}

HMODULE NativeScalarSdk::Module() const noexcept {
    return module_;
}

const ShroudweaveNativeScalarApiV1* NativeScalarSdk::Api() const noexcept {
    return Ready() ? api_ : nullptr;
}

DWORD NativeScalarSdk::LoadError() const noexcept {
    return load_error_;
}

std::int32_t NativeScalarSdk::RegisterOne(
    const ShroudweaveNativeScalarRequestV1& request) noexcept {
    if (!Ready() || registration_attempted_ || request.cbSize != sizeof(request) ||
        request.version != SHROUDWEAVE_NATIVE_SCALAR_REQUEST_VERSION_V1 ||
        request.reserved[0] != 0 || request.reserved[1] != 0 || request.callback == nullptr ||
        request.targetAddress == 0 || request.expectedScalarAddress == 0) {
        return SHROUDWEAVE_NATIVE_SCALAR_INVALID_ARGUMENT;
    }
    registration_attempted_ = true;
    std::uint64_t handle{};
    const std::int32_t result = api_->Register(&request, &handle);
    if (handle != 0) handle_ = handle;
    registration_accepted_ = result == SHROUDWEAVE_NATIVE_SCALAR_OK && handle_ != 0;
    if (result == SHROUDWEAVE_NATIVE_SCALAR_OK && handle_ == 0)
        return SHROUDWEAVE_NATIVE_SCALAR_HOOK_FAILED;
    return result;
}

std::int32_t NativeScalarSdk::RegisterOneExtended(
    const ShroudweaveNativeScalarRequestV2& request) noexcept {
    if (!Ready() || !apiV2_ || registration_attempted_ || request.cbSize!=sizeof(request) ||
        request.version!=2 || request.reserved[0] || request.reserved[1] ||
        !request.callback || !request.targetAddress || !request.expectedScalarAddress)
        return SHROUDWEAVE_NATIVE_SCALAR_INVALID_ARGUMENT;
    registration_attempted_=true;
    std::uint64_t handle{};
    const auto result=apiV2_->Register(&request,&handle);
    if (handle) handle_=handle;
    registration_accepted_=result==SHROUDWEAVE_NATIVE_SCALAR_OK && handle_!=0;
    if (result==SHROUDWEAVE_NATIVE_SCALAR_OK && !handle_)
        return SHROUDWEAVE_NATIVE_SCALAR_HOOK_FAILED;
    return result;
}

std::int32_t NativeScalarSdk::RemoveFinal() noexcept {
    if (!Ready() || handle_ == 0) return SHROUDWEAVE_NATIVE_SCALAR_INVALID_HANDLE;
    const std::int32_t result = api_->Remove(handle_);
    if (result == SHROUDWEAVE_NATIVE_SCALAR_OK) {
        handle_ = 0;
        registration_accepted_ = false;
    }
    // BUSY/RETAINED must leave the handle and both modules resident.
    return result;
}

std::int32_t NativeScalarSdk::RemoveFinalWithRetries(std::uint32_t attempts) noexcept {
    if (attempts == 0) return SHROUDWEAVE_NATIVE_SCALAR_INVALID_ARGUMENT;
    std::int32_t result = SHROUDWEAVE_NATIVE_SCALAR_INVALID_HANDLE;
    for (std::uint32_t attempt = 0; attempt < attempts; ++attempt) {
        result = RemoveFinal();
        if (result != SHROUDWEAVE_NATIVE_SCALAR_BUSY &&
            result != SHROUDWEAVE_NATIVE_SCALAR_RETAINED) {
            return result;
        }
        if (attempt + 1 < attempts) Sleep(25);
    }
    return result;
}

#if defined(SHROUDWEAVE_FLIGHT_TESTING)
void NativeScalarSdk::OverrideApiForTests(
    const ShroudweaveNativeScalarApiV1* api,
    const ShroudweaveNativeScalarApiV2* apiV2) noexcept {
    api_ = api;
    apiV2_ = apiV2;
}
#endif

NativeScalarSdk& ProcessNativeScalarSdk() noexcept {
    static NativeScalarSdk sdk;
    return sdk;
}

} // namespace shroudweave::flight
