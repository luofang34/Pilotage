#ifndef PILOTAGE_DJI_LOGICLINK_H
#define PILOTAGE_DJI_LOGICLINK_H
#include <stdint.h>
#include <stddef.h>

typedef struct { uint32_t stage, status; } DjiError;
typedef struct DjiUsb DjiUsb;
typedef struct {
    uint8_t inputAddress, outputAddress;
    uint16_t inputPacketSize, outputPacketSize;
} DjiProfile;

int dji_usb_open(uint64_t identity, DjiUsb **output, DjiError *error);
int dji_usb_poll(DjiUsb *usb, uint8_t *output, uint32_t capacity,
    uint32_t *count, DjiError *error);
int dji_usb_write(DjiUsb *usb, const uint8_t *bytes, uint32_t count,
    uint64_t deadline_ns, DjiError *error);
int dji_usb_close(DjiUsb *usb, DjiError *error);
uint64_t dji_usb_monotonic_ns(void);
int dji_usb_profile_test(void);
int dji_usb_lifecycle_test(void);
int dji_usb_deadline_test(void);

#ifdef __OBJC__
#import <Foundation/Foundation.h>
#import <IOKit/IOKitLib.h>
#import <IOKit/usb/IOUSBLib.h>
struct DjiUsb {
    IOUSBInterfaceInterface650 **interface;
    io_service_t service;
    uint64_t deviceID;
    CFRunLoopSourceRef source;
    UInt8 inputPipe, outputPipe, original;
    BOOL opened, originalKnown, changed, pending, ready;
    IOReturn readStatus;
    uintptr_t readSize;
    uint8_t bytes[16384];
};
void dji_read_complete(void *context, IOReturn status, void *count);
int dji_write_pipe(DjiUsb *usb, const uint8_t *bytes, uint32_t count,
    uint64_t deadline_ns, DjiError *error);
int dji_fail(DjiError *error, uint32_t stage, uint32_t status);
BOOL dji_number(id value, uint64_t expected);
NSDictionary *dji_properties(io_registry_entry_t entry);
BOOL dji_profile(NSData *data, DjiProfile *profile);
BOOL dji_read_profile(io_service_t service, DjiProfile *profile, DjiError *error);
io_service_t dji_logic_service(io_service_t device, DjiError *error);
#endif
#endif
