#import "logiclink.h"
#import <IOKit/IOCFPlugIn.h>
#import <IOKit/usb/IOUSBLib.h>
#include <string.h>

int dji_fail(DjiError *error, uint32_t stage, uint32_t status) {
    *error = (DjiError){stage, status};
    return -1;
}

NSDictionary *dji_properties(io_registry_entry_t entry) {
    CFMutableDictionaryRef raw = NULL;
    if (IORegistryEntryCreateCFProperties(entry, &raw, kCFAllocatorDefault, 0)
        != KERN_SUCCESS) return nil;
    return CFBridgingRelease(raw);
}

BOOL dji_number(id value, uint64_t expected) {
    return value != nil && CFGetTypeID((__bridge CFTypeRef)value) == CFNumberGetTypeID()
        && [value unsignedLongLongValue] == expected;
}

BOOL dji_profile(NSData *data, DjiProfile *profile) {
    const uint8_t *b = data.bytes;
    if (data.length < 9 || b[0] != 9 || b[1] != 2 || b[4] != 2 || b[5] != 1
        || (NSUInteger)(b[2] | (b[3] << 8)) != data.length) return NO;
    unsigned seen = 0, counts[3] = {0}, directions[3] = {0};
    uint32_t endpointMask = 0;
    int current = -1;
    for (NSUInteger offset = 9; offset < data.length;) {
        if (data.length - offset < 2) return NO;
        const uint8_t *d = b + offset;
        unsigned length = d[0];
        if (length < 2 || length > data.length - offset) return NO;
        if (d[1] == 4) {
            if (length != 9 || d[5] != 255 || d[6] != 240) return NO;
            if (d[2] == 0 && d[3] == 0 && d[4] == 2 && d[7] == 0) current = 0;
            else if (d[2] == 1 && d[3] == 0 && d[4] == 0 && d[7] == 1) current = 1;
            else if (d[2] == 1 && d[3] == 1 && d[4] == 2 && d[7] == 1) current = 2;
            else return NO;
            unsigned bit = 1U << current;
            if (seen & bit) return NO;
            seen |= bit;
        } else if (d[1] == 5) {
            if (length != 7 || current < 0 || current == 1 || d[3] != 2
                || (d[2] & 0x70) != 0 || (d[2] & 15) == 0) return NO;
            uint16_t packet = d[4] | (d[5] << 8);
            if (packet == 0 || packet > 512) return NO;
            uint32_t endpointBit = 1U << ((d[2] & 15) + ((d[2] & 0x80) ? 16 : 0));
            if (endpointMask & endpointBit) return NO;
            endpointMask |= endpointBit;
            unsigned direction = (d[2] & 0x80) ? 1 : 2;
            if (directions[current] & direction) return NO;
            directions[current] |= direction;
            counts[current]++;
            if (current == 2) {
                if (direction == 1) {
                    profile->inputAddress = d[2];
                    profile->inputPacketSize = packet;
                } else {
                    profile->outputAddress = d[2];
                    profile->outputPacketSize = packet;
                }
            }
        } else if (d[1] != 11 && d[1] != 0x24 && d[1] != 0x25) {
            return NO;
        }
        offset += length;
    }
    return seen == 7 && counts[0] == 2 && counts[1] == 0 && counts[2] == 2
        && directions[0] == 3 && directions[2] == 3;
}

BOOL dji_read_profile(io_service_t service, DjiProfile *profile, DjiError *error) {
    IOCFPlugInInterface **plugin = NULL;
    IOUSBDeviceInterface650 **device = NULL;
    SInt32 score = 0;
    IOReturn status = IOCreatePlugInInterfaceForService(service, kIOUSBDeviceUserClientTypeID,
        kIOCFPlugInInterfaceID, &plugin, &score);
    if (status != kIOReturnSuccess || plugin == NULL) {
        if (plugin != NULL) (*plugin)->Release(plugin);
        dji_fail(error, 2, status);
        return NO;
    }
    HRESULT queried = (*plugin)->QueryInterface(plugin,
        CFUUIDGetUUIDBytes(kIOUSBDeviceInterfaceID650), (LPVOID *)&device);
    (*plugin)->Release(plugin);
    if (queried != 0 || device == NULL) {
        if (device != NULL) (*device)->Release(device);
        dji_fail(error, 3, (uint32_t)queried);
        return NO;
    }
    IOUSBConfigurationDescriptorPtr raw = NULL;
    status = (*device)->GetConfigurationDescriptorPtr(device, 0, &raw);
    NSData *data = nil;
    if (status == kIOReturnSuccess && raw != NULL) {
        const uint8_t *b = (const uint8_t *)raw;
        NSUInteger length = b[2] | (b[3] << 8);
        if (length >= 9) data = [NSData dataWithBytes:b length:length];
    }
    (*device)->Release(device);
    if (status == kIOReturnSuccess && data != nil && dji_profile(data, profile)) return YES;
    dji_fail(error, 4, status == kIOReturnSuccess ? kIOReturnBadArgument : status);
    return NO;
}

