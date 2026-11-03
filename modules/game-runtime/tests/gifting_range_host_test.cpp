#include "../native/item-transfer/gifting_range_call.h"
#include "../native/item-transfer/gifting_range_image_validation.h"

#include <algorithm>
#include <array>
#include <cfloat>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <vector>

namespace {
int original_calls{};

float __cdecl stock_distance() noexcept {
    ++original_calls;
    return 321.5f;
}

void check(bool passed) {
    if (!passed) std::abort();
}

struct ImageFixture {
    static constexpr std::uintptr_t base = 0x140000000;
    static constexpr std::uint32_t image_size = 0x00a10000;
    std::vector<std::uint8_t> bytes = std::vector<std::uint8_t>(image_size);
};

bool read_fixture(const void* context, std::uintptr_t address,
                  void* output, std::size_t size) noexcept {
    if (!context || !output) return false;
    const auto& fixture = *static_cast<const ImageFixture*>(context);
    if (address < fixture.base) return false;
    const auto offset = static_cast<std::size_t>(address - fixture.base);
    if (offset > fixture.bytes.size() || size > fixture.bytes.size() - offset)
        return false;
    std::memcpy(output, fixture.bytes.data() + offset, size);
    return true;
}

template <typename T>
void write_fixture(ImageFixture& fixture, std::uint32_t rva, const T& value) {
    check(rva <= fixture.bytes.size() &&
        sizeof(T) <= fixture.bytes.size() - rva);
    std::memcpy(fixture.bytes.data() + rva, &value, sizeof(T));
}

ImageFixture make_gifting_image() {
    using namespace GiftingRange::ImageValidation;
    ImageFixture fixture;
    constexpr std::uint32_t nt_rva = 0x80;

    IMAGE_DOS_HEADER dos{};
    dos.e_magic = IMAGE_DOS_SIGNATURE;
    dos.e_lfanew = nt_rva;
    write_fixture(fixture, 0, dos);

    IMAGE_NT_HEADERS64 nt{};
    nt.Signature = IMAGE_NT_SIGNATURE;
    nt.FileHeader.Machine = IMAGE_FILE_MACHINE_AMD64;
    nt.FileHeader.NumberOfSections = 2;
    nt.FileHeader.TimeDateStamp = 0x01020304;
    nt.FileHeader.SizeOfOptionalHeader = sizeof(IMAGE_OPTIONAL_HEADER64);
    nt.OptionalHeader.Magic = IMAGE_NT_OPTIONAL_HDR64_MAGIC;
    nt.OptionalHeader.SizeOfImage = ImageFixture::image_size;
    nt.OptionalHeader.SizeOfHeaders = 0x1000;
    nt.OptionalHeader.SectionAlignment = 0x1000;
    nt.OptionalHeader.FileAlignment = 0x200;
    nt.OptionalHeader.NumberOfRvaAndSizes = IMAGE_NUMBEROF_DIRECTORY_ENTRIES;
    constexpr std::uint32_t pdata_rva = 0x00a00000;
    nt.OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION] = {
        pdata_rva, sizeof(RUNTIME_FUNCTION)};
    write_fixture(fixture, nt_rva, nt);

    IMAGE_SECTION_HEADER sections[2]{};
    std::memcpy(sections[0].Name, ".text", 5);
    sections[0].Misc.VirtualSize = pdata_rva - 0x1000;
    sections[0].VirtualAddress = 0x1000;
    sections[0].Characteristics = IMAGE_SCN_CNT_CODE |
        IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_EXECUTE;
    std::memcpy(sections[1].Name, ".pdata", 6);
    sections[1].Misc.VirtualSize = 0x1000;
    sections[1].VirtualAddress = pdata_rva;
    sections[1].Characteristics = IMAGE_SCN_CNT_INITIALIZED_DATA |
        IMAGE_SCN_MEM_READ;
    const auto section_rva = nt_rva +
        offsetof(IMAGE_NT_HEADERS64, OptionalHeader) +
        nt.FileHeader.SizeOfOptionalHeader;
    check(section_rva <= fixture.bytes.size() &&
        sizeof(sections) <= fixture.bytes.size() - section_rva);
    std::memcpy(fixture.bytes.data() + section_rva, sections, sizeof(sections));

    RUNTIME_FUNCTION function{};
    function.BeginAddress = static_cast<DWORD>(caller_begin_rva);
    function.EndAddress = static_cast<DWORD>(caller_end_rva);
    function.UnwindData = pdata_rva + 0x20;
    write_fixture(fixture, pdata_rva, function);

    std::array<std::uint8_t, caller_end_rva - caller_begin_rva> caller{};
    for (std::size_t i = 0; i < caller.size(); ++i)
        caller[i] = static_cast<std::uint8_t>((i * 37 + 19) & 0xff);
    const auto call_displacement = static_cast<std::int32_t>(
        static_cast<std::int64_t>(getter_rva) -
        static_cast<std::int64_t>(call_rva + stock_call.size()));
    auto call_bytes = stock_call;
    std::memcpy(call_bytes.data() + 1, &call_displacement,
        sizeof(call_displacement));
    std::memcpy(caller.data() + (call_rva - caller_begin_rva),
        call_bytes.data(), call_bytes.size());
    std::memcpy(fixture.bytes.data() + caller_begin_rva,
        caller.data(), caller.size());
    std::memcpy(fixture.bytes.data() + getter_rva,
        stock_getter.data(), stock_getter.size());
    return fixture;
}

