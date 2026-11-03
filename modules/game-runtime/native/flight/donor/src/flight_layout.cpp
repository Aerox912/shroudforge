#include "flight_layout.hpp"
#include "flight_stamina.hpp"
#include "flight_stamina_profile.hpp"
#include <bcrypt.h>

#include <algorithm>
#include <array>
#include <cstring>
#include <limits>

namespace shroudweave::flight {
namespace {

constexpr std::uint32_t kMachineAmd64 = 0x8664;
constexpr std::uint16_t kPe32Plus = 0x20B;
constexpr std::uint32_t kSectionExecute = 0x20000000;
constexpr std::uint32_t kSectionRead = 0x40000000;
constexpr std::uint32_t kSectionWrite = 0x80000000;
constexpr std::size_t kMaximumSections = 96;
constexpr std::size_t kMaximumImageSize = 1024ull * 1024ull * 1024ull;
constexpr std::size_t kSectionHeaderSize = 40;
constexpr std::size_t kMinimumOptionalHeaderSize = 112;
constexpr std::size_t kClampSize = 48;
constexpr std::uint32_t kLowerLimitBits = 0x3DB2B8C2;
constexpr std::uint32_t kUpperLimitBits = 0x3FB2B8C2;

constexpr std::array<std::uint8_t, kClampSize> kClamp = {
    0xF3,0x0F,0x10,0x05,0,0,0,0,
    0xF2,0x0F,0x11,0x4C,0x24,0x40,0x77,0x15,
    0x41,0x0F,0x2F,0xC0,0x77,0x0F,
    0xF3,0x0F,0x10,0x35,0,0,0,0,
    0xF3,0x41,0x0F,0x5D,0xF0,0xEB,0x03,0x0F,0x28,0xF0,
    0xF3,0x0F,0x10,0x97,0xD0,0x04,0x00,0x00,
};

struct Section {
    std::array<char, 8> name{};
    std::uint32_t virtual_size{};
    std::uint32_t virtual_address{};
    std::uint32_t raw_size{};
    std::uint32_t raw_address{};
    std::uint32_t characteristics{};

