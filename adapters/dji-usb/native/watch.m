#import "watch.h"

struct DjiWatch {
    IONotificationPortRef port;
    CFRunLoopSourceRef source;
    io_iterator_t iterators[4];
    BOOL changed;
};

static void changed(void *context, io_iterator_t iterator) {
    DjiWatch *watch = context;
    io_service_t service;
    while ((service = IOIteratorNext(iterator)) != IO_OBJECT_NULL) {
        watch->changed = YES;
        IOObjectRelease(service);
    }
}

void dji_watch_close(DjiWatch *watch) {
    if (watch == NULL) return;
    if (watch->source != NULL)
        CFRunLoopRemoveSource(CFRunLoopGetCurrent(), watch->source, kCFRunLoopDefaultMode);
    for (unsigned i = 0; i < 4; ++i)
        if (watch->iterators[i] != IO_OBJECT_NULL) IOObjectRelease(watch->iterators[i]);
    if (watch->port != NULL) IONotificationPortDestroy(watch->port);
    free(watch);
}

int dji_watch_open(DjiWatch **output, DjiError *error) {
    *output = NULL;
    DjiWatch *watch = calloc(1, sizeof(*watch));
    if (watch == NULL) return dji_fail(error, 24, kIOReturnNoMemory);
    watch->port = IONotificationPortCreate(kIOMainPortDefault);
    watch->source = watch->port == NULL ? NULL : IONotificationPortGetRunLoopSource(watch->port);
    if (watch->source == NULL) {
        dji_watch_close(watch);
        return dji_fail(error, 24, kIOReturnNoResources);
    }
    CFRunLoopAddSource(CFRunLoopGetCurrent(), watch->source, kCFRunLoopDefaultMode);
    const char *classes[] = {"IOUSBHostDevice", "IOUSBHostInterface"};
    const char *events[] = {kIOFirstMatchNotification, kIOTerminatedNotification};
    for (unsigned i = 0; i < 4; ++i) {
        CFMutableDictionaryRef match = IOServiceMatching(classes[i / 2]);
        if (match == NULL) {
            dji_watch_close(watch);
            return dji_fail(error, 24, kIOReturnNoMemory);
        }
        // Interface arrival matters: device publication can precede both interfaces.
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

int dji_watch_poll(DjiWatch *watch) {
    // The run loop waits for events; the bound permits owner shutdown.
    if (!watch->changed) CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, true);
    BOOL result = watch->changed;
    watch->changed = NO;
    return result ? 1 : 0;
}
