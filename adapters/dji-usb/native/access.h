#ifndef PILOTAGE_DJI_ACCESS_H
#define PILOTAGE_DJI_ACCESS_H
#include "logiclink.h"
#include <IOKit/OSMessageNotification.h>

// Registry state uses both CFBoolean and CFNumber. USB descriptors keep
// their separate numeric-only validation.
static inline BOOL dji_access_value(id value, uint64_t expected) {
    return [value isKindOfClass:NSNumber.class]
        && [value unsignedLongLongValue] == expected;
}

typedef struct {
    uint64_t port, controller, interface;
    char connection[37];
} DjiAccessIdentity;

typedef enum {
    DjiAccessWaiting = 0, DjiAccessUnauthorized = 1, DjiAccessAmbiguous = 2,
    DjiAccessHandshake = 3, DjiAccessReady = 4, DjiAccessReconnect = 5,
    DjiAccessBlocked = 6
} DjiAccessStatus;

typedef struct DjiAccess {
    DjiAccessIdentity identity;
    char attempted[37];
    io_service_t port, interface;
    io_connect_t client;
    IONotificationPortRef notifications;
    CFRunLoopSourceRef source;
    double deadline, cycleAt, handoffAt, permitUntil;
    uint32_t status, setupCount;
    BOOL groundPermit, canCycle, active, cycled, acknowledged, failed, fenced;
} DjiAccess;

DjiAccessStatus dji_access_discover(DjiAccessIdentity *identity);
BOOL dji_access_port_guard(const DjiAccessIdentity *identity, BOOL hostPhase);
BOOL dji_access_controller_guard(const DjiAccessIdentity *identity, BOOL early);
BOOL dji_access_interface_guard(const DjiAccessIdentity *identity, io_service_t service);
BOOL dji_access_owner_guard(io_service_t service);
int dji_access_device_state(void);
BOOL dji_access_role(DjiAccess *access, uint64_t mode);
int dji_access_setup(const uint64_t words[5], uint32_t count);
int dji_access_test(void);
void dji_access_init(DjiAccess *access, BOOL groundPermit);
void dji_access_update(DjiAccess *access, BOOL changed, BOOL permitted);
void dji_access_close(DjiAccess *access);
#endif
