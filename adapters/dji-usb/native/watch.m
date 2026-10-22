#import "watch.h"
#import "access.h"
#include <fcntl.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <unistd.h>

#define WATCH_CLASSES 4
#define WATCH_INTERESTS 32
struct DjiWatch {
    IONotificationPortRef port;
    CFRunLoopSourceRef source;
    io_iterator_t iterators[WATCH_CLASSES * 2];
    io_object_t interests[WATCH_INTERESTS];
    uint64_t interested[WATCH_INTERESTS];
    unsigned interestCount;
    BOOL changed, accessEnabled, permitted, interestFailure;
    DjiAccess access;
    int ownerLock;
};

static void interest(void *context, io_service_t service, uint32_t message, void *argument) {
    (void)service; (void)message; (void)argument;
    DjiWatch *watch = context;
    watch->changed = YES;
}

static void changed(void *context, io_iterator_t iterator) {
    DjiWatch *watch = context;
    io_service_t service;
    while ((service = IOIteratorNext(iterator)) != IO_OBJECT_NULL) {
        watch->changed = YES;
        if (watch->accessEnabled && (IOObjectConformsTo(service, "AppleHPMInterfaceType10")
            || IOObjectConformsTo(service, "IOUSBDeviceController"))) {
            uint64_t id = 0;
            BOOL seen = IORegistryEntryGetRegistryEntryID(service, &id) != KERN_SUCCESS;
            for (unsigned i = 0; i < watch->interestCount; ++i)
                seen |= watch->interested[i] == id;
            if (!seen) {
                if (watch->interestCount == WATCH_INTERESTS) {
                    watch->interestFailure = YES;
                } else {
                    unsigned slot = watch->interestCount++;
                    watch->interested[slot] = id;
                    IOReturn status = IOServiceAddInterestNotification(watch->port, service,
                        kIOGeneralInterest, interest, watch, &watch->interests[slot]);
                    watch->interestFailure |= status != KERN_SUCCESS;
                }
            }
        }
        IOObjectRelease(service);
    }
}

void dji_watch_close(DjiWatch *watch) {
    if (watch == NULL) return;
    if (watch->accessEnabled) dji_access_close(&watch->access);
    if (watch->source != NULL)
        CFRunLoopRemoveSource(CFRunLoopGetCurrent(), watch->source, kCFRunLoopDefaultMode);
    for (unsigned i = 0; i < WATCH_CLASSES * 2; ++i)
        if (watch->iterators[i] != IO_OBJECT_NULL) IOObjectRelease(watch->iterators[i]);
    for (unsigned i = 0; i < watch->interestCount; ++i)
        if (watch->interests[i] != IO_OBJECT_NULL) IOObjectRelease(watch->interests[i]);
    if (watch->port != NULL) IONotificationPortDestroy(watch->port);
    if (watch->ownerLock >= 0) close(watch->ownerLock);
    free(watch);
}

static int openWatch(BOOL access, BOOL groundRecovery, DjiWatch **output, DjiError *error) {
    *output = NULL;
    DjiWatch *watch = calloc(1, sizeof(*watch));
    if (watch == NULL) return dji_fail(error, 24, kIOReturnNoMemory);
    watch->ownerLock = -1;
    watch->accessEnabled = access;
    if (access) {
        // Hold a process-wide advisory owner across both handshake and data use.
        // Do not unlink: an inode replacement would let two owners coexist.
        char path[128];
        snprintf(path, sizeof(path), "/tmp/pilotage-dji-upper-%u.lock", getuid());
        watch->ownerLock = open(path, O_CREAT | O_RDWR | O_CLOEXEC | O_NOFOLLOW, 0600);
        struct stat info;
        if (watch->ownerLock < 0 || fstat(watch->ownerLock, &info) != 0
            || !S_ISREG(info.st_mode) || info.st_uid != getuid() || info.st_nlink != 1
            || (info.st_mode & 077) != 0 || flock(watch->ownerLock, LOCK_EX | LOCK_NB) != 0) {
            dji_watch_close(watch);
            return dji_fail(error, 24, kIOReturnExclusiveAccess);
        }
    }
    dji_access_init(&watch->access, groundRecovery);
    watch->port = IONotificationPortCreate(kIOMainPortDefault);
    watch->source = watch->port == NULL ? NULL : IONotificationPortGetRunLoopSource(watch->port);
    if (watch->source == NULL) {
        dji_watch_close(watch);
        return dji_fail(error, 24, kIOReturnNoResources);
    }
    CFRunLoopAddSource(CFRunLoopGetCurrent(), watch->source, kCFRunLoopDefaultMode);
    const char *classes[] = {"IOUSBHostDevice", "IOUSBHostInterface",
        "AppleHPMInterfaceType10", "IOUSBDeviceController"};
    const char *events[] = {kIOFirstMatchNotification, kIOTerminatedNotification};
    unsigned count = access ? WATCH_CLASSES * 2 : 4;
    for (unsigned i = 0; i < count; ++i) {
        CFMutableDictionaryRef match = IOServiceMatching(classes[i / 2]);
        if (match == NULL) {
            dji_watch_close(watch);
            return dji_fail(error, 24, kIOReturnNoMemory);
        }
        // Register before draining the initial snapshot so an arrival cannot
        // fall between discovery and subscription.
        IOReturn status = IOServiceAddMatchingNotification(watch->port, events[i % 2],
            match, changed, watch, &watch->iterators[i]);
        if (status != KERN_SUCCESS) {
            dji_watch_close(watch);
            return dji_fail(error, 24, status);
        }
        changed(watch, watch->iterators[i]);
    }
    watch->changed = YES;
    *output = watch;
    return 0;
}

int dji_watch_open(DjiWatch **output, DjiError *error) {
    return openWatch(NO, NO, output, error);
}

int dji_watch_access_open(int groundRecovery, DjiWatch **output, DjiError *error) {
    return openWatch(YES, groundRecovery != 0, output, error);
}

void dji_watch_access_permit(DjiWatch *watch, int permitted) {
    if (!watch->permitted && permitted != 0) watch->changed = YES;
    watch->permitted = permitted != 0;
}

uint32_t dji_watch_access_status(DjiWatch *watch) { return watch->access.status; }

int dji_watch_poll(DjiWatch *watch) {
    @autoreleasepool {
        // Timers only advance a bounded handshake attempt; they never start retries.
        if (!watch->changed) CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, true);
        BOOL result = watch->changed;
        watch->changed = NO;
        if (watch->accessEnabled) {
            uint32_t before = watch->access.status;
            if (watch->interestFailure) watch->access.fenced = YES;
            dji_access_update(&watch->access, result, watch->permitted);
            result |= before != watch->access.status;
        }
        return result ? 1 : 0;
    }
}
