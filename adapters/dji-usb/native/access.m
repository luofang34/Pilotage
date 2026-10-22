#import "access.h"
#include <string.h>

static double now(void) { return NSProcessInfo.processInfo.systemUptime; }

void dji_access_init(DjiAccess *access, BOOL groundPermit) {
    memset(access, 0, sizeof(*access));
    access->groundPermit = groundPermit;
    access->permitUntil = now() + 5.0;
}

void dji_access_close(DjiAccess *access) {
    access->active = NO;
    if (access->client != IO_OBJECT_NULL) {
        if (IOServiceClose(access->client) != KERN_SUCCESS) access->fenced = YES;
        access->client = IO_OBJECT_NULL;
    }
    if (access->source != NULL) {
        CFRunLoopRemoveSource(CFRunLoopGetCurrent(), access->source, kCFRunLoopDefaultMode);
        access->source = NULL;
    }
    if (access->notifications != NULL) {
        IONotificationPortDestroy(access->notifications);
        access->notifications = NULL;
    }
    if (access->interface != IO_OBJECT_NULL) {
        IOObjectRelease(access->interface);
        access->interface = IO_OBJECT_NULL;
    }
    if (access->port != IO_OBJECT_NULL) {
        IOObjectRelease(access->port);
        access->port = IO_OBJECT_NULL;
    }
    if (access->fenced) access->status = DjiAccessBlocked;
}

int dji_access_setup(const uint64_t words[5], uint32_t count) {
    if (count != 5 || words[0] != 0 || words[1] > UINT32_MAX
        || words[2] > UINT32_MAX || words[3] > UINT16_MAX
        || (words[2] & UINT16_MAX) != words[3]) return -1;
    return words[1] == 0x40510000 && words[2] == 0 && words[3] == 0 ? 1 : 0;
}

int dji_access_test(void) {
    uint64_t exact[5] = {0, 0x40510000, 0, 0, 7};
    uint64_t other[5] = {0, 0x40510001, 0, 0, 7};
    uint64_t malformed[5] = {0, 0x140510000, 0, 0, 7};
    uint64_t mismatch[5] = {0, 0x40510000, 0, 1, 7};
    return !(dji_access_value(@YES, 1) && dji_access_value(@NO, 0)
        && dji_access_value(@2, 2) && !dji_access_value(@YES, 2)
        && dji_access_setup(exact, 5) == 1
        && dji_access_setup(other, 5) == 0
        && dji_access_setup(malformed, 5) == -1
        && dji_access_setup(mismatch, 5) == -1
        && dji_access_setup(exact, 4) == -1);
}

static void request(void *reference, IOReturn result, void **arguments, uint32_t count) {
    DjiAccess *access = reference;
    if (!access->active || access->failed || access->acknowledged) return;
    if (arguments == NULL || result != KERN_SUCCESS || count == 0 || count > 5) {
        access->failed = YES;
        return;
    }
    uint64_t words[5] = {0};
    for (uint32_t i = 0; i < count; ++i) words[i] = (uintptr_t)arguments[i];
    if (words[0] == 1 || words[0] == 2) return;
    int decision = dji_access_setup(words, count);
    if (decision < 0 || access->setupCount >= 32
        || now() >= access->deadline
        || !dji_access_port_guard(&access->identity, NO)
        || !dji_access_controller_guard(&access->identity, NO)) {
        access->failed = YES;
        return;
    }
    access->setupCount++;
    BOOL acknowledge = decision == 1;
    uint64_t reply[5] = {acknowledge ? 1 : 0, 0, 0, 0, words[4]};
    IOReturn completed = IOConnectCallScalarMethod(access->client, 9, reply, 5, NULL, NULL);
    if (completed != KERN_SUCCESS) access->failed = YES;
    if (acknowledge && completed == KERN_SUCCESS) {
        access->acknowledged = YES;
        // The RC must finish the status phase before the Mac requests host role.
        access->handoffAt = now() + 0.05;
    }
}