io_service_t dji_logic_service(io_service_t device, DjiError *error) {
    io_iterator_t iterator = IO_OBJECT_NULL;
    IOReturn status = IORegistryEntryGetChildIterator(device, kIOServicePlane, &iterator);
    if (status != KERN_SUCCESS) {
        dji_fail(error, 5, status);
        return IO_OBJECT_NULL;
    }
    io_service_t selected = IO_OBJECT_NULL, child;
    unsigned count = 0;
    BOOL valid = YES;
    while ((child = IOIteratorNext(iterator)) != IO_OBJECT_NULL) {
        if (IOObjectConformsTo(child, "IOUSBHostInterface")) {
            count++;
            NSDictionary *p = dji_properties(child);
            BOOL shape = dji_number(p[@"idVendor"], 0x2ca3)
                && dji_number(p[@"idProduct"], 0x1020)
                && dji_number(p[@"bInterfaceClass"], 255)
                && dji_number(p[@"bInterfaceSubClass"], 240)
                && dji_number(p[@"bConfigurationValue"], 1);
            if (shape && dji_number(p[@"bInterfaceNumber"], 1)
                && dji_number(p[@"bInterfaceProtocol"], 1)
                && [p[@"kUSBString"] isEqual:@"com.dji.logiclink"]
                && selected == IO_OBJECT_NULL) {
                if (IOObjectRetain(child) == KERN_SUCCESS) selected = child;
                else valid = NO;
            } else if (!shape || !dji_number(p[@"bInterfaceNumber"], 0)
                || !dji_number(p[@"bInterfaceProtocol"], 0)) valid = NO;
        }
        if (IOObjectRelease(child) != KERN_SUCCESS) valid = NO;
    }
    if (IOObjectRelease(iterator) != KERN_SUCCESS) valid = NO;
    if (!valid || count != 2 || selected == IO_OBJECT_NULL) {
        if (selected != IO_OBJECT_NULL) IOObjectRelease(selected);
        dji_fail(error, 5, kIOReturnBadArgument);
        return IO_OBJECT_NULL;
    }
    return selected;
}

int dji_usb_profile_test(void) {
    @autoreleasepool {
        const uint8_t good[] = {
            9,2,64,0,2,1,0,0x80,250, 9,4,0,0,2,255,240,0,5,
            7,5,0x81,2,0,2,0, 7,5,1,2,0,2,0, 9,4,1,0,0,255,240,1,6,
            9,4,1,1,2,255,240,1,6, 7,5,0x82,2,0,2,0, 7,5,2,2,0,2,0
        };
        DjiProfile profile = {0};
        unsigned failures = !dji_profile([NSData dataWithBytes:good length:sizeof(good)], &profile)
            || profile.inputAddress != 0x82 || profile.outputAddress != 2
            || profile.inputPacketSize != 512 || profile.outputPacketSize != 512;
        const unsigned offsets[] = {15, 50, 44, 52, 53};
        const uint8_t values[] = {0x43, 0, 2, 0x81, 3};
        for (unsigned i = 0; i < sizeof(offsets) / sizeof(offsets[0]); i++) {
            uint8_t bad[sizeof(good)];
            memcpy(bad, good, sizeof(good));
            bad[offsets[i]] = values[i];
            failures += dji_profile([NSData dataWithBytes:bad length:sizeof(bad)], &profile);
        }
        for (size_t size = 0; size < sizeof(good); size++) {
            failures += dji_profile([NSData dataWithBytes:good length:size], &profile);
        }
        return failures == 0 ? 0 : -1;
    }
}
