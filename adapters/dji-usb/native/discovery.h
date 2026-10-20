#ifndef PILOTAGE_DJI_DISCOVERY_H
#define PILOTAGE_DJI_DISCOVERY_H
#import "logiclink.h"

typedef struct {
    uint64_t registryID;
    uint32_t locationID;
    uint32_t reserved;
} DjiCandidate;

int dji_usb_discover(DjiCandidate *output, uint32_t capacity,
    uint32_t *count, DjiError *error);
int dji_usb_discovery_test(void);

#ifdef __OBJC__
BOOL dji_discovery_profile(NSDictionary *device, NSArray<NSDictionary *> *interfaces);
#endif
#endif
