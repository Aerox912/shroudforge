#include "first-person/first_person_control.h"
#include "first-person/first_person_profile.h"
#include "first-person/stamina_anchor_stub.h"

#include <windows.h>

#include <array>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <limits>
#include <string_view>
#include <utility>
#include <vector>

namespace {

unsigned failures{};

void check(bool condition, const char* message) {
    if (condition) return;
    ++failures;
    std::cerr << "FAIL: " << message << '\n';
}

std::vector<std::uint8_t> supported_image() {
    using namespace FirstPersonControl;
    std::vector<std::uint8_t> image(ClientImageSize, 0);

    IMAGE_DOS_HEADER dos{};
    dos.e_magic = IMAGE_DOS_SIGNATURE;
    dos.e_lfanew = 0x100;
    std::memcpy(image.data(), &dos, sizeof(dos));

    IMAGE_NT_HEADERS64 nt{};
    nt.Signature = IMAGE_NT_SIGNATURE;
    nt.FileHeader.Machine = IMAGE_FILE_MACHINE_AMD64;
    nt.FileHeader.TimeDateStamp = ClientTimestamp;
    nt.OptionalHeader.Magic = IMAGE_NT_OPTIONAL_HDR64_MAGIC;
    nt.OptionalHeader.SizeOfImage = static_cast<DWORD>(ClientImageSize);
    std::memcpy(image.data() + dos.e_lfanew, &nt, sizeof(nt));

    const auto base = reinterpret_cast<std::uintptr_t>(image.data());
    constexpr char system_name[] = "player_camera_transform";
    const std::array<std::uintptr_t, 3> descriptor = {
        base + CameraSystemName,
        sizeof(system_name) - 1,
        base + CameraSystemFunction,
    };
    std::memcpy(image.data() + CameraSystemDescriptor,
        descriptor.data(), sizeof(descriptor));
    std::memcpy(image.data() + CameraSystemName, system_name, sizeof(system_name));
    std::memcpy(image.data() + CameraSystemFunction,
        CameraSystemEntry.data(), CameraSystemEntry.size());
    std::memcpy(image.data() + CameraTransformCall,
        OriginalCameraCall.data(), OriginalCameraCall.size());
    std::memcpy(image.data() + CameraTransformFunction,
        CameraTransformEntry.data(), CameraTransformEntry.size());
    std::memcpy(image.data() + 0xE82061,
        CameraDistanceRead.data(), CameraDistanceRead.size());
    std::memcpy(image.data() + 0xE81FBD,
        CameraShoulderRead.data(), CameraShoulderRead.size());
    std::memcpy(image.data() + 0x2B2C75,
        CameraArgument.data(), CameraArgument.size());
    std::memcpy(image.data() + 0x2B2C7C,
        CameraOutputArgument.data(), CameraOutputArgument.size());

    for (const auto& guard : GaugeGuards()) {
        std::memcpy(image.data() + guard.rva, guard.bytes.data(), guard.bytes.size());
    }
    return image;
}

void test_profile_admission() {
    using namespace FirstPersonControl;
    auto image = supported_image();
    const auto base = reinterpret_cast<std::uintptr_t>(image.data());
    auto admitted = [&] {
        return MatchesSupportedImage(image.data(), image.size(), base,
            ClientSha256, OriginalCameraCall, OriginalStaminaCall,
            CameraSystemEntry);
    };

    check(GaugeGuards().size() == 14, "the operation admission uses all 14 stamina guard spans");
    check(admitted(), "exact client identity and camera/stamina preimages are admitted");

    check(!MatchesSupportedImage(image.data(), image.size(), base,
        "0000000000000000000000000000000000000000000000000000000000000000",
        OriginalCameraCall, OriginalStaminaCall, CameraSystemEntry),
        "a wrong image digest is refused");

    const std::array<std::uint8_t, 5> unexpected_call = {
        0xE8, 0x01, 0x02, 0x03, 0x04,
    };
    image[CameraTransformCall] = unexpected_call[0];
    std::memcpy(image.data() + CameraTransformCall + 1,
        unexpected_call.data() + 1, unexpected_call.size() - 1);
    check(!admitted(), "an unexpected camera call-site patch is refused");
    std::memcpy(image.data() + CameraTransformCall,
        OriginalCameraCall.data(), OriginalCameraCall.size());

    // Minimap owns only the camera-system entry at 0x2B2B60. Its entry
    // replacement is admitted only when the loader ownership query supplies
    // this exact current replacement to the profile matcher.
    std::array<std::uint8_t, CameraSystemEntry.size()> minimap_entry_patch{};
    minimap_entry_patch.fill(0x90);
    minimap_entry_patch[0] = 0xE9;
    minimap_entry_patch[1] = 0x31;
    minimap_entry_patch[2] = 0x42;
    minimap_entry_patch[3] = 0x53;
    minimap_entry_patch[4] = 0x64;
    std::memcpy(image.data() + CameraSystemFunction,
        minimap_entry_patch.data(), minimap_entry_patch.size());
    check(!admitted(), "an entry patch without loader ownership evidence is refused");
    check(MatchesSupportedImage(image.data(), image.size(), base,
        ClientSha256, OriginalCameraCall, OriginalStaminaCall,
        minimap_entry_patch),
        "a caller-supplied verified Minimap replacement preserves interior-call admission");
    image[CameraTransformFunction] ^= 1;
    check(!MatchesSupportedImage(image.data(), image.size(), base,
        ClientSha256, OriginalCameraCall, OriginalStaminaCall,
        minimap_entry_patch),
        "an unexpected transform-function mutation is refused with Minimap installed");
    image[CameraTransformFunction] ^= 1;
    std::memcpy(image.data() + CameraSystemFunction,
        CameraSystemEntry.data(), CameraSystemEntry.size());

    image = supported_image();
    const auto refreshed_base = reinterpret_cast<std::uintptr_t>(image.data());
    for (const auto& guard : GaugeGuards()) {
        for (std::size_t index = 0; index < guard.bytes.size(); ++index) {
            const auto rva = guard.rva + index;
            image[rva] ^= 0x01;
            check(!MatchesSupportedImage(image.data(), image.size(), refreshed_base,
                ClientSha256, OriginalCameraCall, OriginalStaminaCall,
                CameraSystemEntry),
                "every individual stamina guard byte rejects a modified image");
            image[rva] ^= 0x01;
        }
    }

    const std::array<std::pair<std::size_t, std::size_t>, 9> camera_ranges = {{
        {CameraSystemFunction, CameraSystemEntry.size()},
        {CameraTransformCall, OriginalCameraCall.size()},
        {CameraSystemDescriptor, sizeof(std::uintptr_t) * 3},
        {CameraTransformFunction, CameraTransformEntry.size()},
        {0xE82061, CameraDistanceRead.size()},
        {0xE81FBD, CameraShoulderRead.size()},
        {0x2B2C75, CameraArgument.size()},
        {0x2B2C7C, CameraOutputArgument.size()},
        {CameraSystemName, sizeof("player_camera_transform")},
    }};
    for (const auto& [begin, length] : camera_ranges) {
        for (std::size_t index = 0; index < length; ++index) {
            image[begin + index] ^= 0x01;
            check(!MatchesSupportedImage(image.data(), image.size(), refreshed_base,
                ClientSha256, OriginalCameraCall, OriginalStaminaCall,
                CameraSystemEntry),
                "every individual camera guard byte rejects a modified image");
            image[begin + index] ^= 0x01;
        }
    }
}

void test_owner_lifecycle() {
    using namespace FirstPersonControl;
    OwnerState owner;

    check(!owner.Prepared(), "camera and gauge hooks start unprepared");
    check(owner.Configure(10, -0.15) == Status::Failed,
        "configuration cannot install or claim an owner before held-entry preparation");
    check(owner.SetMode(10, true) == Status::Failed,
        "mode cannot be enabled before held-entry preparation");
    check(owner.SetMode(10, false) == Status::Ready && !owner.Enabled(),
        "disable remains safe and idempotent before startup preparation");
    check(owner.ReleaseOwner(10) == Status::Ready,
        "release remains safe and idempotent before startup preparation");
    check(owner.MarkPrepared() && owner.Prepared(),
        "validated held-entry preparation marks hooks ready");
    check(owner.SetMode(10, true) == Status::OwnerConflict,
        "prepared mode still requires configuration to claim an owner");

    check(owner.Configure(0, -0.15) == Status::InvalidArgument,
        "zero owner tokens are refused");
    check(owner.Configure(10, std::numeric_limits<double>::quiet_NaN()) == Status::InvalidArgument,
        "NaN camera distances are refused");
    check(owner.Configure(10, std::numeric_limits<double>::infinity()) == Status::InvalidArgument,
        "infinite camera distances are refused");
    check(owner.Configure(10, -0.500001) == Status::InvalidArgument,
        "camera distances below the admitted range are refused");
    check(owner.Configure(10, 0.500001) == Status::InvalidArgument,
        "camera distances above the admitted range are refused");

    for (const double distance : {-0.5, -0.15, 0.0, 0.5}) {
        check(owner.Configure(10, distance) == Status::Ready,
            "finite distances at or inside the supported range are accepted");
        check(owner.CameraDistance() == distance,
            "configuration retains the last valid camera distance");
    }
    check(owner.Configure(20, 0.0) == Status::OwnerConflict,
        "another runtime owner cannot replace the active owner");
    check(owner.SetMode(20, true) == Status::OwnerConflict,
        "another runtime owner cannot enable the mode");
    check(owner.SetMode(10, true) == Status::Ready && owner.Enabled(),
        "the configured owner can enable the shared mode");
    check(owner.ReleaseOwner(20) == Status::OwnerConflict && owner.Enabled(),
        "a stale owner cannot release a newer active owner");
    check(owner.SetMode(10, false) == Status::Ready && !owner.Enabled(),
        "disabling restores original camera and gauge behavior");
    check(owner.SetMode(10, true) == Status::Ready && owner.Enabled(),
        "the same owner can re-enable after disabling");
    check(owner.ReleaseOwner(10) == Status::Ready && !owner.Enabled() && owner.OwnerToken() == 0,
        "owner release disables both controls and clears the generation");
    check(owner.CameraDistance() == DefaultCameraDistance,
        "owner release restores the configured default distance");
    check(owner.ReleaseOwner(10) == Status::Ready,
        "owner release is idempotent after the owner has been cleared");
    owner.Configure(30, 0.25);
    owner.SetMode(30, true);
    owner.Shutdown();
    check(owner.OwnerToken() == 0 && !owner.Enabled() &&
        owner.CameraDistance() == DefaultCameraDistance,
        "process shutdown disables and clears the current owner");
    check(owner.Prepared(),
        "shutdown retains process-resident disabled hooks for safe forwarding");
    check(owner.ShutdownRequested() && owner.Configure(31, 0.0) == Status::Failed &&
        owner.SetMode(31, true) == Status::Failed,
        "process shutdown refuses new owners and mode activation while retaining stubs");
}

void test_gauge_stub_layout() {
    using namespace FirstPersonControl;
    constexpr std::uintptr_t allocation = 0x100000000ULL;
    GaugeCode code;
    check(BuildGaugeCode(allocation + 0x3000, allocation + SharedStateOffset,
            allocation, code), "the stamina anchor stub builds with a shared mode flag");
    check(code.codeSize > 0 && code.codeSize < CodePageSize,
        "the unwindable gauge instructions fit their dedicated executable page");
    check(code.bytes.size() > CodePageSize && code.bytes.size() < AllocationSize - GaugeCodeOffset,
        "gauge constants and unwind metadata fit the separate writable pages");
    check(code.function.BeginAddress == 0 && code.function.EndAddress == code.codeSize,
        "the function table covers only the executable gauge instructions");
    check(code.function.UnwindData >= CodePageSize + GaugeDataOffsetFromState,
        "the unwind record remains outside the executable code page");
    check(code.tail < code.bytes.size() && code.bytes.size() - code.tail >= sizeof(std::uintptr_t),
        "the native setter tail target has a resident pointer slot");
}

} // namespace

int main() {
    test_profile_admission();
    test_owner_lifecycle();
    test_gauge_stub_layout();
    if (failures) {
        std::cerr << failures << " first-person checks failed\n";
        return 1;
    }
    std::cout << "First-person profile, ownership, and stub checks passed\n";
    return 0;
}
