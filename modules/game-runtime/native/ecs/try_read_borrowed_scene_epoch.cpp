#include "ecs/try_read_borrowed_scene_epoch.h"
#include "ecs/ecs_session_state.h"

namespace EcsRuntime {
bool TryReadBorrowedSceneEpoch(const EngineContext::EcsView& current,
                               std::uint64_t& session) noexcept {
    using namespace EcsImplementation;
    session=0;
    if (!current.owners.image || !current.owners.dedicated_server ||
        current.role!=EngineContext::EcsRole::LocalAuthority || !current.scene_owner ||
        current.owners.main_owner!=current.scene_owner || !current.scene_context ||
        current.owners.world_context!=current.scene_context ||
        current.owners.world_uuid!=current.scene_uuid ||
        current.scene_uuid==std::array<std::uint64_t,2>{} || !current.execution_owner ||
        !current.registry_owner || !current.manager || !current.entity_table ||
        current.manager>UINTPTR_MAX-current.layout.manager_count ||
        current.manager>UINTPTR_MAX-current.layout.manager_table) return false;
    try {
        std::unique_lock lock(state_mutex,std::try_to_lock);
        if (!lock.owns_lock() || stop_requested.load(std::memory_order_acquire) ||
            !layout_ready || !component_registry_ready || !component_registry_world_bound ||
            layout_epoch==UINT64_MAX || write_fault_active_locked() ||
            component_registry.owner!=current.registry_owner ||
            component_registry_manager!=current.manager || live_table!=current.entity_table ||
            live_layout.count_address!=current.manager+current.layout.manager_count ||
            live_layout.table_address!=current.manager+current.layout.manager_table ||
            (read_only_binding && read_only_view!=current)) return false;
        session=layout_epoch+1;
        return true;
    } catch (...) {
        return false;
    }
}
}
