#pragma once
#include "engine-context/engine_owners.h"
namespace PlayerOwnership { class PreparedPlayerReader; }
namespace EngineContext {
class ResourceLifeGuard;
// Read-only capability snapshot. A write must repeat this policy under the
// original Scene gate before mutating the component payload.
bool EcsComponentWriteSupported(const EcsView& expected, std::uint16_t component_index);
// Owns the original native-thread registration and Scene execution gates.
// Acquire is nonblocking. Callers prepare buffers and take their own mutexes
// before entry, then perform bounded memory operations. Native calls require
// their own complete contract, such as TrackChange's admitted no-op branch or
// the typed original destroyEntity queue append. Owning this guard alone never
// authorizes an arbitrary callback, system frame or memory mutation.
class EcsWriteGuard {
public:
    EcsWriteGuard();
    ~EcsWriteGuard();
    EcsWriteGuard(const EcsWriteGuard&) = delete;
    EcsWriteGuard& operator=(const EcsWriteGuard&) = delete;
    bool TryAcquire(const EcsView& expected, const char** reason = nullptr);
    // Revalidates the captured complete Scene/owner chain using only the
    // prepared profile, without allocation or acquiring context_mutex.
    bool Validate() const;
    bool ComponentSupported(std::uint16_t component_index) const;
    bool TrackChange(std::uintptr_t entity, std::uint16_t component_index) const;
    void Release() noexcept;
    std::size_t MaximumBytes() const { return maximum_bytes_; }
private:
    friend class ResourceLifeGuard;
    friend class PlayerOwnership::PreparedPlayerReader;
    struct State;
    std::unique_ptr<State> state_;
    std::uintptr_t registry_lock_{}, scene_lock_{};
    std::size_t maximum_bytes_{};
    std::uintptr_t manager_{}, tracker_{}, tracking_context_{};
    std::uintptr_t registry_owner_{};
    std::array<std::uint8_t, 33> tracker_bytes_{};
    std::size_t tracking_manager_offset_{};
    std::size_t tracking_registry_a_{}, tracking_registry_b_{};
};
}
