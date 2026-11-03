#include "abi/runtime_api.h"
#include "graphics/render_overlay.h"
#include "map/capture_game_map.h"
#include "voxels/inspect_registered_writer_metadata.h"
#include "profiles/game_profile.h"
#include <windows.h>
#include "ecs/entity_session.h"
#include "loaded-resources/loaded_resource_access.h"
#include "main-menu/native_main_menu.h"
#include <algorithm>
#include <cctype>
#include <cstring>
#include <string>
#include <nlohmann/json.hpp>
#include "host/runtime_lifecycle_state.h"

using namespace ProviderImplementation;

namespace {
std::string FirstPersonDetail() {
    const auto required = KfcRuntimeFirstPersonDetail(nullptr, 0);
    if (required < 2 || required > 4096) {
        return "first-person provider detail is unavailable";
    }
    std::string detail(required, '\0');
    const auto actual = KfcRuntimeFirstPersonDetail(detail.data(), detail.size());
    if (actual != required || detail.back() != '\0') {
        return "first-person provider detail changed while being copied";
    }
    detail.resize(required - 1);
    return detail;
}

std::string SafeImageSha256(const std::string& value) {
    if (value.size() != 64) return "unknown";
    std::string normalized;
    normalized.reserve(value.size());
    for (const auto byte : value) {
        const auto lower = static_cast<unsigned char>(std::tolower(static_cast<unsigned char>(byte)));
        if (!((lower >= '0' && lower <= '9') || (lower >= 'a' && lower <= 'f'))) return "unknown";
        normalized.push_back(static_cast<char>(lower));
    }
    return normalized;
}

const char* SafeImageTarget(const std::string& value) {
    if (value == "enshrouded.exe") return "enshrouded.exe";
    if (value == "enshrouded_server.exe") return "enshrouded_server.exe";
    return "unknown";
}
}

extern "C" {
KFC_RUNTIME_API void __cdecl KfcRuntimeDiagnostics(char* buffer, std::size_t capacity) {
    if (!buffer || !capacity) return;
    const auto write_error = [buffer, capacity](const char* message, std::size_t required = 0) {
        const auto text = std::string("{\"error\":\"") + message +
            (required ? "\",\"requiredBytes\":" + std::to_string(required) : "\"") + "}";
        const auto size = (std::min)(text.size(), capacity - 1);
        std::memcpy(buffer, text.data(), size); buffer[size] = 0;
    };
    try {
        auto diagnostics = nlohmann::json::parse(EcsRuntime::Diagnostics());
        diagnostics["resourceService"] = nlohmann::json::parse(ResourceRuntime::Status());
        const auto overlay=OverlayRuntime::GraphicsStatus();const auto map=MapRuntime::CapturedFrames().Acquire(0,GetTickCount64());
        diagnostics["mapOverlay"]={{"mapStatus",map.frame.status},{"supportedFeatures",map.frame.supported_features},
            {"hudAdmitted",bool(map.frame.supported_features&SF_MAP_FEATURE_HUD)},{"discoveryAdmitted",bool(map.frame.supported_features&SF_MAP_FEATURE_DISCOVERY)},
            {"overlayStatus",overlay.status},{"overlayReason",overlay.reason},{"deviceGeneration",overlay.device_generation},
            {"renderedFrames",overlay.rendered_frames},{"busyFrames",overlay.busy_frames},{"failedFrames",overlay.failed_frames},
            {"terrainUploadBytes",overlay.terrain_upload_bytes},{"terrainUploadRegions",overlay.terrain_upload_regions},{"cpuFallbackFrames",overlay.cpu_fallback_frames},
            {"action",overlay.reason==SF_OVERLAY_REASON_AWAIT_OBSERVED_CREATE?"requires an observed swapchain recreation after interception; initial swapchain metadata is unavailable":overlay.reason==SF_OVERLAY_REASON_DEVICE_LOST?"requires a valid recreated device/swapchain":""}};
        diagnostics["nativeMainMenu"] = nlohmann::json::parse(NativeMainMenu::Status());
        diagnostics["firstPerson"] = {
            {"available", KfcRuntimeFirstPersonAvailable()},
            {"detail", FirstPersonDetail()}};
        const auto text = diagnostics.dump();
        if (text.size() >= capacity) { write_error("report-too-large", text.size() + 1); return; }
        std::memcpy(buffer, text.data(), text.size()); buffer[text.size()] = 0;
    } catch (...) { write_error("diagnostics-exception"); }
}

KFC_RUNTIME_API void __cdecl KfcRuntimeOperationMetadata(char* buffer, std::size_t capacity) {
    if (!buffer || !capacity) return;
    buffer[0] = '\0';
    constexpr std::size_t maximum_output_bytes = 64 * 1024;
    constexpr char too_small[] =
        "{\"contractId\":\"kfc-runtime.operation-metadata.v1\",\"resolutionStatus\":\"output-too-small\"}";
    try {
        auto result = WorldRuntime::RegisteredVoxelGrid::MetadataSerialization::ToJson(
            WorldRuntime::RegisteredVoxelGrid::InspectOperationMetadata());
        result["runtimeAbi"] = KFC_RUNTIME_ABI_VERSION;
        result["profile"] = {
            {"id", "not-retained"},
            {"target", SafeImageTarget(KfcRuntimeCompatibility::EnshroudedClient::image_target)},
            {"exactImageMatch", KfcRuntimeCompatibility::EnshroudedClient::exact_build_match},
        };
        result["observedImage"] = {
            {"sha256", SafeImageSha256(KfcRuntimeCompatibility::EnshroudedClient::image_sha256)},
            {"timestamp", KfcRuntimeCompatibility::EnshroudedClient::image_timestamp},
            {"size", KfcRuntimeCompatibility::EnshroudedClient::image_size},
        };
        const auto text = result.dump();
        const auto writable_capacity = (std::min)(capacity, maximum_output_bytes);
        if (text.size() + 1 > writable_capacity) {
            if (sizeof(too_small) <= writable_capacity) {
                std::memcpy(buffer, too_small, sizeof(too_small));
            }
            return;
        }
        std::memcpy(buffer, text.data(), text.size());
        buffer[text.size()] = '\0';
    } catch (...) {
        constexpr char failed[] =
            "{\"contractId\":\"kfc-runtime.operation-metadata.v1\",\"resolutionStatus\":\"unknown\"}";
        if (sizeof(failed) <= (std::min)(capacity, maximum_output_bytes))
            std::memcpy(buffer, failed, sizeof(failed));
    }
}

KFC_RUNTIME_API void __cdecl KfcRuntimeStatus(char* buffer, std::size_t capacity) {
    if (!buffer || !capacity) return;
    try {
        const auto text = EcsRuntime::Status() + " resource_service=" + ResourceRuntime::Status() +
            " native_main_menu=" + NativeMainMenu::Status() +
            " first_person_available=" + (KfcRuntimeFirstPersonAvailable() ? "true" : "false") +
            " first_person_detail=" + FirstPersonDetail();
        const auto size = (std::min)(text.size(), capacity - 1);
        std::memcpy(buffer, text.data(), size); buffer[size] = 0;
    } catch (...) { buffer[0] = 0; }
}
}
