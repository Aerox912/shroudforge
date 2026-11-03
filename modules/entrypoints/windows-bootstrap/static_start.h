#pragma once
#include <atomic>

namespace BootstrapImplementation {
enum class HostAttach { Inert, Reject, Deferred, Legacy };
constexpr HostAttach ClassifyHostAttach(bool supported_process, bool winmm,
        bool static_attachment) noexcept {
    if (!supported_process) return HostAttach::Inert;
    if (winmm) return static_attachment ? HostAttach::Deferred : HostAttach::Reject;
    return HostAttach::Legacy;
}
// DLL_PROCESS_ATTACH is serialized by Windows. The atomic also makes accidental
// reentry fail without waiting under the loader lock. Failure is terminal.
class StaticStart final {
    enum class State { Dormant, Starting, Started, Failed };
    std::atomic<State> state_{State::Dormant};
public:
    template<class Start> bool Try(bool eligible, Start&& start) noexcept {
        if (!eligible) return false; // No gate effects for a late/unsupported attach.
        auto expected = State::Dormant;
        if (!state_.compare_exchange_strong(expected, State::Starting))
            return expected == State::Started;
        bool result = false;
        try { result = start(); } catch (...) { result = false; }
        state_.store(result ? State::Started : State::Failed);
        return result;
    }
};
}
