#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "../native_plugin_api.h"

#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(SFNativePluginV1) == 48, "plugin ABI v1 size drift");
_Static_assert(offsetof(SFNativePluginV1, initialize) == 16, "plugin callback offset drift");
_Static_assert(sizeof(SFNativeHostV1) == 96, "host ABI v1 size drift");
_Static_assert(sizeof(SFNoticeServiceV1) == 24, "notice service v1 size drift");
_Static_assert(offsetof(SFNoticeServiceV1, show) == 16, "notice callback offset drift");
_Static_assert(sizeof(SFModNetworkStatusV1) == 40, "network status v1 size drift");
_Static_assert(offsetof(SFModNetworkStatusV1, local_steam_id) == 24, "network status ID offset drift");
_Static_assert(sizeof(SFModNetworkMessageV1) == 32, "network message v1 size drift");
_Static_assert(offsetof(SFModNetworkMessageV1, reply_token) == 24, "network reply token offset drift");
_Static_assert(sizeof(SFModNetworkServiceV1) == 56, "network service v1 size drift");
_Static_assert(offsetof(SFModNetworkServiceV1, read_status) == 24, "network callback offset drift");
_Static_assert(offsetof(SFModNetworkServiceV1, reply) == 48, "network reply callback offset drift");
_Static_assert(offsetof(SFNativeHostV1, package_generation) == 24, "host generation offset drift");
_Static_assert(offsetof(SFNativeHostV1, read_asset) == 64, "host asset callback offset drift");
_Static_assert(offsetof(SFNativeHostV1, drain_services) == 88, "host drain callback offset drift");
#endif

static SFNativeStatusV1 SF_NATIVE_CALL plugin_initialize(
    void *plugin_context,
    const SFNativeHostV1 *host)
{
    (void)plugin_context;
    return host != NULL && host->abi_version == SF_NATIVE_HOST_ABI_V1
        ? SF_NATIVE_STATUS_OK
        : SF_NATIVE_STATUS_INVALID_ARGUMENT;
}

static SFNativeStatusV1 SF_NATIVE_CALL plugin_update(
    void *plugin_context,
    const SFNativeHostV1 *host,
    uint64_t elapsed_microseconds)
{
    (void)plugin_context;
    (void)elapsed_microseconds;
    return host != NULL ? SF_NATIVE_STATUS_OK : SF_NATIVE_STATUS_INVALID_ARGUMENT;
}

static SFNativeStatusV1 SF_NATIVE_CALL plugin_stop(
    void *plugin_context,
    const SFNativeHostV1 *host)
{
    (void)plugin_context;
    return host != NULL ? SF_NATIVE_STATUS_OK : SF_NATIVE_STATUS_INVALID_ARGUMENT;
}

static SFNativeStatusV1 SF_NATIVE_CALL plugin_shutdown(
    void *plugin_context,
    const SFNativeHostV1 *host)
{
    (void)plugin_context;
    return host != NULL ? SF_NATIVE_STATUS_OK : SF_NATIVE_STATUS_INVALID_ARGUMENT;
}

SFNativeStatusV1 SF_NATIVE_CALL ShroudForgePluginEntry(
    uint32_t requested_abi,
    uint32_t out_size,
    SFNativePluginV1 *out_plugin)
{
    if (requested_abi != SF_NATIVE_PLUGIN_ABI_V1
        || out_plugin == NULL
        || out_size < sizeof(SFNativePluginV1)) {
        return SF_NATIVE_STATUS_ABI_MISMATCH;
    }

    const SFNativePluginV1 table = {
        (uint32_t)sizeof(SFNativePluginV1),
        SF_NATIVE_PLUGIN_ABI_V1,
        NULL,
        plugin_initialize,
        plugin_update,
        plugin_stop,
        plugin_shutdown,
    };
    memcpy(out_plugin, &table, sizeof(table));
    return SF_NATIVE_STATUS_OK;
}

int main(void)
{
    struct GuardedPluginOutput {
        SFNativePluginV1 plugin;
        uint64_t sentinel;
    } guarded;
    memset(&guarded, 0xA5, sizeof(guarded));
    guarded.sentinel = UINT64_C(0xA55A5AA5C33C3CC3);

    SFNativeStatusV1 status = ShroudForgePluginEntry(
        SF_NATIVE_PLUGIN_ABI_V1,
        (uint32_t)sizeof(SFNativePluginV1) - 1u,
        &guarded.plugin);
    if (status != SF_NATIVE_STATUS_ABI_MISMATCH
        || guarded.plugin.struct_size != UINT32_C(0xA5A5A5A5)
        || guarded.sentinel != UINT64_C(0xA55A5AA5C33C3CC3)) {
        return 1;
    }

    SFNativePluginV1 plugin = {0};
    status = ShroudForgePluginEntry(
        SF_NATIVE_PLUGIN_ABI_V1,
        (uint32_t)sizeof(plugin),
        &plugin);
    if (status != SF_NATIVE_STATUS_OK
        || plugin.struct_size != sizeof(plugin)
        || plugin.abi_version != SF_NATIVE_PLUGIN_ABI_V1
        || plugin.initialize == NULL
        || plugin.update == NULL
        || plugin.stop == NULL
        || plugin.shutdown == NULL) {
        return 2;
    }

    SFNativeHostV1 host = {0};
    host.abi_version = SF_NATIVE_HOST_ABI_V1;
    if (plugin.initialize(plugin.plugin_context, &host) != SF_NATIVE_STATUS_OK) {
        return 3;
    }
    return 0;
}
