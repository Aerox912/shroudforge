// Exercise the merged public status exports with inert resource/feature providers.
#include <cassert>
#include <iostream>
#include "../native/host/describe_runtime.cpp"

namespace Fixture {
std::string resources = R"({"profileId":"current-resource-profile","state":"ready"})";
}
namespace EcsRuntime {
std::string Diagnostics() { return R"({"profile":"execution-profile"})"; }
std::string Status() { return "ecs-ready"; }
}
namespace ResourceRuntime { std::string Status() { return Fixture::resources; } }
namespace NativeMainMenu { std::string Status() { return R"({"ready":false})"; } }
namespace OverlayRuntime {
SFOverlayStatusV1 GraphicsStatus() {
    SFOverlayStatusV1 status{};
    status.terrain_upload_bytes = 8192;
    status.rendered_frames = 7;
    return status;
}
}
namespace MapRuntime {
SnapshotStore& CapturedFrames() { static SnapshotStore store; return store; }
FrameLease SnapshotStore::Acquire(std::uint64_t, std::uint64_t) const {
    FrameLease lease{};
    lease.frame.supported_features = SF_MAP_FEATURE_HUD | SF_MAP_FEATURE_DISCOVERY;
    return lease;
}
}
extern "C" {
bool KFC_RUNTIME_CALL KfcRuntimeFirstPersonAvailable() { return true; }
std::size_t KFC_RUNTIME_CALL KfcRuntimeFirstPersonDetail(char* buffer, std::size_t capacity) {
    constexpr char detail[] = "private first-person provider";
    if (buffer && capacity >= sizeof(detail)) std::memcpy(buffer, detail, sizeof(detail));
    return sizeof(detail);
}
}

int main() {
    char buffer[8192]{};
    KfcRuntimeDiagnostics(buffer, sizeof(buffer));
    const auto report = nlohmann::json::parse(buffer);
    assert(report["profile"] == "current-resource-profile");
    assert(report["executionProfile"] == "execution-profile");
    assert(report["nativeMainMenu"]["ready"] == false);
    assert(report["firstPerson"]["available"] == true);
    assert(report["firstPerson"]["detail"] == "private first-person provider");
    assert(report["mapOverlay"]["terrainUploadBytes"] == 8192);
    assert(report["mapOverlay"]["renderedFrames"] == 7);
    assert(report["mapOverlay"]["hudAdmitted"] == true);
    assert(report["mapOverlay"]["discoveryAdmitted"] == true);
    KfcRuntimeStatus(buffer, sizeof(buffer));
    assert(std::string(buffer).starts_with("profile=current-resource-profile ecs-ready"));
    assert(std::string(buffer).find("first_person_available=true") != std::string::npos);
    assert(std::string(buffer).find("private first-person provider") != std::string::npos);

    Fixture::resources = R"({"profileId":""})";
    KfcRuntimeDiagnostics(buffer, sizeof(buffer));
    assert(nlohmann::json::parse(buffer)["profile"] == "execution-profile");
    KfcRuntimeStatus(buffer, sizeof(buffer));
    assert(std::string(buffer).starts_with("ecs-ready"));
    KfcRuntimeDiagnostics(buffer, 80);
    assert(nlohmann::json::parse(buffer)["error"] == "report-too-large");
    Fixture::resources = "invalid-json";
    KfcRuntimeDiagnostics(buffer, sizeof(buffer));
    assert(nlohmann::json::parse(buffer)["error"] == "diagnostics-exception");
    KfcRuntimeStatus(buffer, sizeof(buffer));
    assert(buffer[0] == '\0');
    KfcRuntimeDiagnostics(nullptr, 0);
    KfcRuntimeStatus(nullptr, 0);
    std::cout << "Runtime status integration passed (inert providers only)\n";
}
