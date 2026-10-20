#import "logiclink.h"
#import <IOKit/IOCFPlugIn.h>
#import <IOKit/usb/IOUSBLib.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

void dji_read_complete(void *context, IOReturn status, void *count) {
    DjiUsb *usb = context;
    usb->readStatus = status;
    usb->readSize = (uintptr_t)count;
    usb->pending = NO;
    usb->ready = YES;
}

static BOOL sameAttachment(DjiUsb *usb) {
    io_registry_entry_t parent = IO_OBJECT_NULL;
    uint64_t identity = 0;
    IOReturn status = IORegistryEntryGetParentEntry(usb->service, kIOServicePlane, &parent);
    BOOL valid = status == KERN_SUCCESS
        && IORegistryEntryGetRegistryEntryID(parent, &identity) == KERN_SUCCESS
        && identity == usb->deviceID;
    if (parent != IO_OBJECT_NULL) IOObjectRelease(parent);
    UInt8 alternate = 255;
    return valid && (*usb->interface)->GetAlternateSetting(usb->interface, &alternate)
        == kIOReturnSuccess && alternate == 1;
}

static BOOL selectPipes(DjiUsb *usb, DjiProfile profile, DjiError *error) {
    UInt8 count = 0;
    IOReturn status = (*usb->interface)->GetNumEndpoints(usb->interface, &count);
    if (status != kIOReturnSuccess || count != 2) {
        dji_fail(error, 9, status == kIOReturnSuccess ? kIOReturnBadArgument : status);
        return NO;
    }
    for (UInt8 pipe = 1; pipe <= count; pipe++) {
        UInt8 direction = 0, number = 0, transfer = 0, interval = 0;
        UInt16 packet = 0;
        status = (*usb->interface)->GetPipeProperties(usb->interface, pipe, &direction,
            &number, &transfer, &packet, &interval);
        UInt8 address = number | (direction == kUSBIn ? 0x80 : 0);
        if (status != kIOReturnSuccess || transfer != kUSBBulk) {
            dji_fail(error, 9, status == kIOReturnSuccess ? kIOReturnBadArgument : status);
            return NO;
        }
        if (direction == kUSBIn && address == profile.inputAddress
            && packet == profile.inputPacketSize && usb->inputPipe == 0) {
            usb->inputPipe = pipe;
        } else if (direction == kUSBOut && address == profile.outputAddress
            && packet == profile.outputPacketSize && usb->outputPipe == 0) {
            usb->outputPipe = pipe;
        } else {
            dji_fail(error, 9, kIOReturnBadArgument);
            return NO;
        }
    }
    return usb->inputPipe != 0 && usb->outputPipe != 0;
}

static BOOL openInterface(DjiUsb *usb, DjiProfile profile, DjiError *error) {
    IOCFPlugInInterface **plugin = NULL;
    SInt32 score = 0;
    IOReturn status = IOCreatePlugInInterfaceForService(usb->service,
        kIOUSBInterfaceUserClientTypeID, kIOCFPlugInInterfaceID, &plugin, &score);
    if (status != kIOReturnSuccess || plugin == NULL) {
        if (plugin != NULL) (*plugin)->Release(plugin);
        dji_fail(error, 6, status);
        return NO;
    }
    HRESULT queried = (*plugin)->QueryInterface(plugin,
        CFUUIDGetUUIDBytes(kIOUSBInterfaceInterfaceID650), (LPVOID *)&usb->interface);
    (*plugin)->Release(plugin);
    if (queried != 0 || usb->interface == NULL) {
        dji_fail(error, 6, (uint32_t)queried);
        return NO;
    }
    status = (*usb->interface)->USBInterfaceOpen(usb->interface);
    usb->opened = status == kIOReturnSuccess;
    if (!usb->opened) { dji_fail(error, 7, status); return NO; }
    status = (*usb->interface)->GetAlternateSetting(usb->interface, &usb->original);
    usb->originalKnown = status == kIOReturnSuccess && usb->original <= 1;
    if (!usb->originalKnown) {
        dji_fail(error, 8, status == kIOReturnSuccess ? kIOReturnBadArgument : status);
        return NO;
    }
    if (usb->original != 1) {
        usb->changed = YES;
        status = (*usb->interface)->SetAlternateInterface(usb->interface, 1);
        if (status != kIOReturnSuccess) { dji_fail(error, 8, status); return NO; }
    }
    if (!sameAttachment(usb)) { dji_fail(error, 8, kIOReturnNoDevice); return NO; }
    if (!selectPipes(usb, profile, error)) return NO;
    status = (*usb->interface)->CreateInterfaceAsyncEventSource(usb->interface, &usb->source);
    if (status != kIOReturnSuccess || usb->source == NULL) {
        dji_fail(error, 10, status);
        return NO;
    }
    CFRunLoopAddSource(CFRunLoopGetCurrent(), usb->source, kCFRunLoopDefaultMode);
    return YES;
}

