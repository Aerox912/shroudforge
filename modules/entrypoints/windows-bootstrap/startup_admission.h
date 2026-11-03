#pragma once
#include <cstdint>
namespace StartupAdmission {
constexpr std::uint32_t FullV1=0x53460100u;
inline bool FullHandshake(std::uint32_t value) {return (value&~3u)==FullV1;}
inline bool RequiredReady(bool initialized,bool has_hook_export,bool hooks,bool overlay,bool graphics) {
    return initialized&&has_hook_export&&hooks&&(!overlay||graphics);
}
// The existing StartupGate remains the sole entrypoint owner. This wrapper
// prevents a success/failure destructor path from completing it twice.
class Completion final {
public:
    using Complete=void(*)(bool);
    explicit Completion(Complete complete):complete_(complete){}
    Completion(const Completion&)=delete;
    Completion& operator=(const Completion&)=delete;
    ~Completion(){if(!done_)complete_(false);}
    bool release(){if(done_)return false;done_=true;complete_(true);return true;}
private:
    Complete complete_;bool done_{};
};
}
