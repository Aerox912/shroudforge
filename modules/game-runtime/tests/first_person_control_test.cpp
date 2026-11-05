#include "first-person/first_person_control.h"
#include "first-person/first_person_profile.h"
#include "first-person/stamina_anchor_stub.h"

#include <windows.h>
#include "json/single_include/nlohmann/json.hpp"
#include <fstream>
#include <stdexcept>
#include <string>

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

unsigned checks{};

void check(bool condition, const char* message) {
    ++checks;
    if (condition) return;
    ++failures;
    std::cerr << "FAIL: " << message << '\n';
}

// The fixture is derived from SHA-pinned, independently captured JSON packets.
// It never fills bytes from production guard arrays. This is static admission
// evidence only, not a loaded game, provider, or gameplay fixture.
using Json = nlohmann::json;
constexpr char CurrentSha[] =
    "5d9ef2f140b8ed49e30dbee655841b89d08baec20f799a1817e171a18a850c77";

// Standalone current facts from the independent current camera/stamina packets.
// Keep the source-packet hashes when regenerating this synthetic/static fixture.
constexpr char EmbeddedCurrentFacts[] = R"FP5D({"sourceSha256":"5d9ef2f140b8ed49e30dbee655841b89d08baec20f799a1817e171a18a850c77","timestamp":1793791052,"imageSize":44298240,"guardRanges":[[12365776,96],[12508356,7],[12508492,21],[12508533,68],[12367401,13],[12369173,51],[12369224,96],[18674176,5],[10794688,9],[18528338,37],[18682736,238],[18431680,480],[18431568,100],[18573264,8],[12366154,43],[12508276,325],[2854258,40],[2851888,72],[2852232,7],[10255232,103],[10255343,21],[10255409,23],[10255553,33],[10233728,260]],"metadataRanges":[[31611520,192],[31612408,9],[31609672,16],[29869960,9],[31609856,16],[30427328,23],[31610000,16],[30544128,9],[31610016,16],[29734912,17],[31610448,16],[22198040,13],[27653664,16],[27653728,4],[27653736,4],[27653744,8],[27653752,8],[27733440,32],[30389040,7],[27733488,32],[30389120,10]],"spans":[{"rva":12365776,"bytes":"488bc4555356574154415541564157488da868fbffff4881ec58050000488b15b444c7014c8be90f2970a84533e40f297898440f294088440f298878ffffff440f299068ffffff440f299858ffffff440f29a838ffffff440f29b028ffffff48","relocate":false},{"rva":12508356,"bytes":"66c74424680002","relocate":false},{"rva":12508492,"bytes":"0f109128dc00000f109938dc00000f10a148dc0000","relocate":false},{"rva":12508533,"bytes":"0f28442460488d4d000f284c24700f2945000f2845d00f294d10f20f104de00f294570f20f118d800000000f2955200f295d300f2965400f297d500f297560e817d2fdff","relocate":false},{"rva":12367401,"bytes":"410fb65509498bcfe8ca251d00","relocate":false},{"rva":12369173,"bytes":"488b8dc80000000f28ce0fc6ce550f28c60f28fe0f14c10fc6feaa440f28fe66480f7ec20f297c2440440fc6feffe8b8346000","relocate":false},{"rva":12369224,"bytes":"488b8dc80000000f28c7410f14c766480f7ec2e8f0306000488d05112a1c0148c745a808000000488945a0488d55a048b88639a4c8ab4ec7c1488d4d18488945b0e8820de7ff488d4d18e879dde7ff488b8dc8000000e82daa5e000f10180f11","relocate":false},{"rva":18674176,"bytes":"48895170c3","relocate":false},{"rva":10794688,"bytes":"488b41104883c020c3","relocate":false},{"rva":18528338,"bytes":"f20f104370488bcbf20f108be4000000f20f118314010000f20f118b1c010000e8795a0200","relocate":false},{"rva":18682736,"bytes":"f30f1087a80000000f57d2f30f5887b0000000f30f108f1c0100000f57f6450fb606f30f5cc8f30f1087ac000000f30f5887b4000000f30f5fd1f30f108f20010000f30f5cc8f30f5ff14180e001740af30f108fcc000000eb030f57c9f30f1087a80000004533c9f30f58871401000048895c2420f30f58c10f28cae8cf2afcff410fb606d0e8240141f60602740af30f108fd0000000eb030f57c9f30f1087ac000000440fb6c0f30f58871801000048895c2420f30f58c10f28cee87f2bfcfff30f104b74488bcbf30f104370f30f588314010000f30f588b18010000f30f118314010000f30f118b18010000","relocate":false},{"rva":18431680,"bytes":"488b4c24280f57e40f28d8f30f1091f40000000f2fd4764e4584c97549f30f1081a00000000f28e9f30f1091dc000000f30f588198000000f30f5ce84584c07408f30f10a1e4000000f30f5fe50f2fd4770cf30f1091ec000000f30f5dd4f30f1191e40000000fb6516c85d2746583ea01746083ea01743283fa017566f30f1081e4000000f30f5881a00000008b81e400000089811c010000f30f5cc8f30f58cbf30f118914010000c3f30f5c89e40000008b81e400000089811c010000f30f590d0ecd2300f30f58cbf30f118914010000c3f30f589998000000f30f1199140100008b81e400000089811c010000c3488b4c24280f57e40f28d8f30f1091f80000000f2fd4764e4584c97549f30f1081a40000000f28e9f30f1091e0000000f30f58819c000000f30f5ce84584c07408f30f10a1e8000000f30f5fe50f2fd4770cf30f1091f0000000f30f5dd4f30f1191e80000000fb6516d85d2746583ea01746083ea01743283fa017566f30f1081e8000000f30f5881a40000008b81e8000000898120010000f30f5cc8f30f58cbf30f118918010000c3f30f5c89e80000008b81e8000000898120010000f30f590d1ecc2300f30f58cbf30f118918010000c3f30f58999c000000f30f1199180100008b81e8000000898120010000c3","relocate":false},{"rva":18431568,"bytes":"40534883ec30488b59180f2974242066480f6ef20f28ce0f28c60fc6c955f30f588918010000f30f588114010000f30f118918010000f30f1181140100004885db741666480f7ef2488bcbe8b0ffffff488b5b104885db75ea0f287424204883c4305bc3","relocate":false},{"rva":18573264,"bytes":"488d8114010000c3","relocate":false},{"rva":12366154,"bytes":"488b8df00000004c8d0dc80bf0ff48bab2e09ec1bc3c4fc8c74424200100000041b848020000e86b555d00","relocate":false},{"rva":12508276,"bytes":"488b052d18c501b9ce020000488bb0d00000004c8bbea83a0300488d867e3903004d85ff480f44c138180f8415010000410fb7cce8635d170084c00f8404010000488b0dec17c5010f57f648895c246066c74424680002c644247d01488b81d000000048895dd0488b004885c07505488bc3eb04488b4038f20f1080840500008b808c05000089442474488b81c8000000f20f1144246c0f10b8d02c0100e839791500894424784d85ff488b058317c501885c247cc745d80000003fc745dc9a99993e488b88c8000000488d867f390300c745e00ad7a33c0f109128dc00000f109938dc00000f10a148dc0000b9cf020000480f44c138187407c745d4000096420f28442460488d4d000f284c24700f2945000f2845d00f294d10f20f104de00f294570f20f118d800000000f2955200f295d300f2965400f297d500f297560e817d2fdff","relocate":false},{"rva":2854258,"bytes":"488b85d00000000fb7888c000000898e84050000488b85d00000000fb7888e000000898e88050000","relocate":false},{"rva":2851888,"bytes":"48894c24085557488dac24382afdffb8c8d60200e8f7210901482be041b8c8030000488d5570488bf9e822f7700041b8c8030000488d5570488bcfe810a3700084c00f8460690000","relocate":false},{"rva":2852232,"bytes":"488bb520020000","relocate":false},{"rva":10255232,"bytes":"48895c241055565741544155415641574883ec604c8b09488bd9488bfa410fb681b00400004c8d04c500000000410fb781b8040000498d500848899424a00000004c8d3cc500000000410fb781ba040000488d3440498b81c000000048c1e6040fb68800040000","relocate":false},{"rva":10255343,"bytes":"488d0ccd000000004c8d2cc50000000048894c2420","relocate":false},{"rva":10255409,"bytes":"498d041748898424b80000004803ed4803c64889442428","relocate":false},{"rva":10255553,"bytes":"4c8b4424204d85c07417488b13488b4c24284803cf488b92c0000000e8be29a000","relocate":false},{"rva":10233728,"bytes":"40574883ec204c8b01488bfa8b41084c8bc9493b8038080000750832c04883c4205fc3410fb680b0040000498d887004000048895c24384533db410fb798ba04000048896c244048897424484c89642418450fb7a0b80400004c896c2410418bd44c897424084c8d34c5080000004c893c24895c24304b8d2ce684c0743b488bdf482bd94c8b014d85c0740a418b41084d8b14c0eb034533d24c89540b0849ffc34d8b014883c108410fb680b00400004c3bd872cf8b5c243041837908004d8da8c8010000498bb0c0010000450fb6b8bb0000000f85210100004585e47425498bcd4d8d043e66660f1f840000000000488b01488d49104989004d8d40084883ea0175ec","relocate":false},{"rva":31611520,"bytes":"f85de24101000000090000000000000030842b4001000000000000000000000000000000000000004853e2410100000001000000000000006053e2410100000013000000000000009054e241010000000100000000000000a054e241010000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000b054e241010000001a0000000000000000000000000000005056e241010000000200000000000000","relocate":true},{"rva":31612408,"bytes":"706c617965725f7569","relocate":false},{"rva":31609672,"bytes":"88c7c741010000000900000000000000","relocate":true},{"rva":29869960,"bytes":"4f6e53756363657373","relocate":false},{"rva":31609856,"bytes":"c048d041010000001700000000000000","relocate":true},{"rva":30427328,"bytes":"4e6574776f726b506c6179657241747472696275746573","relocate":false},{"rva":31610000,"bytes":"0011d241010000000900000000000000","relocate":true},{"rva":30544128,"bytes":"55694f666673657473","relocate":false},{"rva":31610016,"bytes":"00b8c541010000001100000000000000","relocate":true},{"rva":29734912,"bytes":"436c69656e74506c61796572496e707574","relocate":false},{"rva":31610448,"bytes":"18b75241010000000d00000000000000","relocate":true},{"rva":22198040,"bytes":"4733385569506c617944617461","relocate":false},{"rva":27653664,"bytes":"c048d041010000001700000000000000","relocate":true},{"rva":27653728,"bytes":"00010000","relocate":false},{"rva":27653736,"bytes":"5f000000","relocate":false},{"rva":27653744,"bytes":"fb95bc0b14c9a3b4","relocate":false},{"rva":27653752,"bytes":"d026a74101000000","relocate":true},{"rva":27733440,"bytes":"30b3cf41010000000700000000000000609b8141010000008c00000000000000","relocate":true},{"rva":30389040,"bytes":"7374616d696e61","relocate":false},{"rva":27733488,"bytes":"80b3cf41010000000a00000000000000609b8141010000008e00000000000000","relocate":true},{"rva":30389120,"bytes":"7374616d696e614d6178","relocate":false},{"rva":2849024,"bytes":"405557488dac24b8feffff4881ec48020000","relocate":false},{"rva":2849330,"bytes":"e86914be00","relocate":false},{"rva":15306400,"bytes":"488bc4488958084889781055488d68a14881ecf0000000","relocate":false},{"rva":15306929,"bytes":"f30f10a3a4140000","relocate":false},{"rva":15306765,"bytes":"f20f10a38c140000","relocate":false},{"rva":2849301,"bytes":"488b5588","relocate":false},{"rva":2849308,"bytes":"488b4d90","relocate":false},{"rva":31608384,"bytes":"b05de24101000000170000000000000000792b4001000000","relocate":true},{"rva":31612336,"bytes":"706c617965725f63616d6572615f7472616e73666f726d00","relocate":false}],"staminaCallRva":12369219,"setterRva":18674176,"provenance":[{"path":"E:\\Build\\sf178-current5d-stamina-source-20261105-r1\\STAMINA-FACTS.json","bytes":23412,"sha256":"7e4df1baa212d6ef5fb85a7ce2f20d34a74e6e612048fd6c17be8ed95ef4b0c3"},{"path":"E:\\Build\\sf178-current5d-map-anchor-helper-20261105-r1\\CAMERA-CAPTURE-FACTS.json","bytes":3933,"sha256":"a02eac854710ce8996ab61d084ac168df252644e933a7f531c7c7f780264eff3"}],"cameraRanges":[[2849024,18],[2849330,5],[15306400,23],[15306929,8],[15306765,8],[2849301,4],[2849308,4],[31608384,24],[31612336,24]],"cameraCallRva":2849330,"cameraSystemRva":2849024,"transformRva":15306400,"cameraEntryHex":"405557488dac24b8feffff4881ec48020000"})FP5D";