static BOOL begin(DjiAccess *access) {
    access->port = IOServiceGetMatchingService(kIOMainPortDefault,
        IORegistryEntryIDMatching(access->identity.port));
    access->interface = IOServiceGetMatchingService(kIOMainPortDefault,
        IORegistryEntryIDMatching(access->identity.interface));
    if (access->port == IO_OBJECT_NULL || access->interface == IO_OBJECT_NULL
        || !dji_access_port_guard(&access->identity, NO)
        || !dji_access_controller_guard(&access->identity, YES)
        || !dji_access_interface_guard(&access->identity, access->interface)
        || !dji_access_owner_guard(access->interface)) return NO;
    access->notifications = IONotificationPortCreate(kIOMainPortDefault);
    access->source = access->notifications == NULL ? NULL
        : IONotificationPortGetRunLoopSource(access->notifications);
    if (access->source == NULL) return NO;
    CFRunLoopAddSource(CFRunLoopGetCurrent(), access->source, kCFRunLoopDefaultMode);
    if (IOServiceOpen(access->interface, mach_task_self(), 123, &access->client) != KERN_SUCCESS)
        return NO;
    if (!dji_access_port_guard(&access->identity, NO)
        || !dji_access_controller_guard(&access->identity, YES)
        || !dji_access_interface_guard(&access->identity, access->interface)) return NO;
    io_async_ref64_t reference = {0};
    reference[kIOAsyncCalloutFuncIndex] = (io_user_reference_t)request;
    reference[kIOAsyncCalloutRefconIndex] = (io_user_reference_t)access;
    uint64_t flags[3] = {1, 0, 0};
    IOReturn status = IOConnectCallAsyncScalarMethod(access->client, 8,
        IONotificationPortGetMachPort(access->notifications), reference, 3, flags, 3, NULL, NULL);
    // A busy registration can leave provider cleanup ambiguous. Do not retry it.
    if (status != KERN_SUCCESS) { access->fenced = YES; return NO; }
    access->active = YES;
    access->deadline = now() + 12.0;
    access->cycleAt = now() + 0.25;
    access->handoffAt = 0;
    access->setupCount = 0;
    access->cycled = access->acknowledged = access->failed = NO;
    return YES;
}

static void advance(DjiAccess *access, BOOL permitted) {
    if (!permitted || access->failed || now() >= access->deadline
        || !dji_access_port_guard(&access->identity, NO)) {
        access->status = DjiAccessReconnect;
        dji_access_close(access);
        return;
    }
    if (access->acknowledged && now() >= access->handoffAt) {
        BOOL handed = dji_access_role(access, 2);
        access->status = handed ? DjiAccessHandshake : DjiAccessReconnect;
        dji_access_close(access);
        return;
    }
    if (!access->acknowledged && !access->cycled && access->canCycle
        && now() >= access->cycleAt && dji_access_controller_guard(&access->identity, NO)) {
        access->canCycle = NO;
        access->cycled = YES;
        // One cycle only. The request callback is already armed and remains alive
        // across the same physical connection's temporary host phase.
        if (dji_access_device_state() != 0 || !dji_access_role(access, 2)
            || !dji_access_role(access, 5)) {
            access->status = DjiAccessReconnect;
            dji_access_close(access);
        }
    }
}

void dji_access_update(DjiAccess *access, BOOL changed, BOOL permitted) {
    if (access->fenced) { access->status = DjiAccessBlocked; return; }
    if (access->active) { advance(access, permitted); return; }
    if (!changed) return;
    DjiAccessIdentity identity;
    DjiAccessStatus status = dji_access_discover(&identity);
    if (status == DjiAccessUnauthorized) access->attempted[0] = 0;
    if (status != DjiAccessHandshake) {
        access->groundPermit = NO;
        access->status = status;
        return;
    }
    if (!permitted || strcmp(access->attempted, identity.connection) == 0) return;
    access->identity = identity;
    if (!dji_access_controller_guard(&identity, YES)) return;
    strlcpy(access->attempted, identity.connection, sizeof(access->attempted));
    access->status = DjiAccessHandshake;
    access->canCycle = access->groundPermit && now() < access->permitUntil;
    access->groundPermit = NO;
    if (!begin(access)) {
        access->status = DjiAccessReconnect;
        dji_access_close(access);
    }
}
