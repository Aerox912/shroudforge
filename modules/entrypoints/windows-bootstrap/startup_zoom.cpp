#include "startup_zoom.h"
#include "startup_zoom_profile.h"
#include "startup_gate.h"
#include "engine-hooks/write_executable_bytes.h"
#include <psapi.h>
#include <cstring>
#include <algorithm>

namespace StartupZoom {
namespace {
bool Readable(const unsigned char* address, std::size_t size) {
    while (size) {
        MEMORY_BASIC_INFORMATION region{};
        if (!VirtualQuery(address, &region, sizeof(region)) || region.State != MEM_COMMIT ||
            (region.Protect & (PAGE_GUARD | PAGE_NOACCESS)) || !region.Protect) return false;
        const auto available = static_cast<const unsigned char*>(region.BaseAddress) + region.RegionSize - address;
        const auto part = (std::min)(size, static_cast<std::size_t>(available));
        if (!part) return false;
        address += part; size -= part;
    }
    return true;
}
bool Protection(const unsigned char* address, DWORD expected) {
    MEMORY_BASIC_INFORMATION region{};
    return VirtualQuery(address, &region, sizeof(region)) && region.State == MEM_COMMIT &&
        region.Protect == expected &&
        static_cast<const unsigned char*>(region.BaseAddress) + region.RegionSize >= address + 4;
}
bool Validate(unsigned char* image, std::size_t size) {
    if (!image || size != Profile::ImageSize || !Readable(image, sizeof(IMAGE_DOS_HEADER))) return false;
    const auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(image);
    if (dos->e_magic != IMAGE_DOS_SIGNATURE || dos->e_lfanew <= 0 ||
        static_cast<std::size_t>(dos->e_lfanew) > size - sizeof(IMAGE_NT_HEADERS64)) return false;
    const auto* nt = reinterpret_cast<const IMAGE_NT_HEADERS64*>(image + dos->e_lfanew);
    if (!Readable(reinterpret_cast<const unsigned char*>(nt), sizeof(*nt)) || nt->Signature != IMAGE_NT_SIGNATURE ||
        nt->FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 || nt->FileHeader.TimeDateStamp != Profile::Timestamp ||
        nt->FileHeader.SizeOfOptionalHeader != sizeof(IMAGE_OPTIONAL_HEADER64) ||
        nt->OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC || nt->OptionalHeader.SizeOfImage != size ||
        nt->OptionalHeader.NumberOfRvaAndSizes <= IMAGE_DIRECTORY_ENTRY_EXCEPTION ||
        !nt->FileHeader.NumberOfSections || nt->FileHeader.NumberOfSections > 96) return false;
    const auto* sections = IMAGE_FIRST_SECTION(nt);
    const auto sectionOffset = reinterpret_cast<const unsigned char*>(sections) - image;
    const auto sectionBytes = sizeof(IMAGE_SECTION_HEADER) * nt->FileHeader.NumberOfSections;
    if (sectionOffset < 0 || static_cast<std::size_t>(sectionOffset) > size - sectionBytes ||
        !Readable(reinterpret_cast<const unsigned char*>(sections), sectionBytes)) return false;
    const auto& exception = nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION];
    if (!exception.VirtualAddress || !exception.Size || exception.Size % sizeof(RUNTIME_FUNCTION) ||
        exception.Size > size || exception.VirtualAddress > size - exception.Size ||
        !Readable(image + exception.VirtualAddress, exception.Size)) return false;
    const auto* functions = reinterpret_cast<const RUNTIME_FUNCTION*>(image + exception.VirtualAddress);
    for (const auto& signature : Profile::Signatures) {
        if (signature.rva > size - signature.length || !Readable(image + signature.rva, signature.length) ||
            std::memcmp(image + signature.rva, signature.bytes.data(), signature.length)) return false;
        unsigned associations = 0;
        for (std::size_t i=0; i<exception.Size/sizeof(RUNTIME_FUNCTION); ++i) {
            const auto& f = functions[i];
            if (f.BeginAddress <= signature.rva && signature.rva < f.EndAddress) {
                if (f.BeginAddress != signature.begin || f.EndAddress != signature.end || f.UnwindData != signature.unwind ||
                    f.EndAddress > size || f.UnwindData >= size) return false;
                ++associations;
            }
        }
        if (associations != 1) return false;
        unsigned matches = 0, containing = 0;
        for (unsigned i=0; i<nt->FileHeader.NumberOfSections; ++i) {
            const auto& section = sections[i];
            if (!(section.Characteristics & IMAGE_SCN_MEM_EXECUTE)) continue;
            const auto length = static_cast<std::size_t>(section.Misc.VirtualSize);
            if (!length || length > size || section.VirtualAddress > size-length ||
                !Readable(image + section.VirtualAddress, length)) return false;
            if (signature.rva >= section.VirtualAddress && signature.rva + signature.length <= section.VirtualAddress + length)
                ++containing;
            if (length >= signature.length) for (std::size_t offset=0; offset<=length-signature.length; ++offset) {
                if (image[section.VirtualAddress+offset] == signature.bytes[0] &&
                    !std::memcmp(image+section.VirtualAddress+offset, signature.bytes.data(), signature.length)) ++matches;
            }
        }
        if (matches != 1 || containing != 1) return false;
    }
    return true;
}
}

