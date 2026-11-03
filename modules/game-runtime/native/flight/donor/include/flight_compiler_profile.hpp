#pragma once

#ifndef SHROUDWEAVE_FLIGHT_MSC_FULL_VER
#error Select the explicit reviewed Flight compiler profile through CMake.
#endif
static_assert(SHROUDWEAVE_FLIGHT_MSC_FULL_VER == 194435228 ||
              SHROUDWEAVE_FLIGHT_MSC_FULL_VER == 194435229,
              "Unsupported Flight compiler profile.");
static_assert(_MSC_FULL_VER == SHROUDWEAVE_FLIGHT_MSC_FULL_VER,
              "Compiler differs from the explicitly selected Flight profile.");
