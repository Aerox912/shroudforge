#include "impact/resolve_registered_operation.h"
#include "named-services/named_service_contract.h"

namespace ImpactRuntime {
namespace {
using NamedServiceImplementation::add;
using NamedServiceImplementation::copy;
using NamedServiceImplementation::field;

constexpr std::size_t maximum_buckets = 4096;

struct Table {
    std::uintptr_t bits{}, keys{}, records{};
    std::uint64_t buckets{};
    std::uint32_t count{};
    bool operator==(const Table&) const = default;
};

struct OperationRecord {
    std::uintptr_t callback{}, name{};
    std::uint64_t length{}, capacity{};
    std::uintptr_t allocator{};
    std::array<char,32> inline_name{};
    std::uint8_t arguments{};
    std::array<std::uint8_t,7> padding{};
    bool operator==(const OperationRecord&) const = default;
};
static_assert(sizeof(OperationRecord) == 80);
static_assert(offsetof(OperationRecord, inline_name) == 40);
static_assert(offsetof(OperationRecord, arguments) == 72);

struct Selection {
    std::uintptr_t address{};
    std::uint32_t key{};
    std::size_t slot{};
    OperationRecord record;
    bool operator==(const Selection&) const = default;
};

using Occupancy = std::array<std::uint64_t, maximum_buckets / 64>;

bool range(std::uintptr_t address, std::size_t size){
    return address && size && size <= UINTPTR_MAX - address;
}

bool main_image_bounds(std::uintptr_t& base, std::uint32_t& size) {
    const auto module = GetModuleHandleW(nullptr);
    base = reinterpret_cast<std::uintptr_t>(module);
    IMAGE_DOS_HEADER dos{};
    if (!base || !copy(&dos, module, sizeof(dos)) || dos.e_magic != IMAGE_DOS_SIGNATURE ||
        dos.e_lfanew < static_cast<LONG>(sizeof(dos)) || dos.e_lfanew > 1024 * 1024)
        return false;
    std::uintptr_t header{};
    if (!add(base, static_cast<std::size_t>(dos.e_lfanew), header)) return false;
    IMAGE_NT_HEADERS64 nt{};
    if (!copy(&nt, reinterpret_cast<const void*>(header), sizeof(nt)) ||
        nt.Signature != IMAGE_NT_SIGNATURE || nt.FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 ||
        nt.OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC || !nt.OptionalHeader.SizeOfImage)
        return false;
    size = nt.OptionalHeader.SizeOfImage;
    return true;
}

bool table(std::uintptr_t service, Table& result){
    return field(service, 0x28, result.bits) && field(service, 0x30, result.buckets) &&
        field(service, 0x48, result.keys) && field(service, 0x60, result.records) &&
        field(service, 0x74, result.count);
}

bool occupied(const Occupancy& bits, std::size_t slot){
    return (bits[slot / 64] & (std::uint64_t{1} << (slot % 64))) != 0;
}

ResolveResult select(const Table& current, const Occupancy& bits,
    const char* name, std::size_t length, Selection& output){
    std::size_t seen{}, matches{};
    Selection selected;
    for (std::size_t slot = 0; slot < current.buckets; ++slot) {
        if (!occupied(bits, slot)) continue;
        ++seen;
        Selection candidate;
        candidate.slot = slot;
        if (!add(current.records, slot * sizeof(OperationRecord), candidate.address) ||
            !copy(&candidate.record, reinterpret_cast<const void*>(candidate.address), sizeof(OperationRecord)) ||
            !field(current.keys, slot * sizeof(std::uint32_t), candidate.key))
            return ResolveResult::RegistryUnavailable;
        const auto& row = candidate.record;
        if (row.capacity != row.inline_name.size() || !row.length || row.length >= row.capacity ||
            row.name != candidate.address + offsetof(OperationRecord, inline_name) ||
            row.inline_name[static_cast<std::size_t>(row.length)] != '\0' ||
            !std::all_of(row.inline_name.begin(), row.inline_name.begin() + static_cast<std::size_t>(row.length),
                [](unsigned char byte) { return byte >= 32 && byte < 127; }))
            return ResolveResult::UnsupportedContract;
        if (row.length != length || std::memcmp(row.inline_name.data(), name, length)) continue;
        if (++matches != 1) return ResolveResult::UnsupportedContract;
        selected = candidate;
    }
    if (seen != current.count) return ResolveResult::UnsupportedContract;
    if (!matches) return ResolveResult::RegistryUnavailable;
    // The name selects the record. Its observed key only verifies that the
    // record is actually reachable in the original occupied linear-probe map.
    auto slot = static_cast<std::size_t>(selected.key) & (current.buckets - 1);
    for (std::size_t visited = 0; visited < current.buckets; ++visited) {
        std::uint32_t key{};
        if (!occupied(bits, slot)) return ResolveResult::UnsupportedContract;
        if (!field(current.keys, slot * sizeof(key), key)) return ResolveResult::RegistryUnavailable;
        if (key == selected.key) {
            if (slot != selected.slot) return ResolveResult::UnsupportedContract;
            output = selected;
            return ResolveResult::Found;
        }
        slot = (slot + 1) & (current.buckets - 1);
    }
    return ResolveResult::UnsupportedContract;
}

}

ResolveResult ResolveRegisteredOperationPinned(const NamedServiceImplementation::BindingContract& binding,
    const EngineContext::EcsView& owner, const char* exact_name, RegisteredOperation& output){
    if (!exact_name || binding.key != "impact::ImpactSystem")
        return ResolveResult::UnsupportedContract;
    std::size_t length{};
    while (length < 32 && exact_name[length]) ++length;
    if (!length || length == 32) return ResolveResult::UnsupportedContract;
    std::uintptr_t service{}, again{};
    if (!NamedServiceImplementation::resolve_binding(binding, owner, service))
        return ResolveResult::RegistryUnavailable;
    Table current, repeated;
    if (!table(service, current)) return ResolveResult::RegistryUnavailable;
    if (current.buckets < 2 || current.buckets > maximum_buckets ||
        (current.buckets & (current.buckets - 1)) || !current.count || current.count > current.buckets)
        return ResolveResult::UnsupportedContract;
    const auto words = static_cast<std::size_t>((current.buckets + 63) / 64);
    if (!range(current.bits, words * sizeof(std::uint64_t)) ||
        !range(current.keys, static_cast<std::size_t>(current.buckets) * sizeof(std::uint32_t)) ||
        !range(current.records, static_cast<std::size_t>(current.buckets) * sizeof(OperationRecord)))
        return ResolveResult::UnsupportedContract;
    Occupancy bits{}, bits_again{};
    if (!copy(bits.data(), reinterpret_cast<const void*>(current.bits), words * sizeof(std::uint64_t)))
        return ResolveResult::RegistryUnavailable;
    if (current.buckets < 64 && (bits[0] >> current.buckets)) return ResolveResult::UnsupportedContract;
    Selection selected, selected_again;
    auto result = select(current, bits, exact_name, length, selected);
    if (result != ResolveResult::Found) return result;
    if (!table(service, repeated) || repeated != current ||
        !copy(bits_again.data(), reinterpret_cast<const void*>(current.bits), words * sizeof(std::uint64_t)) ||
        bits_again != bits) return ResolveResult::RegistryUnavailable;
    result = select(repeated, bits_again, exact_name, length, selected_again);
    if (result != ResolveResult::Found) return result;
    if (selected_again != selected || !table(service, repeated) || repeated != current ||
        !copy(bits_again.data(), reinterpret_cast<const void*>(current.bits), words * sizeof(std::uint64_t)) ||
        bits_again != bits ||
        !NamedServiceImplementation::resolve_binding(binding, owner, again) || again != service)
        return ResolveResult::RegistryUnavailable;
    if (!selected.record.callback || !NamedServiceImplementation::executable_callback(selected.record.callback))
        return ResolveResult::UnsupportedContract;
    output = {selected.record.callback, selected.record.arguments, selected.record.inline_name};
    return ResolveResult::Found;
}

ResolveResult ResolveRegisteredTerraformingMetadataPinned(
    const NamedServiceImplementation::BindingContract& binding,
    const EngineContext::EcsView& owner, std::uint64_t session_generation,
    RegisteredOperationMetadata& output) {
    if (!session_generation || KfcRuntimeWorldSessionId() != session_generation)
        return ResolveResult::SessionChanged;
    RegisteredOperation operation;
    const auto resolved = ResolveRegisteredOperationPinned(binding, owner, "terraforming", operation);
    if (resolved != ResolveResult::Found) return resolved;
    if (std::strcmp(operation.name.data(), "terraforming")) return ResolveResult::UnsupportedContract;

    std::uintptr_t base{};
    std::uint32_t image_size{};
    if (!main_image_bounds(base, image_size) || operation.callback < base ||
        operation.callback - base >= image_size || operation.callback - base > UINT32_MAX)
        return ResolveResult::UnsupportedContract;
    if (!NamedServiceImplementation::executable_callback(operation.callback) ||
        KfcRuntimeWorldSessionId() != session_generation)
        return ResolveResult::SessionChanged;

    RegisteredOperationMetadata prepared;
    prepared.callback_rva = static_cast<std::uint32_t>(operation.callback - base);
    prepared.image_size = image_size;
    prepared.session_generation = session_generation;
    prepared.arguments = operation.arguments;
    prepared.name = operation.name;
    output = prepared;
    return ResolveResult::Found;
}

ResolveResult ResolveRegisteredOperation(std::uint64_t expected_session,
    const char* exact_name, RegisteredOperation& output){
    if (!expected_session || !exact_name || !*exact_name || std::strlen(exact_name) >= 32)
        return ResolveResult::UnsupportedContract;
    if (KfcRuntimeWorldSessionId() != expected_session) return ResolveResult::SessionChanged;
    try {
        const auto binding = NamedServiceImplementation::binding_contract("impact::ImpactSystem");
        RegisteredOperation found;
        {
            std::scoped_lock operation(NamedServiceImplementation::operation_mutex);
            EcsRuntime::SessionGuard session;
            if (!session.TryAcquire(expected_session)) return ResolveResult::SessionChanged;
            EngineContext::EcsWriteGuard scene;
            if (!scene.TryAcquire(session.View())) return ResolveResult::RegistryUnavailable;
            if (!scene.Validate()) return ResolveResult::SessionChanged;
            const auto result = ResolveRegisteredOperationPinned(binding, session.View(), exact_name, found);
            if (result != ResolveResult::Found) return result;
            if (!scene.Validate()) return ResolveResult::SessionChanged;
        }
        // No borrowed service data or locks survive this point. Main-image code
        // remains loaded independently of the world and operation table lifetime.
        if (KfcRuntimeWorldSessionId() != expected_session) return ResolveResult::SessionChanged;
        output = found;
        return ResolveResult::Found;
    } catch (...) { return ResolveResult::RegistryUnavailable; }
}
}
