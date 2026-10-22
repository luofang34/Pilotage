#import "access.h"
#include <net/if_media.h>

static id property(io_registry_entry_t entry, NSString *key) {
    return CFBridgingRelease(IORegistryEntryCreateCFProperty(entry,
        (__bridge CFStringRef)key, kCFAllocatorDefault, 0));
}

static uint64_t identity(io_registry_entry_t entry) {
    uint64_t result = 0;
    return IORegistryEntryGetRegistryEntryID(entry, &result) == KERN_SUCCESS ? result : 0;
}

BOOL dji_access_port_guard(const DjiAccessIdentity *expected, BOOL hostPhase) {
    io_service_t port = IOServiceGetMatchingService(kIOMainPortDefault,
        IORegistryEntryIDMatching(expected->port));
    if (port == IO_OBJECT_NULL) return NO;
    uint64_t mode = hostPhase ? 2 : 5, role = hostPhase ? 2 : 1;
    BOOL valid = IOObjectConformsTo(port, "AppleHPMInterfaceType10")
        && [property(port, @"ConnectionUUID") isEqual:@(expected->connection)]
        && dji_access_value(property(port, @"ConnectionActive"), 1)
        && dji_access_value(property(port, @"AuthorizationPending"), 0)
        && dji_access_value(property(port, @"UserAuthorizationPending"), 0)
        && dji_access_value(property(port, @"IOAccessoryUSBModeType"), mode)
        && dji_access_value(property(port, @"IOAccessoryUSBConnectType"), hostPhase ? 4 : 2);
    io_iterator_t children = IO_OBJECT_NULL;
    unsigned usb2 = 0, usb3 = 0;
    if (IORegistryEntryGetChildIterator(port, kIOServicePlane, &children) != KERN_SUCCESS) {
        valid = NO;
    } else {
        io_service_t child;
        while ((child = IOIteratorNext(children)) != IO_OBJECT_NULL) {
            id type = property(child, @"TransportType");
            if (dji_access_value(type, 2) || dji_access_value(type, 3)) {
                // The private role setter writes AuthenticationRequired=false.
                // It is admissible only when this does not change that policy.
                valid &= dji_access_value(property(child, @"AuthenticationRequired"), 0)
                    && dji_access_value(property(child, @"AuthorizationStatus"), 2)
                    && dji_access_value(property(child, @"DataRole"), role);
                if (dji_access_value(type, 2)) usb2++; else usb3++;
            }
            IOObjectRelease(child);
        }
        valid &= IOIteratorIsValid(children);
        IOObjectRelease(children);
    }
    IOObjectRelease(port);
    return valid && usb2 == 1 && usb3 == 1;
}

BOOL dji_access_controller_guard(const DjiAccessIdentity *expected, BOOL early) {
    io_service_t controller = IOServiceGetMatchingService(kIOMainPortDefault,
        IORegistryEntryIDMatching(expected->controller));
    if (controller == IO_OBJECT_NULL) return NO;
    id state = property(controller, @"CurrentState");
    BOOL dictionary = [state isKindOfClass:NSDictionary.class];
    BOOL configured = dictionary && [state[@"DeviceState"] isEqual:@"Configured"]
        && dji_access_value(state[@"SelectedConfiguration"], 1)
        && dji_access_value(state[@"ConnectionSpeed"], 2);
    BOOL powered = dictionary && [state[@"DeviceState"] isEqual:@"Powered"]
        && dji_access_value(state[@"DeviceAddress"], 0)
        && [@[@"SS.Disabled", @"Rx.Detect"] containsObject:state[@"HardwareLinkState"] ?: NSNull.null]
        && dji_access_value(state[@"ConnectionSpeed"], 4);
    BOOL enumerating = dictionary
        && [@[@"Powered", @"Default", @"Addressed"] containsObject:state[@"DeviceState"] ?: NSNull.null]
        && dji_access_value(state[@"SelectedConfiguration"], 0)
        && (dji_access_value(state[@"ConnectionSpeed"], 0) || dji_access_value(state[@"ConnectionSpeed"], 2) || powered);
    BOOL valid = IOObjectConformsTo(controller, "AppleT8122USBXDCI")
        && [property(controller, @"ConfigurationType") isEqual:@"ncmAuxBringup"]
        && dictionary && dji_access_value(state[@"OnBus"], 1)
        && (configured || (early && enumerating));
    IOObjectRelease(controller);
    return valid;
}

