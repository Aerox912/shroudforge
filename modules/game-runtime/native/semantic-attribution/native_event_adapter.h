#pragma once
#include "semantic-attribution/native_frame_capture.h"
#include "semantic-attribution/owned_ledger.h"
#include "semantic-attribution/prepare_attribution.h"
#include <cstddef>
#include <cstdint>
#include <span>

namespace SemanticAttribution {
#ifdef SF_SEMANTIC_ATTRIBUTION_TESTING
namespace testing {class NativeAdapterTestAccess;}
#endif
namespace Native {
inline constexpr std::uint64_t ArchivedProfileIdentity=0xd7baa768eb00aa89ULL;
struct ResidentSite final {
    const Site* site{};
    std::uintptr_t relay{};
    std::uint32_t before_return{},after_return{};
};
}
// Private host-owned scalar attribution only. No NativeCallRoot, native borrow,
// row dereference, SDK export or action permission is produced by this type.
class NativeEventAdapter final {
public:
    NativeEventAdapter(const NativeEventAdapter&)=delete;
    NativeEventAdapter& operator=(const NativeEventAdapter&)=delete;
    static std::size_t RequiredStorage() noexcept;
    void Close() noexcept;
    // Called only after the complete reserved patch set is published under the
    // genuine held-entry token. No startup code calls this preparation package.
    ~NativeEventAdapter();
private:
    static NativeEventAdapter* Construct(std::span<std::byte>,Ledger&,
        std::uint64_t incarnation,std::uintptr_t image,std::uint32_t image_size,
        std::span<const Native::ResidentSite>) noexcept;
    bool PublishTransport() noexcept;
    void PoisonAndClose(CloseReason) noexcept;
    // Caller owns the same exclusive gate used by Capture and publication.
    void CloseLocked(CloseReason) noexcept;
    struct Impl;
    explicit NativeEventAdapter(Impl* impl) noexcept:impl_(impl){}
    void Capture(Native::SavedMachine&,const void* xstate) noexcept;
    EventReply Send(NativeSite,NativeFrameScalar,const Event&) noexcept;
    Impl* impl_{};
    friend void DispatchNativeAttribution(Native::SavedMachine*,const void*) noexcept;
#ifdef SF_SEMANTIC_ATTRIBUTION_TESTING
    friend class testing::NativeAdapterTestAccess;
#endif
    friend Native::PreparationResult Native::PrepareAttribution(const EngineHooks::HeldEntryToken&,
        const EngineHooks::RetainedModuleOwnerToken&,Limits) noexcept;
};
void DispatchNativeAttribution(Native::SavedMachine*,const void*) noexcept;
}
