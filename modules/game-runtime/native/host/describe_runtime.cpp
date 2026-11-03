#include "abi/runtime_api.h"
#include "graphics/render_overlay.h"
#include "map/capture_game_map.h"
#include <windows.h>
#include "ecs/entity_session.h"
#include "loaded-resources/loaded_resource_access.h"
#include "main-menu/native_main_menu.h"
#include <algorithm>
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
        diagnostics["nativeMainMenu"] = nlohmann::json::parse(NativeMainMenu::Status());
        diagnostics["firstPerson"] = {
            {"available", KfcRuntimeFirstPersonAvailable()},
            {"detail", FirstPersonDetail()}};
        const auto overlay = OverlayRuntime::GraphicsStatus();
        const auto map = MapRuntime::CapturedFrames().Acquire(0, GetTickCount64());
        diagnostics["mapOverlay"] = {
            {"mapStatus", map.frame.status},
            {"supportedFeatures", map.frame.supported_features},
            {"hudAdmitted", bool(map.frame.supported_features & SF_MAP_FEATURE_HUD)},
            {"discoveryAdmitted", bool(map.frame.supported_features & SF_MAP_FEATURE_DISCOVERY)},
            {"overlayStatus", overlay.status},
            {"overlayReason", overlay.reason},
            {"deviceGeneration", overlay.device_generation},
            {"renderedFrames", overlay.rendered_frames},
            {"busyFrames", overlay.busy_frames},
            {"failedFrames", overlay.failed_frames},
            {"terrainUploadBytes", overlay.terrain_upload_bytes},
            {"terrainUploadRegions", overlay.terrain_upload_regions},
            {"cpuFallbackFrames", overlay.cpu_fallback_frames},
            {"action", overlay.reason == SF_OVERLAY_REASON_AWAIT_OBSERVED_CREATE
                ? "requires an observed swapchain recreation after interception; initial swapchain metadata is unavailable"
                : overlay.reason == SF_OVERLAY_REASON_DEVICE_LOST
                    ? "requires a valid recreated device/swapchain"
                    : ""}
        };
        const auto text = diagnostics.dump();
        if (text.size() >= capacity) { write_error("report-too-large", text.size() + 1); return; }
        std::memcpy(buffer, text.data(), text.size()); buffer[text.size()] = 0;
    } catch (...) { write_error("diagnostics-exception"); }
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
