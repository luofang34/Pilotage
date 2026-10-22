#import "access.h"
#include <string.h>

static id property(io_registry_entry_t entry, NSString *key) {
    return CFBridgingRelease(IORegistryEntryCreateCFProperty(entry,
        (__bridge CFStringRef)key, kCFAllocatorDefault, 0));
}

static uint64_t identity(io_registry_entry_t entry) {
    uint64_t result = 0;
    return IORegistryEntryGetRegistryEntryID(entry, &result) == KERN_SUCCESS ? result : 0;
}

int dji_access_device_state(void) {
    io_iterator_t iterator = IO_OBJECT_NULL;
    if (IOServiceGetMatchingServices(kIOMainPortDefault,
        IOServiceMatching("IOUSBHostDevice"), &iterator) != KERN_SUCCESS) return -1;
    if (iterator == IO_OBJECT_NULL) return 0;
    BOOL found = NO;
    io_service_t service;
    while ((service = IOIteratorNext(iterator)) != IO_OBJECT_NULL) {
        found |= dji_access_value(property(service, @"idVendor"), 0x2ca3)
            && dji_access_value(property(service, @"idProduct"), 0x1020)
            && dji_access_value(property(service, @"bDeviceClass"), 239)
            && dji_access_value(property(service, @"bDeviceSubClass"), 2)
            && dji_access_value(property(service, @"bDeviceProtocol"), 1);
        IOObjectRelease(service);
    }
    BOOL valid = IOIteratorIsValid(iterator);
    IOObjectRelease(iterator);
    return valid ? found : -1;
}

static BOOL controllerPort(io_service_t controller, uint64_t number) {
    io_registry_entry_t parent = IO_OBJECT_NULL;
    if (IORegistryEntryGetParentEntry(controller, kIOServicePlane, &parent) != KERN_SUCCESS)
        return NO;
    id port = property(parent, @"port-number");
    BOOL valid = NO;
    if ([port isKindOfClass:NSData.class] && [port length] == sizeof(uint32_t)) {
        uint32_t value;
        [port getBytes:&value length:sizeof(value)];
        valid = CFSwapInt32LittleToHost(value) == number;
    }
    IOObjectRelease(parent);
    return valid;
}

static BOOL controllerForPort(uint64_t number, DjiAccessIdentity *candidate) {
    io_iterator_t controllers = IO_OBJECT_NULL;
    if (IOServiceGetMatchingServices(kIOMainPortDefault,
        IOServiceMatching("IOUSBDeviceController"), &controllers) != KERN_SUCCESS) return NO;
    unsigned matched = 0, interfaces = 0;
    BOOL valid = YES;
    io_service_t controller;
    while ((controller = IOIteratorNext(controllers)) != IO_OBJECT_NULL) {
        if (controllerPort(controller, number)
            && IOObjectConformsTo(controller, "AppleT8122USBXDCI")
            && [property(controller, @"ConfigurationType") isEqual:@"ncmAuxBringup"]) {
            matched++;
            candidate->controller = identity(controller);
            io_iterator_t children = IO_OBJECT_NULL;
            if (IORegistryEntryGetChildIterator(controller, kIOServicePlane, &children) != KERN_SUCCESS) {
                valid = NO;
            } else {
                io_service_t child;
                while ((child = IOIteratorNext(children)) != IO_OBJECT_NULL) {
                    if (IOObjectConformsTo(child, "IOUSBDeviceInterface")
                        && [property(child, @"USBDeviceFunction") isEqual:@"AppleUSBNCMData"]) {
                        interfaces++;
                        candidate->interface = identity(child);
                    }
                    IOObjectRelease(child);
                }
                valid &= IOIteratorIsValid(children);
                IOObjectRelease(children);
            }
        }
        IOObjectRelease(controller);
    }
    valid &= IOIteratorIsValid(controllers);
    IOObjectRelease(controllers);
    return valid && matched == 1 && interfaces == 1;
}

DjiAccessStatus dji_access_discover(DjiAccessIdentity *candidate) {
    memset(candidate, 0, sizeof(*candidate));
    int device = dji_access_device_state();
    if (device < 0) return DjiAccessBlocked;
    if (device > 0) return DjiAccessReady;
    io_iterator_t ports = IO_OBJECT_NULL;
    if (IOServiceGetMatchingServices(kIOMainPortDefault,
        IOServiceMatching("AppleHPMInterfaceType10"), &ports) != KERN_SUCCESS)
        return DjiAccessBlocked;
    unsigned count = 0;
    BOOL unauthorized = NO, valid = YES;
    io_service_t port;
    while ((port = IOIteratorNext(ports)) != IO_OBJECT_NULL) {
        if (dji_access_value(property(port, @"ConnectionActive"), 1)
            && dji_access_value(property(port, @"IOAccessoryUSBModeType"), 5)
            && dji_access_value(property(port, @"IOAccessoryUSBConnectType"), 2)) {
            id uuid = property(port, @"ConnectionUUID");
            NSUUID *parsed = [uuid isKindOfClass:NSString.class]
                ? [[NSUUID alloc] initWithUUIDString:uuid] : nil;
            id number = property(port, @"PortNumber");
            if (parsed != nil && [number isKindOfClass:NSNumber.class]) {
                count++;
                candidate->port = identity(port);
                strlcpy(candidate->connection, parsed.UUIDString.UTF8String, sizeof(candidate->connection));
                unauthorized |= !dji_access_port_guard(candidate, NO);
                valid &= controllerForPort([number unsignedLongLongValue], candidate);
            }
        }
        IOObjectRelease(port);
    }
    valid &= IOIteratorIsValid(ports);
    IOObjectRelease(ports);
    if (count > 1) return DjiAccessAmbiguous;
    if (unauthorized) return DjiAccessUnauthorized;
    if (!valid) return DjiAccessBlocked;
    return count == 1 ? DjiAccessHandshake : DjiAccessWaiting;
}
