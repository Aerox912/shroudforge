#pragma once
// The public standalone SDK owns the single canonical notice declaration.
// Host code imports its types without declaring an exported plugin entrypoint.
#pragma push_macro("SF_NATIVE_PLUGIN_EXPORT")
#undef SF_NATIVE_PLUGIN_EXPORT
#define SF_NATIVE_PLUGIN_EXPORT
#include "../../../mod-runtime/native_plugin_api.h"
#pragma pop_macro("SF_NATIVE_PLUGIN_EXPORT")
