#import "discovery.h"

int dji_usb_discovery_test(void) {
    @autoreleasepool {
        NSDictionary *device = @{
            @"idVendor": @0x2ca3, @"idProduct": @0x1020, @"bDeviceClass": @239,
            @"bDeviceSubClass": @2, @"bDeviceProtocol": @1, @"bNumConfigurations": @1,
            @"kUSBCurrentConfiguration": @1, @"USB Product Name": @"C5"
        };
        NSDictionary *iap = @{
            @"idVendor": @0x2ca3, @"idProduct": @0x1020, @"bConfigurationValue": @1,
            @"bInterfaceClass": @255, @"bInterfaceSubClass": @240, @"bInterfaceProtocol": @0,
            @"bInterfaceNumber": @0, @"bAlternateSetting": @0, @"bNumEndpoints": @2,
            @"kUSBString": @"iAP Interface"
        };
        NSDictionary *logic = @{
            @"idVendor": @0x2ca3, @"idProduct": @0x1020, @"bConfigurationValue": @1,
            @"bInterfaceClass": @255, @"bInterfaceSubClass": @240, @"bInterfaceProtocol": @1,
            @"bInterfaceNumber": @1, @"bAlternateSetting": @0, @"bNumEndpoints": @0,
            @"kUSBString": @"com.dji.logiclink"
        };
        unsigned failures = !dji_discovery_profile(device, @[iap, logic])
            || !dji_discovery_profile(device, @[logic, iap]);
        failures += dji_discovery_profile(device, @[]);
        failures += dji_discovery_profile(device, @[iap]);
        failures += dji_discovery_profile(device, @[logic, logic]);
        failures += dji_discovery_profile(device, @[iap, logic, logic]);
        for (NSString *key in device) {
            NSMutableDictionary *bad = [device mutableCopy];
            [bad removeObjectForKey:key];
            failures += dji_discovery_profile(bad, @[iap, logic]);
        }
        for (NSDictionary *interface in @[iap, logic]) {
            for (NSString *key in interface) {
                NSMutableDictionary *bad = [interface mutableCopy];
                [bad removeObjectForKey:key];
                NSArray *interfaces = interface == iap ? @[bad, logic] : @[iap, bad];
                failures += dji_discovery_profile(device, interfaces);
            }
        }
        NSMutableDictionary *active = [logic mutableCopy];
        active[@"bAlternateSetting"] = @1;
        active[@"bNumEndpoints"] = @2;
        failures += !dji_discovery_profile(device, @[iap, active]);
        active[@"bAlternateSetting"] = @2;
        failures += dji_discovery_profile(device, @[iap, active]);
        active[@"bAlternateSetting"] = @1;
        active[@"bInterfaceClass"] = @2; // A CDC interface is not the upper profile.
        failures += dji_discovery_profile(device, @[iap, active]);
        return failures == 0 ? 0 : -1;
    }
}
