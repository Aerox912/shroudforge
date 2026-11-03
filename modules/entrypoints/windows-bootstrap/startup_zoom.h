#pragma once
#include <windows.h>
#include <cstddef>
#include <cstdint>
#include "../../game-runtime/native/engine-hooks/held_entry_preparation.h"
namespace StartupZoom {
enum class Result { Disabled, Applied, Unavailable, Unsafe };
// Ownership lasts until process exit after Commit. Only the parked bootstrap
// may roll back an uncommitted exact replacement; there is no live toggle API.
class Cap final {
public:
    Cap() = default;
    Cap(const Cap&) = delete;
    Cap& operator=(const Cap&) = delete;
    ~Cap();
    Result Apply(unsigned demand, const EngineHooks::HeldEntryProof& proof);
    void Commit() { committed_ = owned_; }
    bool Rollback();
    bool Rollback(const EngineHooks::HeldEntryProof& proof);
#ifdef SHROUDFORGE_ZOOM_TEST
    using Writer = bool (*)(std::uintptr_t, const void*, const void*, std::size_t);
    Result TestApply(unsigned char* image, std::size_t size, unsigned demand, bool parked, Writer writer);
#endif
private:
    using Replace = bool (*)(std::uintptr_t, const void*, const void*, std::size_t);
    Result ApplyImage(unsigned char* image, std::size_t size, unsigned demand, bool parked,
                      const EngineHooks::HeldEntryProof* proof, Replace test_writer);
    bool Restore(const EngineHooks::HeldEntryProof* proof, Replace test_writer);
    unsigned char* target_ = nullptr;
    DWORD protection_ = 0;
    bool owned_ = false, committed_ = false, attempted_ = false, uncertain_ = false;
    const EngineHooks::HeldEntryProof* proof_ = nullptr;
#ifdef SHROUDFORGE_ZOOM_TEST
    Replace test_writer_ = nullptr;
#endif
};
}
