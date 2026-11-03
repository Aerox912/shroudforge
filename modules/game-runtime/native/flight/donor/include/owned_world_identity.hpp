#pragma once

#include <cstddef>
#include <cstdint>
#include <limits>

namespace shroudweave::client_context::world_identity {
enum class Kind : std::uint8_t { unknown, canonical, owned_secondary };

inline bool add(std::uintptr_t base, std::uintptr_t offset,
                std::uintptr_t& result) noexcept {
    if (offset > (std::numeric_limits<std::uintptr_t>::max)()-base) return false;
    result=base+offset;
    return true;
}

// Pinned constructor983080 creates precisely these two Root-owned worlds.
// This is world identity only. A secondary result is NOT local-player proof:
// the caller must additionally establish the canonical entity-index alias.
// Every read must enter the caller's existing confirmation journal.
template<class Read>
Kind admit(std::uintptr_t root, std::uintptr_t world, std::uintptr_t allocator,
           std::uint8_t nativeKind, Read&& read) noexcept {
    std::uintptr_t canonical{}, registry{}, ownerField{};
    if (!root || !add(root,0xE179C0,canonical) || !add(root,0x928,registry))
        return Kind::unknown;
    if (world==canonical) return Kind::canonical;
    if (nativeKind!=1 || !add(root,0x15F56C0,ownerField)) return Kind::unknown;
    std::uintptr_t owner{}, expected{}, sourceField{}, canonicalAllocatorField{};
    std::uintptr_t canonicalRegistry{}, canonicalAllocator{}, source{};
    if (!read(ownerField,&owner,sizeof(owner)) || !owner || (owner&7)!=0 ||
        !add(owner,0x140,expected) || expected!=world ||
        !add(owner,0x7DDE40,sourceField) ||
        !read(sourceField,&source,sizeof(source)) || source!=canonical ||
        !add(canonical,0x30,canonicalAllocatorField) ||
        !read(canonical,&canonicalRegistry,sizeof(canonicalRegistry)) ||
        !read(canonicalAllocatorField,&canonicalAllocator,sizeof(canonicalAllocator)) ||
        canonicalRegistry!=registry || canonicalAllocator!=allocator)
        return Kind::unknown;
    return Kind::owned_secondary;
}
} // namespace shroudweave::client_context::world_identity
