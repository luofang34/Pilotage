#import "discovery.h"

static BOOL device_profile(NSDictionary *p) {
    return dji_number(p[@"idVendor"], 0x2ca3)
        && dji_number(p[@"idProduct"], 0x1020)
        && dji_number(p[@"bDeviceClass"], 239)
        && dji_number(p[@"bDeviceSubClass"], 2)
        && dji_number(p[@"bDeviceProtocol"], 1)
        && dji_number(p[@"bNumConfigurations"], 1)
        && dji_number(p[@"kUSBCurrentConfiguration"], 1)
        && [p[@"USB Product Name"] isEqual:@"C5"];
}

BOOL dji_discovery_profile(NSDictionary *device, NSArray<NSDictionary *> *interfaces) {
    if (!device_profile(device) || interfaces.count != 2) return NO;
    unsigned seen = 0;
    for (NSDictionary *p in interfaces) {
        if (!dji_number(p[@"idVendor"], 0x2ca3)
            || !dji_number(p[@"idProduct"], 0x1020)
            || !dji_number(p[@"bConfigurationValue"], 1)
            || !dji_number(p[@"bInterfaceClass"], 255)
            || !dji_number(p[@"bInterfaceSubClass"], 240)) return NO;
        unsigned bit;
        if (dji_number(p[@"bInterfaceNumber"], 0)
            && dji_number(p[@"bInterfaceProtocol"], 0)
            && dji_number(p[@"bAlternateSetting"], 0)
            && dji_number(p[@"bNumEndpoints"], 2)
            && [p[@"kUSBString"] isEqual:@"iAP Interface"]) {
            bit = 1;
        } else if (dji_number(p[@"bInterfaceNumber"], 1)
            && dji_number(p[@"bInterfaceProtocol"], 1)
            && [p[@"kUSBString"] isEqual:@"com.dji.logiclink"]
            && ((dji_number(p[@"bAlternateSetting"], 0) && dji_number(p[@"bNumEndpoints"], 0))
                || (dji_number(p[@"bAlternateSetting"], 1) && dji_number(p[@"bNumEndpoints"], 2)))) {
            bit = 2;
        } else return NO;
        if (seen & bit) return NO;
        seen |= bit;
    }
    return seen == 3;
}

static NSArray<NSDictionary *> *interface_properties(io_service_t device, DjiError *error) {
    io_iterator_t iterator = IO_OBJECT_NULL;
    IOReturn status = IORegistryEntryGetChildIterator(device, kIOServicePlane, &iterator);
    if (status != KERN_SUCCESS) {
        dji_fail(error, 21, status);
        return nil;
    }
    NSMutableArray *properties = [NSMutableArray array];
    BOOL valid = YES;
    io_service_t child;
    while ((child = IOIteratorNext(iterator)) != IO_OBJECT_NULL) {
        if (IOObjectConformsTo(child, "IOUSBHostInterface")) {
            NSDictionary *p = dji_properties(child);
            if (p == nil) valid = NO;
            else [properties addObject:p];
        }
        if (IOObjectRelease(child) != KERN_SUCCESS) valid = NO;
    }
    if (!IOIteratorIsValid(iterator)) valid = NO;
    if (IOObjectRelease(iterator) != KERN_SUCCESS) valid = NO;
    if (!valid) {
        dji_fail(error, 21, kIOReturnNotReady);
        return nil;
    }
    return properties;
}

static int candidate(io_service_t service, DjiCandidate *output, DjiError *error) {
    NSDictionary *p = dji_properties(service);
    if (p == nil) return dji_fail(error, 22, kIOReturnNotReady);
    if (!device_profile(p)) return 0;
    NSArray *interfaces = interface_properties(service, error);
    if (interfaces == nil) return -1;
    if (!dji_discovery_profile(p, interfaces)) return 0;
    id location = p[@"locationID"];
    if (location == nil || CFGetTypeID((__bridge CFTypeRef)location) != CFNumberGetTypeID()
        || [location unsignedLongLongValue] > UINT32_MAX)
        return dji_fail(error, 22, kIOReturnBadArgument);
    uint64_t identity = 0;
    IOReturn status = IORegistryEntryGetRegistryEntryID(service, &identity);
    if (status != KERN_SUCCESS || identity == 0)
        return dji_fail(error, 22, status == KERN_SUCCESS ? kIOReturnBadArgument : status);
    *output = (DjiCandidate){identity, [location unsignedIntValue], 0};
    return 1;
}

int dji_usb_discover(DjiCandidate *output, uint32_t capacity,
    uint32_t *count, DjiError *error) {
    @autoreleasepool {
        *count = 0;
        NSMutableDictionary *match = CFBridgingRelease(IOServiceMatching("IOUSBHostDevice"));
        if (match == nil) return dji_fail(error, 20, kIOReturnNoMemory);
        match[@kIOPropertyMatchKey] = @{@"idVendor": @0x2ca3, @"idProduct": @0x1020};
        io_iterator_t iterator = IO_OBJECT_NULL;
        IOReturn status = IOServiceGetMatchingServices(kIOMainPortDefault,
            (__bridge_retained CFDictionaryRef)match, &iterator);
        if (status != KERN_SUCCESS) return dji_fail(error, 20, status);
        // IOKit returns success with no iterator when the matching set is empty.
        if (iterator == IO_OBJECT_NULL) return 0;
        int result = 0;
        io_service_t service;
        while ((service = IOIteratorNext(iterator)) != IO_OBJECT_NULL) {
            DjiCandidate found = {0};
            int selected = candidate(service, &found, error);
            if (IOObjectRelease(service) != KERN_SUCCESS) {
                result = dji_fail(error, 23, kIOReturnNotReady);
                break;
            }
            if (selected < 0) { result = -1; break; }
            if (selected == 0) continue;
            if (*count >= capacity) {
                result = dji_fail(error, 23, kIOReturnNoSpace);
                break;
            }
            output[*count] = found;
            (*count)++;
        }
        if (!IOIteratorIsValid(iterator) && result == 0)
            result = dji_fail(error, 23, kIOReturnNotReady);
        if (IOObjectRelease(iterator) != KERN_SUCCESS && result == 0)
            result = dji_fail(error, 23, kIOReturnNotReady);
        if (result != 0) *count = 0;
        return result;
    }
}
