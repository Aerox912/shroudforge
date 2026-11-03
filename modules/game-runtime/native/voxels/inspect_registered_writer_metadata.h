#pragma once

#include <array>
#include <cstddef>
#include <cstdint>
#include <nlohmann/json.hpp>

namespace WorldRuntime::RegisteredVoxelGrid {
enum class OperationMetadataStatus : std::uint8_t {
    Unknown,
    Busy,
    Stale,
    Unsupported,
    DecodeFailure,
    Partial,
    Found,
};

enum class WriterMetadataStatus : std::uint8_t {
    NotInspected,
    Busy,
    UnsupportedArity,
    DecodeFailure,
    Stale,
    Found,
};

inline constexpr std::size_t maximum_metadata_path = 7;

struct OperationMetadataPathStep {
    std::uint32_t rva{};
    std::array<char, 65> code_sha256{};
};

// This is an allowlisted, pointer-free snapshot. Runtime code addresses and
// borrowed Scene/service data stay inside the implementation.
struct OperationMetadata {
    OperationMetadataStatus status{OperationMetadataStatus::Unknown};
    WriterMetadataStatus writer_status{WriterMetadataStatus::NotInspected};
    bool operation_found{};
    std::uint8_t argument_count{};
    std::array<char, 32> operation_name{};
    std::uint32_t image_size{};
    std::uint64_t session_generation{};
    std::uint32_t callback_rva{};
    std::uint32_t writer_candidate_rva{};
    std::array<OperationMetadataPathStep, maximum_metadata_path> writer_path{};
    std::size_t writer_path_count{};
};

OperationMetadata InspectOperationMetadata();

namespace MetadataSerialization {
inline const char* status_name(OperationMetadataStatus status) {
    switch (status) {
    case OperationMetadataStatus::Busy: return "busy";
    case OperationMetadataStatus::Stale: return "stale";
    case OperationMetadataStatus::Unsupported: return "unsupported";
    case OperationMetadataStatus::DecodeFailure: return "decode-failure";
    case OperationMetadataStatus::Partial: return "partial";
    case OperationMetadataStatus::Found: return "found";
    case OperationMetadataStatus::Unknown: default: return "unknown";
    }
}

inline const char* writer_status_name(WriterMetadataStatus status) {
    switch (status) {
    case WriterMetadataStatus::Busy: return "busy";
    case WriterMetadataStatus::UnsupportedArity: return "unsupported-arity";
    case WriterMetadataStatus::DecodeFailure: return "decode-failure";
    case WriterMetadataStatus::Stale: return "stale";
    case WriterMetadataStatus::Found: return "found";
    case WriterMetadataStatus::NotInspected: default: return "not-inspected";
    }
}

inline bool is_terraforming_name(const std::array<char, 32>& name) {
    constexpr char expected[] = "terraforming";
    for (std::size_t i = 0; i < sizeof(expected); ++i)
        if (name[i] != expected[i]) return false;
    return true;
}

inline bool is_sha256(const std::array<char, 65>& digest) {
    for (std::size_t i = 0; i < 64; ++i)
        if (!((digest[i] >= '0' && digest[i] <= '9') || (digest[i] >= 'a' && digest[i] <= 'f')))
            return false;
    return digest[64] == '\0';
}

inline nlohmann::json ToJson(const OperationMetadata& metadata) {
    nlohmann::json result{
        {"contractId", "kfc-runtime.operation-metadata.v1"},
        {"resolutionStatus", status_name(metadata.status)},
    };

    const bool resolved = metadata.status == OperationMetadataStatus::Found ||
        metadata.status == OperationMetadataStatus::Partial;
    if (!resolved) return result;
    if (!metadata.operation_found || !metadata.session_generation || !metadata.image_size ||
        metadata.callback_rva >= metadata.image_size || !is_terraforming_name(metadata.operation_name)) {
        result["resolutionStatus"] = "unknown";
        return result;
    }

    result["sessionGeneration"] = metadata.session_generation;
    result["operation"] = {
        {"name", "terraforming"},
        {"argumentCount", metadata.argument_count},
        {"parameterSchema", "unknown"},
        {"callbackRva", metadata.callback_rva},
    };
    result["writerStatus"] = writer_status_name(metadata.writer_status);

    if (metadata.writer_status != WriterMetadataStatus::Found) return result;
    if (!metadata.writer_candidate_rva || metadata.writer_candidate_rva >= metadata.image_size ||
        !metadata.writer_path_count || metadata.writer_path_count > maximum_metadata_path) {
        result["resolutionStatus"] = "partial";
        result["writerStatus"] = "invalid-metadata";
        return result;
    }

    nlohmann::json path = nlohmann::json::array();
    bool valid_path = metadata.writer_path[0].rva == metadata.callback_rva;
    for (std::size_t i = 0; valid_path && i < metadata.writer_path_count; ++i) {
        const auto& step = metadata.writer_path[i];
        valid_path = step.rva < metadata.image_size && is_sha256(step.code_sha256);
        for (std::size_t previous = 0; valid_path && previous < i; ++previous)
            valid_path = metadata.writer_path[previous].rva != step.rva;
        if (valid_path) path.push_back({{"rva", step.rva}, {"codeSha256", step.code_sha256.data()}});
    }
    valid_path = valid_path && metadata.writer_path[metadata.writer_path_count - 1].rva ==
        metadata.writer_candidate_rva;
    if (!valid_path) {
        result["resolutionStatus"] = "partial";
        result["writerStatus"] = "invalid-metadata";
        return result;
    }
    result["writerCandidateRva"] = metadata.writer_candidate_rva;
    result["writerPath"] = std::move(path);
    return result;
}
}
}