int dji_usb_open(uint64_t identity, DjiUsb **output, DjiError *error) {
    @autoreleasepool {
        *output = NULL;
        DjiUsb *usb = calloc(1, sizeof(DjiUsb));
        if (usb == NULL) return dji_fail(error, 1, kIOReturnNoMemory);
        usb->deviceID = identity;
        io_service_t device = identity == 0 ? IO_OBJECT_NULL :
            IOServiceGetMatchingService(kIOMainPortDefault, IORegistryEntryIDMatching(identity));
        NSDictionary *p = device == IO_OBJECT_NULL ? nil : dji_properties(device);
        uint64_t actual = 0;
        BOOL valid = device != IO_OBJECT_NULL && IOObjectConformsTo(device, "IOUSBHostDevice")
            && IORegistryEntryGetRegistryEntryID(device, &actual) == KERN_SUCCESS && actual == identity
            && dji_number(p[@"idVendor"], 0x2ca3) && dji_number(p[@"idProduct"], 0x1020)
            && dji_number(p[@"bNumConfigurations"], 1) && dji_number(p[@"kUSBCurrentConfiguration"], 1)
            && dji_number(p[@"bDeviceClass"], 239) && dji_number(p[@"bDeviceSubClass"], 2)
            && dji_number(p[@"bDeviceProtocol"], 1);
        DjiProfile profile = {0};
        if (!valid) dji_fail(error, 1, kIOReturnNoDevice);
        if (valid) valid = dji_read_profile(device, &profile, error);
        if (valid) {
            usb->service = dji_logic_service(device, error);
            valid = usb->service != IO_OBJECT_NULL;
        }
        if (device != IO_OBJECT_NULL) IOObjectRelease(device);
        if (valid) valid = openInterface(usb, profile, error);
        if (!valid) {
            DjiError cleanup = {0};
            if (dji_usb_close(usb, &cleanup) != 0) *error = cleanup;
            return -1;
        }
        *output = usb;
        return 0;
    }
}

int dji_usb_poll(DjiUsb *usb, uint8_t *output, uint32_t capacity,
    uint32_t *count, DjiError *error) {
    @autoreleasepool {
        *count = 0;
        if (capacity < sizeof(usb->bytes)) return dji_fail(error, 11, kIOReturnBadArgument);
        if (!usb->pending && !usb->ready) {
            usb->pending = YES;
            IOReturn status = (*usb->interface)->ReadPipeAsyncTO(usb->interface,
                usb->inputPipe, usb->bytes, sizeof(usb->bytes), 0, 0, dji_read_complete, usb);
            if (status != kIOReturnSuccess) {
                usb->pending = NO;
                return dji_fail(error, 11, status);
            }
        }
        if (!usb->ready) CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.002, true);
        if (!usb->ready) return 1;
        usb->ready = NO;
        if (usb->readStatus != kIOReturnSuccess)
            return dji_fail(error, 12, usb->readStatus);
        if (usb->readSize > sizeof(usb->bytes))
            return dji_fail(error, 12, kIOReturnOverrun);
        memcpy(output, usb->bytes, usb->readSize);
        *count = (uint32_t)usb->readSize;
        return 0;
    }
}

uint64_t dji_usb_monotonic_ns(void) {
    return clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
}

int dji_write_pipe(DjiUsb *usb, const uint8_t *bytes, uint32_t count,
    uint64_t deadline_ns, DjiError *error) {
    uint64_t now = dji_usb_monotonic_ns();
    uint64_t remaining_ms = deadline_ns > now ? (deadline_ns - now) / 1000000 : 0;
    if (remaining_ms == 0) return dji_fail(error, 13, kIOReturnTimeout);
    // IOKit accepts whole milliseconds. Rounding down preserves the caller's deadline.
    uint32_t timeout = remaining_ms < 50 ? (uint32_t)remaining_ms : 50;
    IOReturn status = (*usb->interface)->WritePipeTO(usb->interface,
        usb->outputPipe, (void *)bytes, count, timeout, timeout);
    return status == kIOReturnSuccess ? 0 : dji_fail(error, 14, status);
}

int dji_usb_write(DjiUsb *usb, const uint8_t *bytes, uint32_t count,
    uint64_t deadline_ns, DjiError *error) {
    @autoreleasepool {
        if (count == 0 || count > 16384)
            return dji_fail(error, 13, kIOReturnBadArgument);
        if (deadline_ns <= dji_usb_monotonic_ns())
            return dji_fail(error, 13, kIOReturnTimeout);
        if (!sameAttachment(usb)) return dji_fail(error, 13, kIOReturnNoDevice);
        return dji_write_pipe(usb, bytes, count, deadline_ns, error);
    }
}

int dji_usb_close(DjiUsb *usb, DjiError *error) {
    @autoreleasepool {
        if (usb->pending) {
            IOReturn status = (*usb->interface)->AbortPipe(usb->interface, usb->inputPipe);
            double until = NSProcessInfo.processInfo.systemUptime + 0.25;
            while (usb->pending && NSProcessInfo.processInfo.systemUptime < until)
                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.002, true);
            // A callback retains this context until cancellation completes.
            // Keep it alive if completion is unknown; release would permit UAF.
            if (usb->pending) return dji_fail(error, 15,
                status == kIOReturnSuccess ? kIOReturnTimeout : status);
        }
        BOOL ok = YES;
        if (usb->opened && usb->changed) {
            IOReturn status = (*usb->interface)->SetAlternateInterface(usb->interface, usb->original);
            if (status != kIOReturnSuccess) { dji_fail(error, 16, status); ok = NO; }
        }
        if (usb->opened && usb->originalKnown) {
            UInt8 actual = 255;
            IOReturn status = (*usb->interface)->GetAlternateSetting(usb->interface, &actual);
            if (status != kIOReturnSuccess || actual != usb->original) {
                dji_fail(error, 16, status == kIOReturnSuccess ? kIOReturnBadArgument : status);
                ok = NO;
            }
        }
        if (usb->opened) {
            IOReturn status = (*usb->interface)->USBInterfaceClose(usb->interface);
            if (status != kIOReturnSuccess) { dji_fail(error, 17, status); ok = NO; }
        }
        if (usb->source != NULL) {
            CFRunLoopRemoveSource(CFRunLoopGetCurrent(), usb->source, kCFRunLoopDefaultMode);
            CFRelease(usb->source);
        }
        if (usb->interface != NULL) (*usb->interface)->Release(usb->interface);
        if (usb->service != IO_OBJECT_NULL) IOObjectRelease(usb->service);
        free(usb);
        return ok ? 0 : -1;
    }
}
