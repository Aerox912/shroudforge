#pragma once
#include "current_player_binding.h"
#include "engine-context/engine_owners.h"
#include <memory>
namespace EngineContext { class EcsWriteGuard; }
namespace PlayerOwnership {
enum class PlayerReadStatus { Ok, NoScope, Unavailable, Busy, Invalid, Unreadable, ReleaseFailed };
// Prepare erfolgt vor den originalen Gates. Copy leiht den bereits gehaltenen
// Registry-/Scene-Guard und liefert nur eigene Werte. Der Aufrufer haelt auch
// die aktuelle ECS-Session; Entity-Mitgliedschaft/Generation bleibt dessen
// Aufgabe. Keine Ausgabe ist ein Borrow oder eine Flight-Berechtigung.
class PreparedPlayerReader {
public:
    PreparedPlayerReader();
    ~PreparedPlayerReader();
    bool Prepare();
    PlayerReadStatus Copy(const EngineContext::EcsWriteGuard&,const EngineContext::EcsView&,
                          std::uint64_t session,CurrentSnapshot&) const noexcept;
private:
    struct State;
    std::unique_ptr<State> state_;
};
}