std::vector<std::uint8_t> hex_bytes(const std::string& hex) {
    if (hex.size() % 2) throw std::runtime_error("odd captured hex length");
    std::vector<std::uint8_t> bytes;
    for (std::size_t i = 0; i < hex.size(); i += 2) {
        bytes.push_back(static_cast<std::uint8_t>(std::stoul(hex.substr(i, 2), nullptr, 16)));
    }
    return bytes;
}

std::vector<std::uint8_t> captured_image(const Json& facts, std::uintptr_t base) {
    if (facts.at("sourceSha256") != CurrentSha ||
        facts.at("timestamp") != 0x6aeb144c || facts.at("imageSize") != 0x2a3f000) {
        throw std::runtime_error("wrong independently captured image identity");
    }
    std::vector<std::uint8_t> image(0x2a3f000, 0);
    IMAGE_DOS_HEADER dos{};
    dos.e_magic = IMAGE_DOS_SIGNATURE;
    dos.e_lfanew = 0x100;
    std::memcpy(image.data(), &dos, sizeof(dos));
    IMAGE_NT_HEADERS64 nt{};
    nt.Signature = IMAGE_NT_SIGNATURE;
    nt.FileHeader.Machine = IMAGE_FILE_MACHINE_AMD64;
    nt.FileHeader.TimeDateStamp = 0x6aeb144c;
    nt.OptionalHeader.Magic = IMAGE_NT_OPTIONAL_HDR64_MAGIC;
    nt.OptionalHeader.SizeOfImage = 0x2a3f000;
    std::memcpy(image.data() + 0x100, &nt, sizeof(nt));
    std::vector<bool> filled(image.size(), false);
    for (const auto& span : facts.at("spans")) {
        const auto rva = span.at("rva").get<std::size_t>();
        auto bytes = hex_bytes(span.at("bytes").get<std::string>());
        if (rva > image.size() || bytes.size() > image.size() - rva) {
            throw std::runtime_error("captured span exceeds image");
        }
        if (span.at("relocate").get<bool>()) {
            for (std::size_t at = 0; at + 8 <= bytes.size(); at += 8) {
                std::uint64_t value{};
                std::memcpy(&value, bytes.data() + at, 8);
                if (value >= 0x140000000ULL && value < 0x142a3f000ULL) {
                    value = base + value - 0x140000000ULL;
                    std::memcpy(bytes.data() + at, &value, 8);
                }
            }
        }
        for (std::size_t at = 0; at < bytes.size(); ++at) {
            if (filled[rva + at] && image[rva + at] != bytes[at]) {
                throw std::runtime_error("independent capture spans disagree");
            }
            image[rva + at] = bytes[at];
            filled[rva + at] = true;
        }
    }
    return image;
}

