#pragma once

#include <array>
#include <cstddef>
#include <cstdint>

// Privater Join-Kern. Der native Adapter muss einen zusammenhaengenden aktuellen
// Snapshot unter den originalen World-/Netzwerk-Lebensdauergrenzen kopieren.
// Diese Datentypen sind weder eine externe ABI noch ein Besitz-/Schreibtoken.
// entity_current setzt eine gepruefte Tabellenmitgliedschaft voraus. Aufrufer
// duerfen es nicht aus einer zuvor gespeicherten ID oder Generation ableiten.
namespace PlayerOwnership {
inline constexpr std::size_t player_limit = 16;
inline constexpr std::size_t machine_limit = 17;

struct WorldIdentity {
    std::uint64_t session{};
    std::array<std::uint64_t, 2> uuid{};
    bool operator==(const WorldIdentity&) const = default;
};
struct PlayerRow {
    std::uint32_t gameplay_key{}, machine_key{};
    std::uint8_t platform_tag{};
    std::uint64_t platform_uid{};
};
struct MachineRow {
    std::uint32_t key{};
    std::size_t player_count{};
    std::array<std::uint32_t, player_limit> players{};
};
struct WorldPlayer {
    std::uint32_t gameplay_key{}, machine_key{}, entity_id{}, entity_generation{};
    bool entity_current{};
};
struct CurrentSnapshot {
    WorldIdentity world;
    std::array<PlayerRow, player_limit> players{};
    std::array<MachineRow, machine_limit> machines{};
    std::array<WorldPlayer, player_limit> bindings{};
};
struct Binding {
    WorldIdentity world;
    std::uint64_t platform_uid{};
    std::uint32_t gameplay_key{}, machine_key{}, entity_id{}, entity_generation{};
    bool operator==(const Binding&) const = default;
};
enum class Status { InvalidInput, NoPlayer, Ambiguous, InvalidBinding, Resolved };
struct Result {
    Status status{Status::InvalidInput};
    Binding binding{};
};

// Die UID kommt vom aktuellen Transport-Absender, nicht aus dessen Modpayload.
// Die Funktion beansprucht keine staerkere Authentifizierung als der native
// Spiel-/Transportvertrag. Sie fuehrt keine Enginezugriffe oder Allokationen aus.
inline Result resolve(const CurrentSnapshot& snapshot, std::uint64_t peer_uid) noexcept {
    if (!peer_uid || !snapshot.world.session || snapshot.world.uuid == std::array<std::uint64_t, 2>{})
        return {};
    const PlayerRow* player{};
    for (const auto& row : snapshot.players) {
        if (!row.gameplay_key || row.platform_tag != 3 || row.platform_uid != peer_uid) continue;
        if (player) return {Status::Ambiguous, {}};
        player = &row;
    }
    if (!player) return {Status::NoPlayer, {}};
    for (const auto& row : snapshot.players) {
        if (&row != player && row.gameplay_key == player->gameplay_key)
            return {Status::InvalidBinding, {}};
    }
    if (!player->machine_key) return {Status::InvalidBinding, {}};
    // Native Maskierung liefert den Poolindex, aber keine Arraygrenze.
    const auto machine_index = player->machine_key & 0x7fU;
    if (machine_index >= snapshot.machines.size()) return {Status::InvalidBinding, {}};
    const auto& machine = snapshot.machines[machine_index];
    if (machine.key != player->machine_key || machine.player_count > machine.players.size())
        return {Status::InvalidBinding, {}};
    std::size_t owned_matches{};
    for (std::size_t i = 0; i < machine.player_count; ++i)
        if (machine.players[i] == player->gameplay_key) ++owned_matches;
    if (owned_matches != 1) return {Status::InvalidBinding, {}};

    const WorldPlayer* selected{};
    for (const auto& row : snapshot.bindings) {
        if (row.gameplay_key != player->gameplay_key) continue;
        if (selected) return {Status::Ambiguous, {}};
        selected = &row;
    }
    if (!selected || selected->machine_key != player->machine_key || !selected->entity_id ||
        !selected->entity_current) return {Status::InvalidBinding, {}};
    for (const auto& row : snapshot.bindings) {
        if (&row != selected && row.gameplay_key && row.entity_id == selected->entity_id)
            return {Status::Ambiguous, {}};
    }
    return {Status::Resolved, {snapshot.world, peer_uid, player->gameplay_key, player->machine_key,
                              selected->entity_id, selected->entity_generation}};
}

// Nur Wertgleichheit eines frisch gelesenen aktuellen Joins. Kein Engine-Pin:
// Die Wirkung muss ihre native World-/Entity-Lebensdauer zusaetzlich besitzen.
inline bool still_matches(const Binding& selected, const Result& current) noexcept {
    return current.status == Status::Resolved && selected == current.binding;
}
}
