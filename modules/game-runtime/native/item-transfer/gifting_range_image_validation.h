#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>
#include <windows.h>
#include <bcrypt.h>

namespace GiftingRange::ImageValidation {
inline constexpr std::uintptr_t caller_begin_rva = 0x1991c7;
inline constexpr std::uintptr_t caller_end_rva = 0x19924f;
inline constexpr std::uintptr_t call_rva = 0x199239;
inline constexpr std::uintptr_t getter_rva = 0x933e80;
inline constexpr std::size_t getter_size = 9;
inline constexpr std::array<std::uint8_t, 5> stock_call{
    0xe8,0x42,0xac,0x79,0x00};
inline constexpr std::array<std::uint8_t, 9> stock_getter{
    0xf3,0x0f,0x10,0x05,0x8c,0x53,0x2a,0x00,0xc3};

using MemoryReader = bool (*)(const void* context, std::uintptr_t address,
                              void* output, std::size_t size) noexcept;
struct CodeDigests {
    std::array<std::uint8_t, 32> caller{};
    std::array<std::uint8_t, 32> getter{};
};

inline bool read_at_base(std::uintptr_t base, std::uint32_t rva,
                         MemoryReader reader, const void* context,
                         void* output, std::size_t size) noexcept {
    if (!base || !reader || !output || !size ||
        base > std::numeric_limits<std::uintptr_t>::max() - rva)
        return false;
    const auto address = base + rva;
    if (size > std::numeric_limits<std::uintptr_t>::max() - address)
        return false;
    return reader(context, address, output, size);
}

inline bool read_within_image(std::uintptr_t base, std::uint32_t image_size,
                              std::uint32_t rva, MemoryReader reader,
                              const void* context, void* output,
                              std::size_t size) noexcept {
    if (!size || size > image_size || rva > image_size - size)
        return false;
    return read_at_base(base, rva, reader, context, output, size);
}

inline bool is_executable_range(const IMAGE_SECTION_HEADER* sections,
                                unsigned section_count,
                                std::uint32_t image_size,
                                std::uint32_t rva,
                                std::size_t size) noexcept {
    if (!sections || !size || size > image_size || rva > image_size - size)
        return false;
    for (unsigned i = 0; i < section_count; ++i) {
        const auto begin = sections[i].VirtualAddress;
        const auto length = sections[i].Misc.VirtualSize;
        if (!(sections[i].Characteristics & IMAGE_SCN_MEM_EXECUTE) ||
            !length || begin >= image_size || length > image_size - begin)
            continue;
        if (rva >= begin && size <= length && rva - begin <= length - size)
            return true;
    }
    return false;
}

inline bool compute_sha256(const void* bytes, std::size_t size,
                           std::array<std::uint8_t, 32>& digest) noexcept {
    if (!bytes || !size || size > ULONG_MAX) return false;
    BCRYPT_ALG_HANDLE algorithm{};
    BCRYPT_HASH_HANDLE hash{};
    DWORD object_size{}, digest_size{}, received{};
    std::array<UCHAR, 1024> object{};
    bool ok = BCryptOpenAlgorithmProvider(
        &algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) >= 0;
    if (ok) ok = BCryptGetProperty(algorithm, BCRYPT_OBJECT_LENGTH,
        reinterpret_cast<PUCHAR>(&object_size), sizeof(object_size),
        &received, 0) >= 0 && object_size && object_size <= object.size();
    if (ok) ok = BCryptGetProperty(algorithm, BCRYPT_HASH_LENGTH,
        reinterpret_cast<PUCHAR>(&digest_size), sizeof(digest_size),
        &received, 0) >= 0 && digest_size == digest.size();
    if (ok) ok = BCryptCreateHash(algorithm, &hash, object.data(), object_size,
        nullptr, 0, 0) >= 0;
    if (ok) ok = BCryptHashData(hash,
        reinterpret_cast<PUCHAR>(const_cast<void*>(bytes)),
        static_cast<ULONG>(size), 0) >= 0;
    if (ok) ok = BCryptFinishHash(hash, digest.data(), digest_size, 0) >= 0;
    if (hash) BCryptDestroyHash(hash);
    if (algorithm) BCryptCloseAlgorithmProvider(algorithm, 0);
    return ok;
}

inline bool sha256_matches(const void* bytes, std::size_t size,
                           const std::array<std::uint8_t, 32>& expected) noexcept {
    std::array<std::uint8_t, 32> actual{};
    return compute_sha256(bytes, size, actual) && actual == expected;
}

inline bool validate_loaded_image(std::uintptr_t base, MemoryReader reader,
                                  const void* context,
                                  const CodeDigests& expected,
                                  std::uint32_t& validated_image_size) noexcept {
    validated_image_size = 0;
    if (!base || !reader) return false;

    IMAGE_DOS_HEADER dos{};
    if (!read_at_base(base, 0, reader, context, &dos, sizeof(dos)) ||
        dos.e_magic != IMAGE_DOS_SIGNATURE ||
        dos.e_lfanew < static_cast<LONG>(sizeof(dos)))
        return false;

    const auto nt_rva = static_cast<std::uint32_t>(dos.e_lfanew);
    IMAGE_NT_HEADERS64 nt{};
    if (!read_at_base(base, nt_rva, reader, context, &nt, sizeof(nt)) ||
        nt.Signature != IMAGE_NT_SIGNATURE ||
        nt.FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 ||
        nt.OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC ||
        nt.FileHeader.SizeOfOptionalHeader < sizeof(IMAGE_OPTIONAL_HEADER64) ||
        !nt.FileHeader.NumberOfSections ||
        nt.FileHeader.NumberOfSections > 96)
        return false;

    // The bound comes from the currently mapped PE header. File timestamp,
    // whole-file hash, and a profile's expected image size are not admission
    // checks for this code-specific adapter.
    const auto image_size = nt.OptionalHeader.SizeOfImage;
    if (!image_size ||
        base > std::numeric_limits<std::uintptr_t>::max() - image_size ||
        nt_rva > image_size || sizeof(nt) > image_size - nt_rva)
        return false;

    const auto section_rva = static_cast<std::uint64_t>(nt_rva) +
        offsetof(IMAGE_NT_HEADERS64, OptionalHeader) +
        nt.FileHeader.SizeOfOptionalHeader;
    const auto section_bytes =
        static_cast<std::uint64_t>(nt.FileHeader.NumberOfSections) *
        sizeof(IMAGE_SECTION_HEADER);
    if (section_rva > image_size ||
        section_bytes > image_size - section_rva)
        return false;
    std::array<IMAGE_SECTION_HEADER, 96> sections{};
    if (!read_within_image(base, image_size,
            static_cast<std::uint32_t>(section_rva), reader, context,
            sections.data(), static_cast<std::size_t>(section_bytes)))
        return false;
    for (unsigned i = 0; i < nt.FileHeader.NumberOfSections; ++i) {
        const auto begin = sections[i].VirtualAddress;
        const auto length = sections[i].Misc.VirtualSize;
        if (begin >= image_size ||
            (length && length > image_size - begin))
            return false;
    }
    if (!is_executable_range(sections.data(), nt.FileHeader.NumberOfSections,
            image_size, static_cast<std::uint32_t>(caller_begin_rva),
            caller_end_rva - caller_begin_rva) ||
        !is_executable_range(sections.data(), nt.FileHeader.NumberOfSections,
            image_size, static_cast<std::uint32_t>(getter_rva), getter_size))
        return false;

    if (nt.OptionalHeader.NumberOfRvaAndSizes <=
        IMAGE_DIRECTORY_ENTRY_EXCEPTION)
        return false;
    const auto exception =
        nt.OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION];
    if (!exception.VirtualAddress ||
        exception.Size < sizeof(RUNTIME_FUNCTION) ||
        exception.Size % sizeof(RUNTIME_FUNCTION) ||
        exception.VirtualAddress >= image_size ||
        exception.Size > image_size - exception.VirtualAddress)
        return false;
    const auto count = exception.Size / sizeof(RUNTIME_FUNCTION);
    std::size_t low{}, high = count;
    bool caller_entry{};
    while (low < high) {
        const auto middle = low + (high - low) / 2;
        RUNTIME_FUNCTION entry{};
        const auto rva = exception.VirtualAddress +
            static_cast<std::uint32_t>(middle * sizeof(entry));
        if (!read_within_image(base, image_size, rva, reader, context,
                &entry, sizeof(entry)) ||
            entry.BeginAddress >= entry.EndAddress ||
            entry.EndAddress > image_size ||
            !entry.UnwindData || entry.UnwindData >= image_size)
            return false;
        if (entry.BeginAddress == caller_begin_rva &&
            entry.EndAddress == caller_end_rva) {
            caller_entry = true;
            break;
        }
        if (entry.BeginAddress < caller_begin_rva) low = middle + 1;
        else high = middle;
    }
    if (!caller_entry) return false;

    std::array<std::uint8_t, caller_end_rva - caller_begin_rva> caller{};
    std::array<std::uint8_t, getter_size> getter{};
    std::array<std::uint8_t, stock_call.size()> call{};
    if (!read_within_image(base, image_size,
            static_cast<std::uint32_t>(caller_begin_rva), reader, context,
            caller.data(), caller.size()) ||
        !read_within_image(base, image_size,
            static_cast<std::uint32_t>(getter_rva), reader, context,
            getter.data(), getter.size()) ||
        !read_within_image(base, image_size,
            static_cast<std::uint32_t>(call_rva), reader, context,
            call.data(), call.size()) ||
        !sha256_matches(caller.data(), caller.size(), expected.caller) ||
        !sha256_matches(getter.data(), getter.size(), expected.getter) ||
        call != stock_call || getter != stock_getter)
        return false;

    std::int32_t displacement{};
    std::memcpy(&displacement, call.data() + 1, sizeof(displacement));
    const auto actual_target =
        static_cast<std::int64_t>(call_rva + call.size()) + displacement;
    if (actual_target != static_cast<std::int64_t>(getter_rva))
        return false;

    validated_image_size = image_size;
    return true;
}
}