void test_profile_admission(const Json& facts) {
    using namespace FirstPersonControl;
    // Deliberately use a simulated relocated base different from the buffer.
    constexpr std::uintptr_t base = 0x180000000ULL;
    auto image = captured_image(facts, base);
    auto admitted = [&] {
        return MatchesSupportedImage(image.data(), image.size(), base,
            CurrentSha, OriginalCameraCall, OriginalStaminaCall, CameraSystemEntry);
    };
    check(MatchesClientPeIdentity(image.data(), image.size()),
        "captured current 5d9 PE identity is admitted");
    check(admitted(), "independently captured current 5d9 static profile is admitted");
    check(GaugeGuards().size() == 24, "all fourteen historical, five FP semantic and five iterator spans are used");
    check(!MatchesSupportedImage(image.data(), image.size(), base,
        "d64966e23b1a458ee6f147bcbceac0b577fa7e39a212f371287d8cc119817882",
        OriginalCameraCall, OriginalStaminaCall, CameraSystemEntry), "old d649 digest is refused");
    check(!MatchesSupportedImage(image.data(), image.size(), base,
        "0403981fd2e8ad18bd8c5698dff03c8d505b5a46677832c496db9c645513f2a6",
        OriginalCameraCall, OriginalStaminaCall, CameraSystemEntry), "previous 0403 supported digest is refused");
    check(!MatchesSupportedImage(image.data(), image.size(), base,
        std::string_view(CurrentSha, 63), OriginalCameraCall, OriginalStaminaCall,
        CameraSystemEntry), "truncated image digest is refused");
    check(!MatchesSupportedImage(image.data(), image.size(), base + 0x1000,
        CurrentSha, OriginalCameraCall, OriginalStaminaCall, CameraSystemEntry),
        "stale descriptor relocation base is refused");
    check(!MatchesSupportedImage(image.data(), image.size() - 1, base,
        CurrentSha, OriginalCameraCall, OriginalStaminaCall, CameraSystemEntry),
        "truncated mapped image is refused");
    auto* nt = reinterpret_cast<IMAGE_NT_HEADERS64*>(image.data() + 0x100);
    nt->FileHeader.TimeDateStamp = 0x6ad77f0c;
    nt->OptionalHeader.SizeOfImage = 0x2a1e000;
    check(!admitted(), "old PE timestamp and image size are refused even with current digest");
    nt->FileHeader.TimeDateStamp = 0x6aeb144c;
    nt->OptionalHeader.SizeOfImage = 0x2a3f000;
    nt->FileHeader.Machine = IMAGE_FILE_MACHINE_I386;
    check(!admitted(), "wrong PE architecture is refused");
    nt->FileHeader.Machine = IMAGE_FILE_MACHINE_AMD64;

    for (const char* group : {"guardRanges", "cameraRanges", "metadataRanges"}) {
        for (const auto& range : facts.at(group)) {
            const auto begin = range.at(0).get<std::size_t>();
            const auto length = range.at(1).get<std::size_t>();
            for (std::size_t at = 0; at < length; ++at) {
                image[begin + at] ^= 1;
                check(!admitted(), "each independently captured guard/registration/name byte rejects corruption");
                image[begin + at] ^= 1;
            }
        }
    }

    // This checks only admission of an explicitly supplied owned replacement.
    // It is not Minimap coexistence or loader-ownership runtime evidence.
    std::array<std::uint8_t, 18> entry_patch{};
    entry_patch.fill(0x90);
    entry_patch[0] = 0xe9;
    std::memcpy(image.data() + facts.at("cameraSystemRva").get<std::size_t>(), entry_patch.data(), entry_patch.size());
    check(!admitted(), "an unowned entry replacement is refused");
    check(MatchesSupportedImage(image.data(), image.size(), base, CurrentSha,
        OriginalCameraCall, OriginalStaminaCall, entry_patch),
        "explicitly supplied verified entry replacement preserves interior-call admission");
    image[facts.at("transformRva").get<std::size_t>()] ^= 1;
    check(!MatchesSupportedImage(image.data(), image.size(), base, CurrentSha,
        OriginalCameraCall, OriginalStaminaCall, entry_patch),
        "transform mutation remains refused with an explicit entry replacement");
    image[facts.at("transformRva").get<std::size_t>()] ^= 1;
    const auto entry = hex_bytes(facts.at("cameraEntryHex").get<std::string>());
    std::memcpy(image.data() + facts.at("cameraSystemRva").get<std::size_t>(), entry.data(), entry.size());
    const std::array<std::uint8_t, 5> call_patch = {0xe8, 1, 2, 3, 4};
    std::memcpy(image.data() + facts.at("staminaCallRva").get<std::size_t>(), call_patch.data(), call_patch.size());
    check(!admitted(), "an unexpected stamina call patch is refused");
    check(MatchesSupportedImage(image.data(), image.size(), base, CurrentSha,
        OriginalCameraCall, call_patch, CameraSystemEntry),
        "explicitly supplied owned stamina call replacement is admitted");
    image[facts.at("staminaCallRva").get<std::size_t>() - 1] ^= 1;
    check(!MatchesSupportedImage(image.data(), image.size(), base, CurrentSha,
        OriginalCameraCall, call_patch, CameraSystemEntry),
        "stamina call override does not bypass adjacent argument guards");

    auto relocated = captured_image(facts, 0x200000000ULL);
    check(MatchesSupportedImage(relocated.data(), relocated.size(), 0x200000000ULL,
        CurrentSha, OriginalCameraCall, OriginalStaminaCall, CameraSystemEntry),
        "independent relocation of all captured metadata is admitted");
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

int main(int argc, char** argv) {
    if (argc > 2) { std::cerr << "at most one captured facts JSON path allowed\n"; return 2; }
    Json facts;
    try {
        if (argc == 2) {
            std::ifstream input(argv[1]);
            if (!input) throw std::runtime_error("captured facts JSON unavailable");
            input >> facts;
        } else {
            facts = Json::parse(EmbeddedCurrentFacts);
        }
        test_profile_admission(facts);
    } catch (const std::exception& error) {
        std::cerr << "fixture error: " << error.what() << '\n'; return 2;
    }
    test_owner_lifecycle();
    test_gauge_stub_layout();
    if (failures) {
        std::cerr << failures << " first-person checks failed\n";
        return 1;
    }
    std::cout << checks << " synthetic/static First-person checks passed; 0 failures\n";
    return 0;
}