    std::uint64_t mapped_size() const noexcept {
        return virtual_size != 0 ? virtual_size : raw_size;
    }
};

struct Headers {
    std::uint32_t image_size{};
    std::uint32_t header_size{};
    std::uint16_t section_count{};
    std::array<Section, kMaximumSections> sections{};
};

template <typename T>
bool ReadValue(const std::uint8_t* bytes, std::size_t size, std::size_t offset, T& value) noexcept {
    if (bytes == nullptr || offset > size || sizeof(T) > size - offset) {
        return false;
    }
    std::memcpy(&value, bytes + offset, sizeof(T));
    return true;
}

bool RangeFits(std::uint64_t start, std::uint64_t length, std::uint64_t total) noexcept {
    return start <= total && length <= total - start;
}

ScanStatus ParseHeaders(const std::uint8_t* bytes, std::size_t size, bool mapped_image,
                        Headers& headers) noexcept {
    std::uint16_t dos_magic{};
    std::int32_t pe_offset_signed{};
    if (!ReadValue(bytes, size, 0, dos_magic) || dos_magic != 0x5A4D ||
        !ReadValue(bytes, size, 0x3C, pe_offset_signed) || pe_offset_signed < 64) {
        return ScanStatus::malformed_image;
    }

    const auto pe_offset = static_cast<std::size_t>(pe_offset_signed);
    std::uint32_t signature{};
    std::uint16_t machine{}, section_count{}, optional_size{}, optional_magic{};
    if (!ReadValue(bytes, size, pe_offset, signature) || signature != 0x00004550 ||
        !ReadValue(bytes, size, pe_offset + 4, machine) ||
        !ReadValue(bytes, size, pe_offset + 6, section_count) ||
        !ReadValue(bytes, size, pe_offset + 20, optional_size)) {
        return ScanStatus::malformed_image;
    }
    if (machine != kMachineAmd64) {
        return ScanStatus::unsupported_architecture;
    }
    if (section_count == 0 || section_count > kMaximumSections ||
        optional_size < kMinimumOptionalHeaderSize || optional_size > 4096) {
        return ScanStatus::malformed_image;
    }

    const std::size_t optional_offset = pe_offset + 24;
    std::uint32_t image_size{}, header_size{};
    if (!ReadValue(bytes, size, optional_offset, optional_magic) || optional_magic != kPe32Plus) {
        return ScanStatus::unsupported_architecture;
    }
    if (!ReadValue(bytes, size, optional_offset + 56, image_size) ||
        !ReadValue(bytes, size, optional_offset + 60, header_size) ||
        image_size == 0 || image_size > kMaximumImageSize || header_size == 0 ||
        header_size > image_size || (mapped_image && size != image_size) ||
        (!mapped_image && header_size > size)) {
        return ScanStatus::malformed_image;
    }

    const std::uint64_t section_table = static_cast<std::uint64_t>(optional_offset) + optional_size;
    const std::uint64_t section_table_end = section_table +
        static_cast<std::uint64_t>(section_count) * kSectionHeaderSize;
    if (section_table_end > size || section_table_end > header_size) {
        return ScanStatus::malformed_image;
    }

    headers.image_size = image_size;
    headers.header_size = header_size;
    headers.section_count = section_count;
    for (std::size_t i = 0; i < section_count; ++i) {
        const auto offset = static_cast<std::size_t>(section_table) + i * kSectionHeaderSize;
        auto& section = headers.sections[i];
        if (!ReadValue(bytes, size, offset, section.name) ||
            !ReadValue(bytes, size, offset + 8, section.virtual_size) ||
            !ReadValue(bytes, size, offset + 12, section.virtual_address) ||
            !ReadValue(bytes, size, offset + 16, section.raw_size) ||
            !ReadValue(bytes, size, offset + 20, section.raw_address) ||
            !ReadValue(bytes, size, offset + 36, section.characteristics)) {
            return ScanStatus::malformed_image;
        }
        if (!RangeFits(section.virtual_address, section.mapped_size(), image_size)) {
            return ScanStatus::invalid_sections;
        }
        if (!mapped_image && !RangeFits(section.raw_address, section.raw_size, size)) {
            return ScanStatus::malformed_image;
        }
    }
    return ScanStatus::found;
}

bool IsNamed(const Section& section, const char* name) noexcept {
    for (std::size_t i = 0; i < section.name.size(); ++i) {
        const char expected = name[i] == '\0' ? '\0' : name[i];
        if (section.name[i] != expected) {
            return false;
        }
        if (expected == '\0') {
            for (std::size_t j = i + 1; j < section.name.size(); ++j) {
                if (section.name[j] != '\0') {
                    return false;
                }
            }
            return true;
        }
    }
    return name[section.name.size()] == '\0';
}

const Section* FindUniqueSection(const Headers& headers, const char* name, bool& duplicate) noexcept {
    const Section* found = nullptr;
    duplicate = false;
    for (std::size_t i = 0; i < headers.section_count; ++i) {
        if (IsNamed(headers.sections[i], name)) {
            if (found != nullptr) {
                duplicate = true;
                return nullptr;
            }
            found = &headers.sections[i];
        }
    }
    return found;
}

bool RangesOverlap(std::uint64_t first, std::uint64_t first_size,
                   std::uint64_t second, std::uint64_t second_size) noexcept {
    return first < second + second_size && second < first + first_size;
}

bool HasCharacteristics(const Section& section, std::uint32_t required,
                        std::uint32_t forbidden) noexcept {
    return (section.characteristics & required) == required &&
           (section.characteristics & forbidden) == 0;
}

bool Contains(const Section& section, std::uint32_t rva, std::size_t size) noexcept {
    const auto start = static_cast<std::uint64_t>(section.virtual_address);
    const auto extent = section.mapped_size();
    return rva >= start && RangeFits(rva - start, size, extent);
}

bool SignatureMatches(const std::uint8_t* candidate) noexcept {
    for (std::size_t i = 0; i < kClamp.size(); ++i) {
        if ((i >= 4 && i <= 7) || (i >= 26 && i <= 29)) {
            continue; // The two RIP-relative displacements are resolved and validated below.
        }
        if (candidate[i] != kClamp[i]) {
            return false;
        }
    }
    return true;
}

bool ResolveRipTarget(const std::uint8_t* image, std::size_t image_size,
                      std::uint32_t site_rva, std::size_t displacement_offset,
                      std::size_t next_instruction_offset, std::uint32_t& target_rva) noexcept {
    std::int32_t displacement{};
    if (!ReadValue(image, image_size, static_cast<std::size_t>(site_rva) + displacement_offset,
                   displacement)) {
        return false;
    }
    const auto target = static_cast<std::int64_t>(site_rva) +
                        static_cast<std::int64_t>(next_instruction_offset) + displacement;
    if (target < 0 || target > std::numeric_limits<std::uint32_t>::max()) {
        return false;
    }
    target_rva = static_cast<std::uint32_t>(target);
    return true;
}

bool ReadBits(const std::uint8_t* image, std::size_t image_size,
              std::uint32_t rva, std::uint32_t& bits) noexcept {
    return ReadValue(image, image_size, rva, bits);
}

} // namespace

bool StaminaProfileMatches(const std::uint8_t* image, std::size_t size,
                           const std::uint8_t* installed_site) noexcept {
    Headers headers{};
    if (ParseHeaders(image,size,true,headers) != ScanStatus::found) return false;
    bool duplicate_text{}, duplicate_data{};
    const auto* text=FindUniqueSection(headers,".text",duplicate_text);
    const auto* data=FindUniqueSection(headers,".rdata",duplicate_data);
    if (!text || !data || duplicate_text || duplicate_data ||
        !HasCharacteristics(*text,kSectionRead|kSectionExecute,kSectionWrite) ||
        !HasCharacteristics(*data,kSectionRead,kSectionWrite|kSectionExecute) ||
        RangesOverlap(text->virtual_address,text->mapped_size(),data->virtual_address,data->mapped_size()) ||
        !Contains(*text,kStaminaSiteRva,8)) return false;
    if (installed_site && std::memcmp(image+kStaminaSiteRva,installed_site,8)) return false;
    struct Provider { BCRYPT_ALG_HANDLE value{}; ~Provider(){if(value)BCryptCloseAlgorithmProvider(value,0);} } provider;
    if (BCryptOpenAlgorithmProvider(&provider.value,BCRYPT_SHA256_ALGORITHM,nullptr,0)<0) return false;
    DWORD object_size{}, actual{};
    if (BCryptGetProperty(provider.value,BCRYPT_OBJECT_LENGTH,reinterpret_cast<PUCHAR>(&object_size),
                          sizeof(object_size),&actual,0)<0 || actual!=sizeof(object_size) || !object_size || object_size>65536) return false;
    try {
        std::vector<std::uint8_t> object(object_size);
        for (const auto& pin:kStaminaCodePins) {
            if (!Contains(*text,pin.rva,pin.size)) return false;
            struct Hash { BCRYPT_HASH_HANDLE value{}; ~Hash(){if(value)BCryptDestroyHash(value);} } hash;
            if (BCryptCreateHash(provider.value,&hash.value,object.data(),object_size,nullptr,0,0)<0) return false;
            const auto feed=[&](const std::uint8_t* bytes,std::size_t count) {
                return BCryptHashData(hash.value,const_cast<PUCHAR>(bytes),static_cast<ULONG>(count),0)>=0;
            };
            if (installed_site && kStaminaSiteRva>=pin.rva &&
                kStaminaSiteRva+8<=static_cast<std::uint64_t>(pin.rva)+pin.size) {
                constexpr std::uint8_t original[]={0xF3,0x0F,0x58,0x30,0xF3,0x0F,0x11,0x30};
                const auto before=kStaminaSiteRva-pin.rva;
                if (!feed(image+pin.rva,before) || !feed(original,8) ||
                    !feed(image+kStaminaSiteRva+8,pin.size-before-8)) return false;
            } else if (!feed(image+pin.rva,pin.size)) return false;
            std::array<std::uint8_t,32> digest{};
            if (BCryptFinishHash(hash.value,digest.data(),static_cast<ULONG>(digest.size()),0)<0 || digest!=pin.sha256) return false;
        }
        // Enum name/number associations are validated in the actual reflection,
        // including relocation. They are not inferred from an old enum alone.
        struct EnumPin { std::uint32_t record,name,value; const char* text; };
        constexpr EnumPin enums[]={{0x17F4A78,0x1C47A30,7,"Dead"},
            {0x17F4B40,0x1C47CF8,12,"Spawning"},{0x17F4F28,0x1C49048,37,"HangGliding"}};
        std::int32_t pe{}; std::uint64_t preferred{}, common_base{};
        if (!ReadValue(image,size,0x3C,pe) || !ReadValue(image,size,static_cast<std::size_t>(pe)+48,preferred)) return false;
        for (const auto& pin:enums) {
            const auto length=std::strlen(pin.text);
            if (!Contains(*data,pin.record,24) || !Contains(*data,pin.name,length+1)) return false;
            std::uint64_t pointer{}, stored_size{}, value{};
            if (!ReadValue(image,size,pin.record,pointer) || !ReadValue(image,size,pin.record+8,stored_size) ||
                !ReadValue(image,size,pin.record+16,value) || pointer<pin.name || stored_size!=length || value!=pin.value ||
                std::memcmp(image+pin.name,pin.text,length+1)) return false;
            const auto base=pointer-pin.name;
            if (base!=preferred && base!=reinterpret_cast<std::uintptr_t>(image)) return false;
            if (common_base && common_base!=base) return false;
            common_base=base;
        }
        constexpr char local_name[]="LocalPlayerData";
        return Contains(*data,0x150E6D0,sizeof(local_name)) &&
               std::memcmp(image+0x150E6D0,local_name,sizeof(local_name))==0;
    } catch (...) { return false; }
}

ScanResult FindTargetLayout(const std::uint8_t* mapped_image, std::size_t image_size) noexcept {
    if (mapped_image == nullptr || image_size == 0 || image_size > kMaximumImageSize) {
        return {ScanStatus::malformed_image, {}, 0};
    }
    Headers headers{};
    const auto header_status = ParseHeaders(mapped_image, image_size, true, headers);
    if (header_status != ScanStatus::found) {
        return {header_status, {}, 0};
    }

    bool text_duplicate{}, data_duplicate{};
    const auto* text = FindUniqueSection(headers, ".text", text_duplicate);
    const auto* data = FindUniqueSection(headers, ".rdata", data_duplicate);
    if (text == nullptr || data == nullptr || text_duplicate || data_duplicate ||
        !HasCharacteristics(*text, kSectionRead | kSectionExecute, kSectionWrite) ||
        !HasCharacteristics(*data, kSectionRead, kSectionWrite | kSectionExecute) ||
        RangesOverlap(text->virtual_address, text->mapped_size(),
                      data->virtual_address, data->mapped_size())) {
        return {ScanStatus::invalid_sections, {}, 0};
    }

    std::size_t match_count = 0;
    std::uint32_t match_rva{};
    const auto text_size = static_cast<std::size_t>(text->mapped_size());
    if (text_size >= kClampSize) {
        const auto scan_end = static_cast<std::size_t>(text->virtual_address) + text_size - kClampSize;
        for (std::size_t rva = text->virtual_address; rva <= scan_end; ++rva) {
            if (SignatureMatches(mapped_image + rva)) {
                ++match_count;
                match_rva = static_cast<std::uint32_t>(rva);
            }
        }
    }
    if (match_count == 0) {
        return {ScanStatus::no_signature, {}, 0};
    }
    if (match_count != 1) {
        return {ScanStatus::ambiguous_signature, {}, match_count};
    }

    std::uint32_t lower_rva{}, upper_rva{};
    if (!ResolveRipTarget(mapped_image, image_size, match_rva, 4, 8, lower_rva) ||
        !ResolveRipTarget(mapped_image, image_size, match_rva, 26, 30, upper_rva) ||
        lower_rva == upper_rva || (lower_rva & 3u) != 0 || (upper_rva & 3u) != 0 ||
        !Contains(*data, lower_rva, sizeof(std::uint32_t)) ||
        !Contains(*data, upper_rva, sizeof(std::uint32_t))) {
        return {ScanStatus::invalid_operand, {}, match_count};
    }

    std::uint32_t lower_bits{}, upper_bits{};
    if (!ReadBits(mapped_image, image_size, lower_rva, lower_bits) ||
        !ReadBits(mapped_image, image_size, upper_rva, upper_bits) ||
        lower_bits != kLowerLimitBits || upper_bits != kUpperLimitBits) {
        return {ScanStatus::invalid_constants, {}, match_count};
    }
    return {ScanStatus::found, {match_rva, lower_rva, upper_rva}, match_count};
}

bool MapPeFile(const std::uint8_t* file_bytes, std::size_t file_size,
               std::vector<std::uint8_t>& mapped_image) noexcept {
    mapped_image.clear();
    Headers headers{};
    if (ParseHeaders(file_bytes, file_size, false, headers) != ScanStatus::found) {
        return false;
    }
    try {
        std::vector<std::uint8_t> mapped(headers.image_size, 0);
        std::memcpy(mapped.data(), file_bytes, headers.header_size);
        for (std::size_t i = 0; i < headers.section_count; ++i) {
            const auto& section = headers.sections[i];
            if (section.raw_size == 0) {
                continue;
            }
            const auto copy_size = std::min<std::size_t>(section.raw_size,
                headers.image_size - section.virtual_address);
            if (copy_size > 0) {
                std::memcpy(mapped.data() + section.virtual_address,
                            file_bytes + section.raw_address, copy_size);
            }
        }
        mapped_image.swap(mapped);
        return true;
    } catch (...) {
        mapped_image.clear();
        return false;
    }
}

const char* ScanStatusName(ScanStatus status) noexcept {
    switch (status) {
        case ScanStatus::found: return "found";
        case ScanStatus::malformed_image: return "malformed_image";
        case ScanStatus::unsupported_architecture: return "unsupported_architecture";
        case ScanStatus::invalid_sections: return "invalid_sections";
        case ScanStatus::no_signature: return "no_signature";
        case ScanStatus::ambiguous_signature: return "ambiguous_signature";
        case ScanStatus::invalid_operand: return "invalid_operand";
        case ScanStatus::invalid_constants: return "invalid_constants";
    }
    return "unknown";
}

bool ToggleState::Sample(bool key_down, bool focused) noexcept {
    if (!focused) {
        armed_ = false;
        return false;
    }
    if (!key_down) {
        armed_ = true;
        return false;
    }
    if (!armed_) {
        return false;
    }
    armed_ = false;
    enabled_ = !enabled_;
    return true;
}

} // namespace shroudweave::flight
