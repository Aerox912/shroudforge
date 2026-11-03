#pragma once
#include <cstdint>
#include <vector>

namespace OverlayRuntime {
inline constexpr std::uint32_t NoticeWidth=640, NoticeHeight=72;
// Event-time only. No font file or GDI object survives this call.
std::int32_t RasterizeNotice(const std::uint8_t*,std::uint32_t,std::vector<std::uint8_t>&);
}
