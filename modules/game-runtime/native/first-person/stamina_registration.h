#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <limits>

namespace FirstPersonControl {

template <typename T>
inline bool MatchesStaminaValue(const std::uint8_t* image, std::size_t size,
    std::size_t rva, T expected) noexcept {
    if (!image || rva > size || sizeof(T) > size - rva) return false;
    T actual{};
    std::memcpy(&actual, image + rva, sizeof(actual));
    return actual == expected;
}

template <std::size_t N>
inline bool MatchesStaminaName(const std::uint8_t* image, std::size_t size,
    std::uintptr_t base, std::size_t tuple, std::size_t nameRva,
    const char (&name)[N]) noexcept {
    return MatchesStaminaValue(image, size, tuple, base + nameRva) &&
        MatchesStaminaValue(image, size, tuple + 8, std::uint64_t{N - 1}) &&
        nameRva <= size && N - 1 <= size - nameRva &&
        std::memcmp(image + nameRva, name, N - 1) == 0;
}

inline bool MatchesStaminaRegistration(const std::uint8_t* image,
    std::size_t size, std::uintptr_t base) noexcept {
    if (!image || !base || base > (std::numeric_limits<std::uintptr_t>::max)() - size) {
        return false;
    }

    // Exact current player_ui registration. Absolute pointers are compared
    // after relocation; names and vector roles come from independent captures.
    constexpr std::size_t descriptorRva = 0x1E26A40;
    const std::array<std::uint64_t, 24> expected = {
        base + 0x1E26DB8, 9, base + 0x2B8350,
        0, 0, base + 0x1E26308, 1,
        base + 0x1E26320, 19,
        base + 0x1E26450, 1,
        base + 0x1E26460, 1,
        0, 0, 0, 0, 0, 0,
        base + 0x1E26470, 26, 0,
        base + 0x1E26610, 2,
    };
    if (descriptorRva > size || sizeof(expected) > size - descriptorRva ||
        std::memcmp(image + descriptorRva, expected.data(), sizeof(expected)) != 0) {
        return false;
    }
    if (!MatchesStaminaName(image, size, base, descriptorRva, 0x1E26DB8, "player_ui") ||
        !MatchesStaminaName(image, size, base, 0x1E26308, 0x1C7D788, "OnSuccess") ||
        !MatchesStaminaName(image, size, base, 0x1E263C0, 0x1D058C0, "NetworkPlayerAttributes") ||
        !MatchesStaminaName(image, size, base, 0x1E26450, 0x1D22100, "UiOffsets") ||
        !MatchesStaminaName(image, size, base, 0x1E26460, 0x1C5C800, "ClientPlayerInput") ||
        !MatchesStaminaName(image, size, base, 0x1E26610, 0x152C718, "G38UiPlayData")) {
        return false;
    }

    // Owning NetworkPlayerAttributes schema: 0x100-byte class, 95 fields.
    // Fields 37/38 are the named uint16 inputs consumed by the snapshot writer.
    return MatchesStaminaName(image, size, base, 0x1A60620, 0x1D058C0,
               "NetworkPlayerAttributes") &&
        MatchesStaminaValue(image, size, 0x1A60660, std::uint32_t{0x100}) &&
        MatchesStaminaValue(image, size, 0x1A60668, std::uint32_t{95}) &&
        MatchesStaminaValue(image, size, 0x1A60670, std::uint64_t{0xB4A3C9140BBC95FB}) &&
        MatchesStaminaValue(image, size, 0x1A60678, base + 0x1A736D0) &&
        MatchesStaminaName(image, size, base, 0x1A73DC0, 0x1CFC330, "stamina") &&
        MatchesStaminaValue(image, size, 0x1A73DD0, base + 0x181AB60) &&
        MatchesStaminaValue(image, size, 0x1A73DD8, std::uint64_t{0x8C}) &&
        MatchesStaminaName(image, size, base, 0x1A73DF0, 0x1CFC380, "staminaMax") &&
        MatchesStaminaValue(image, size, 0x1A73E00, base + 0x181AB60) &&
        MatchesStaminaValue(image, size, 0x1A73E08, std::uint64_t{0x8E});
}

} // namespace FirstPersonControl
