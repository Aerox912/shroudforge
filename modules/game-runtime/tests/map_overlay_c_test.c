#include "abi/map_overlay_api.h"
#include <stddef.h>
#include <stdio.h>
_Static_assert(sizeof(SFMapFrameV1)==640,"map frame ABI");
_Static_assert(sizeof(SFMapMarkerV1)==88,"marker ABI");
_Static_assert(offsetof(SFMapMarkerV1,icon_candidates)==48,"candidate ABI");
_Static_assert(sizeof(SFMapServiceV1)==40,"map service ABI");
_Static_assert(sizeof(SFOverlayServiceV1)==40,"overlay service ABI");
_Static_assert(sizeof(SFOverlayEncoderV1)==56,"encoder ABI");
_Static_assert(sizeof(SFOverlayFrameV1)==32,"render frame ABI");
_Static_assert(sizeof(SFOverlayStatusV1)==72,"status ABI");
_Static_assert(offsetof(SFOverlayStatusV1,reason)==12,"reason ABI");
int main(void){SFOverlayStatusV1 status={0};status.reason=SF_OVERLAY_REASON_AWAIT_OBSERVED_CREATE;
    if(status.reason!=1 || SF_MAP_UNSUPPORTED_BUILD!=2)return 1;
    puts("PASS: C consumer map/overlay ABI sizes, candidate and status offsets");return 0;}
