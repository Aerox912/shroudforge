#pragma once
#include <windows.h>
#include <cstddef>
#include <cstdint>
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
    Result Apply(unsigned demand);
    void Commit() { committed_ = owned_; }
    bool Rollback();
#ifdef SHROUDFORGE_ZOOM_TEST
    using Writer = bool (*)(std::uintptr_t, const void*, const void*, std::size_t);
    Result TestApply(unsigned char* image, std::size_t size, unsigned demand, bool parked, Writer writer);
#endif
private:
    Result ApplyImage(unsigned char* image, std::size_t size, unsigned demand, bool parked);
    unsigned char* target_ = nullptr;
    DWORD protection_ = 0;
    bool owned_ = false, committed_ = false, attempted_ = false, uncertain_ = false;
    using Replace = bool (*)(std::uintptr_t, const void*, const void*, std::size_t);
    Replace replace_ = nullptr;
};
}