GiftingRange::ImageValidation::CodeDigests digests_for(
    const ImageFixture& fixture) {
    using namespace GiftingRange::ImageValidation;
    CodeDigests digests{};
    check(compute_sha256(fixture.bytes.data() + caller_begin_rva,
        caller_end_rva - caller_begin_rva, digests.caller));
    check(compute_sha256(fixture.bytes.data() + getter_rva,
        getter_size, digests.getter));
    return digests;
}

bool validate_fixture(const ImageFixture& fixture,
                      const GiftingRange::ImageValidation::CodeDigests& digests,
                      std::uint32_t& validated_size) {
    return GiftingRange::ImageValidation::validate_loaded_image(
        fixture.base, &read_fixture, &fixture, digests, validated_size);
}

void test_gifting_image_validation() {
    using namespace GiftingRange::ImageValidation;
    constexpr std::uint32_t prior_profile_timestamp = 1792507668;
    constexpr std::uint32_t prior_profile_image_size = 0x1dd1000;
    auto valid = make_gifting_image();
    IMAGE_NT_HEADERS64 nt{};
    std::memcpy(&nt, valid.bytes.data() + 0x80, sizeof(nt));
    check(nt.FileHeader.TimeDateStamp != prior_profile_timestamp);
    check(nt.OptionalHeader.SizeOfImage != prior_profile_image_size);

    const auto expected = digests_for(valid);
    std::uint32_t validated_size{};
    check(validate_fixture(valid, expected, validated_size));
    check(validated_size == ImageFixture::image_size);

    auto changed = valid;
    changed.bytes[caller_begin_rva + 7] ^= 0x01;
    check(!validate_fixture(changed, expected, validated_size));

    auto wrong_target = valid;
    const auto wrong_displacement = static_cast<std::int32_t>(
        static_cast<std::int64_t>(getter_rva + 1) -
        static_cast<std::int64_t>(call_rva + stock_call.size()));
    std::memcpy(wrong_target.bytes.data() + call_rva + 1,
        &wrong_displacement, sizeof(wrong_displacement));
    const auto wrong_target_digests = digests_for(wrong_target);
    check(!validate_fixture(wrong_target, wrong_target_digests, validated_size));

    auto shifted = valid;
    std::memmove(shifted.bytes.data() + caller_begin_rva + 1,
        shifted.bytes.data() + caller_begin_rva,
        caller_end_rva - caller_begin_rva);
    shifted.bytes[caller_begin_rva] = 0xcc;
    check(!validate_fixture(shifted, expected, validated_size));

    auto wrong_function_range = valid;
    const auto pdata_rva = nt.OptionalHeader
        .DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION].VirtualAddress;
    RUNTIME_FUNCTION function{};
    std::memcpy(&function, wrong_function_range.bytes.data() + pdata_rva,
        sizeof(function));
    ++function.BeginAddress;
    ++function.EndAddress;
    write_fixture(wrong_function_range, pdata_rva, function);
    check(!validate_fixture(wrong_function_range, expected, validated_size));
}
}

int main() {
    test_gifting_image_validation();

    GiftingRange::PublicationGate gate;

    check(!GiftingRange::SelectsDedicatedServerInstall(0, true, true));
    check(!GiftingRange::SelectsDedicatedServerInstall(1, false, true));
    check(!GiftingRange::SelectsDedicatedServerInstall(1, true, false));
    check(GiftingRange::SelectsDedicatedServerInstall(1, true, true));

    constexpr std::uintptr_t call_site =
        GiftingRange::ImageValidation::call_rva;
    constexpr std::uintptr_t original_target =
        GiftingRange::ImageValidation::getter_rva;
    constexpr std::uintptr_t relay = 0x199500;
    constexpr auto stock_call = GiftingRange::ImageValidation::stock_call;
    std::array<std::uint8_t, 5> rewritten{};
    check(GiftingRange::BuildDirectCallPatch(call_site, stock_call, original_target,
        relay, rewritten));
    check(rewritten[0] == stock_call[0]);
    std::int32_t new_displacement{};
    std::memcpy(&new_displacement, rewritten.data() + 1, sizeof(new_displacement));
    check(static_cast<std::int64_t>(call_site + 5) + new_displacement == relay);
    check(!GiftingRange::BuildDirectCallPatch(call_site, stock_call,
        original_target + 1, relay, rewritten));
    check(!GiftingRange::BuildDirectCallPatch(call_site, stock_call,
        original_target, 0x9000000000, rewritten));

    check(GiftingRange::TransferDistance(gate, &stock_distance) == 321.5f);
    check(original_calls == 1);

    check(gate.MarkPrepared());
    check(GiftingRange::TransferDistance(gate, &stock_distance) == 321.5f);
    check(original_calls == 2);

    check(gate.Complete(false));
    check(!gate.Active());
    check(!gate.Complete(true));
    check(!gate.MarkPrepared());
    check(GiftingRange::TransferDistance(gate, &stock_distance) == 321.5f);
    check(original_calls == 3);

    GiftingRange::PublicationGate accepted;
    check(accepted.MarkPrepared());
    check(accepted.Complete(true));
    check(!accepted.MarkPrepared());
    check(GiftingRange::TransferDistance(accepted, &stock_distance) == FLT_MAX);
    check(original_calls == 3);
}