BOOL dji_access_interface_guard(const DjiAccessIdentity *expected, io_service_t service) {
    io_registry_entry_t parent = IO_OBJECT_NULL;
    if (IORegistryEntryGetParentEntry(service, kIOServicePlane, &parent) != KERN_SUCCESS) return NO;
    id configuration = property(service, @"Configuration"), index = property(service, @"Index");
    BOOL binding = dji_access_value(configuration, 1) && dji_access_value(index, 1);
    BOOL early = configuration == nil && index == nil
        && dji_access_value(property(service, @"IsActive"), 0)
        && dji_access_controller_guard(expected, YES);
    BOOL valid = identity(service) == expected->interface
        && identity(parent) == expected->controller
        && IOObjectConformsTo(service, "IOUSBDeviceInterface")
        && [property(service, @"USBDeviceFunction") isEqual:@"AppleUSBNCMData"]
        && [property(service, @"IOUserClientClass") isEqual:@"IOUSBDeviceInterfaceUserClient"]
        && (binding || early);
    IOObjectRelease(parent);
    return valid;
}

BOOL dji_access_owner_guard(io_service_t service) {
    io_iterator_t children = IO_OBJECT_NULL;
    if (IORegistryEntryGetChildIterator(service, kIOServicePlane, &children) != KERN_SUCCESS) return NO;
    unsigned count = 0;
    BOOL valid = YES;
    io_registry_entry_t child;
    while ((child = IOIteratorNext(children)) != IO_OBJECT_NULL) {
        count++;
        id link = property(child, @"IOLinkStatus");
        valid &= IOObjectConformsTo(child, "AppleUSBDeviceNCM11Data")
            && dji_access_value(property(child, @"HostAttached"), 0)
            && dji_access_value(property(child, @"IOLinkSpeed"), 0)
            && [link isKindOfClass:NSNumber.class]
            && ([link unsignedLongLongValue] & IFM_AVALID) != 0
            && ([link unsignedLongLongValue] & IFM_ACTIVE) == 0;
        IOObjectRelease(child);
    }
    valid &= count == 1 && IOIteratorIsValid(children);
    IOObjectRelease(children);
    io_iterator_t descendants = IO_OBJECT_NULL;
    if (IORegistryEntryCreateIterator(service, kIOServicePlane, kIORegistryIterateRecursively,
        &descendants) != KERN_SUCCESS) return NO;
    while ((child = IOIteratorNext(descendants)) != IO_OBJECT_NULL) {
        if (IOObjectConformsTo(child, "IOUserClient")) {
            // IONetworkStack's shared client is not an owner of this USB provider.
            io_registry_entry_t parent = IO_OBJECT_NULL;
            BOOL shared = IOObjectConformsTo(child, "IONetworkStackUserClient")
                && IORegistryEntryGetParentEntry(child, kIOServicePlane, &parent) == KERN_SUCCESS
                && IOObjectConformsTo(parent, "IONetworkStack");
            valid &= shared;
            if (parent != IO_OBJECT_NULL) IOObjectRelease(parent);
        }
        IOObjectRelease(child);
    }
    valid &= IOIteratorIsValid(descendants);
    IOObjectRelease(descendants);
    return valid;
}

BOOL dji_access_role(DjiAccess *access, uint64_t mode) {
    BOOL fromHost = mode == 5;
    if (NSProcessInfo.processInfo.systemUptime >= access->deadline) {
        access->fenced = YES;
        return NO;
    }
    if (!dji_access_port_guard(&access->identity, fromHost)
        || (!fromHost && !dji_access_controller_guard(&access->identity, NO))) return NO;
    io_connect_t client = IO_OBJECT_NULL;
    if (IOServiceOpen(access->port, mach_task_self(), 0, &client) != KERN_SUCCESS) return NO;
    BOOL valid = dji_access_port_guard(&access->identity, fromHost);
    IOReturn status = valid
        ? IOConnectCallScalarMethod(client, 1, &mode, 1, NULL, NULL) : kIOReturnNotPermitted;
    IOReturn closed = IOServiceClose(client);
    BOOL timely = NSProcessInfo.processInfo.systemUptime < access->deadline;
    if (status != KERN_SUCCESS || closed != KERN_SUCCESS || !timely) access->fenced = YES;
    return status == KERN_SUCCESS && closed == KERN_SUCCESS && timely;
}