Cap::~Cap() { if (owned_ && !committed_) Rollback(); }

Result Cap::Apply(unsigned demand) {
    if (demand == 0) return Result::Disabled;
    if (demand != 1) return Result::Unavailable;
    // No late attach: main must still be parked and completion unsignalled.
    const bool parked = StartupGate::entrypoint &&
        WaitForSingleObject(StartupGate::entered, 0) == WAIT_OBJECT_0 &&
        WaitForSingleObject(StartupGate::completed, 0) == WAIT_TIMEOUT;
    if (!parked) return Result::Unavailable;
    MODULEINFO info{};
    if (!K32GetModuleInformation(GetCurrentProcess(), GetModuleHandleW(nullptr), &info, sizeof(info)))
        return Result::Unavailable;
    // Retain the owner even if another component later releases its DLL handle.
    // Never undo this feature from DllMain or a live stop/hot-toggle path.
    HMODULE pinned{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
        reinterpret_cast<LPCWSTR>(&Validate), &pinned)) return Result::Unavailable;
    replace_ = EngineHooks::ReplaceExecutableBytes;
    return ApplyImage(static_cast<unsigned char*>(info.lpBaseOfDll), info.SizeOfImage, demand, parked);
}

Result Cap::ApplyImage(unsigned char* image, std::size_t size, unsigned demand, bool parked) {
    if (attempted_) return Result::Unavailable;
    attempted_ = true;
    if (demand == 0) return Result::Disabled;
    if (demand != 1 || !parked || !Validate(image,size)) return Result::Unavailable;
    target_ = image + Profile::Signatures[0].rva;
    MEMORY_BASIC_INFORMATION region{};
    if (!VirtualQuery(target_, &region, sizeof(region)) ||
        (region.Protect != PAGE_EXECUTE_READ && region.Protect != PAGE_EXECUTE)) return Result::Unavailable;
    protection_ = region.Protect;
    const bool wrote = replace_(reinterpret_cast<std::uintptr_t>(target_), Profile::Original, Profile::Replacement, 4);
    owned_ = std::memcmp(target_, Profile::Replacement, 4) == 0;
    const bool protectedAgain = Protection(target_, protection_);
    // Check the primitive's ignored cache-flush result ourselves before release.
    const bool flushed = FlushInstructionCache(GetCurrentProcess(), target_, 4) != FALSE;
    if (wrote && owned_ && protectedAgain && flushed) return Result::Applied;
    if (!wrote && !owned_ && !std::memcmp(target_, Profile::Original, 4) && protectedAgain && flushed)
        return Result::Unavailable;
    uncertain_ = true;
    return Result::Unsafe;
}

bool Cap::Rollback() {
    if (committed_) return false;
    if (!owned_) return !uncertain_;
    if (!Protection(target_, protection_) || std::memcmp(target_, Profile::Replacement, 4)) return false;
    if (!replace_(reinterpret_cast<std::uintptr_t>(target_), Profile::Replacement, Profile::Original, 4) ||
        !Protection(target_, protection_) || std::memcmp(target_, Profile::Original, 4) ||
        !FlushInstructionCache(GetCurrentProcess(), target_, 4)) return false;
    owned_ = false;
    uncertain_ = false;
    return true;
}
#ifdef SHROUDFORGE_ZOOM_TEST
Result Cap::TestApply(unsigned char* image, std::size_t size, unsigned demand, bool parked, Writer writer) {
    replace_ = writer;
    return ApplyImage(image,size,demand,parked);
}
#endif
}
