#import "logiclink.h"
#include <stdlib.h>
#include <string.h>

typedef struct {
    IOUSBInterfaceInterface650 *table;
    DjiUsb *usb;
    unsigned step, errors;
    BOOL completes;
    IOReturn restoreStatus, closeStatus;
} Fixture;

static void step(Fixture *fixture, unsigned expected) {
    if (++fixture->step != expected) fixture->errors++;
}

static IOReturn abortInput(void *self, UInt8 pipe) {
    Fixture *fixture = self;
    step(fixture, 1);
    if (pipe != 1) fixture->errors++;
    if (fixture->completes) dji_read_complete(fixture->usb, kIOReturnAborted, NULL);
    return kIOReturnSuccess;
}

static IOReturn restore(void *self, UInt8 alternate) {
    Fixture *fixture = self;
    step(fixture, 2);
    if (fixture->usb->pending || alternate != 0) fixture->errors++;
    return fixture->restoreStatus;
}

static IOReturn verify(void *self, UInt8 *alternate) {
    Fixture *fixture = self;
    step(self, 3);
    *alternate = 0;
    return fixture->restoreStatus;
}

static IOReturn closeInterface(void *self) {
    Fixture *fixture = self;
    step(self, 4);
    return fixture->closeStatus;
}

static ULONG releaseInterface(void *self) {
    step(self, 5);
    return 0;
}

static int cleanupTest(BOOL completes, IOReturn restoration, IOReturn closeStatus, int expected) {
    IOUSBInterfaceInterface650 table = {
        .AbortPipe = abortInput, .SetAlternateInterface = restore,
        .GetAlternateSetting = verify, .USBInterfaceClose = closeInterface,
        .Release = releaseInterface,
    };
    DjiUsb *usb = calloc(1, sizeof(DjiUsb));
    if (usb == NULL) return 1;
    Fixture fixture = {.table = &table, .usb = usb, .completes = completes,
        .restoreStatus = restoration, .closeStatus = closeStatus};
    *usb = (DjiUsb){
        .interface = &fixture.table, .inputPipe = 1, .opened = YES,
        .originalKnown = YES, .changed = YES, .pending = YES,
    };
    DjiError error = {0};
    int result = dji_usb_close(usb, &error);
    if (!completes) {
        if (result != -1 || error.stage != 15 || fixture.step != 1) fixture.errors++;
        // The failed close must retain the callback context and interface.
        dji_read_complete(usb, kIOReturnAborted, NULL);
        result = dji_usb_close(usb, &error);
    }
    return fixture.errors + (result != expected) + (fixture.step != 5);
}

static int completionTest(void) {
    DjiUsb usb = {0};
    uint8_t output[16384];
    memset(output, 7, sizeof(output));
    usb.bytes[0] = 1;
    usb.bytes[1] = 2;
    DjiError error = {0};
    uint32_t count = 55;
    dji_read_complete(&usb, kIOReturnError, (void *)(uintptr_t)2);
    int result = dji_usb_poll(&usb, output, sizeof(output), &count, &error);
    int failures = result != -1 || count != 0 || output[0] != 7 || error.stage != 12;
    dji_read_complete(&usb, kIOReturnSuccess, (void *)(uintptr_t)2);
    result = dji_usb_poll(&usb, output, sizeof(output), &count, &error);
    failures += result != 0 || count != 2 || output[0] != 1 || output[1] != 2;
    dji_read_complete(&usb, kIOReturnSuccess, (void *)(uintptr_t)16385);
    result = dji_usb_poll(&usb, output, sizeof(output), &count, &error);
    failures += result != -1 || count != 0;
    return failures;
}

int dji_usb_lifecycle_test(void) {
    @autoreleasepool {
        return cleanupTest(YES, 0, 0, 0) + cleanupTest(NO, 0, 0, 0)
            + cleanupTest(YES, kIOReturnNoDevice, kIOReturnNoDevice, 1)
            + cleanupTest(YES, kIOReturnError, kIOReturnSuccess, 1)
            + cleanupTest(YES, 0, kIOReturnError, -1) + completionTest();
    }
}

typedef struct {
    IOUSBInterfaceInterface650 *table;
    uint32_t calls, errors, limit;
} WriteFixture;

static IOReturn writeOutput(void *self, UInt8 pipe, void *bytes, UInt32 count,
    UInt32 noDataTimeout, UInt32 completionTimeout) {
    WriteFixture *fixture = self;
    fixture->calls++;
    if (pipe != 2 || count != 1 || *(uint8_t *)bytes != 7
        || completionTimeout == 0 || completionTimeout > fixture->limit
        || noDataTimeout != completionTimeout) fixture->errors++;
    return kIOReturnSuccess;
}

int dji_usb_deadline_test(void) {
    IOUSBInterfaceInterface650 table = {.WritePipeTO = writeOutput};
    WriteFixture fixture = {.table = &table, .limit = 50};
    DjiUsb usb = {.interface = &fixture.table, .outputPipe = 2};
    DjiError error = {0};
    uint8_t byte = 7;
    int result = dji_write_pipe(&usb, &byte, 1, 0, &error);
    unsigned errors = result != -1 || error.status != (uint32_t)kIOReturnTimeout
        || fixture.calls != 0;
    result = dji_write_pipe(&usb, &byte, 1, dji_usb_monotonic_ns() + 999999, &error);
    errors += result != -1 || error.status != (uint32_t)kIOReturnTimeout
        || fixture.calls != 0;
    result = dji_write_pipe(&usb, &byte, 1, dji_usb_monotonic_ns() + 50000000, &error);
    errors += result != 0 || fixture.calls != 1;
    // The native write also keeps its 50 ms cap if a caller supplies more time.
    result = dji_write_pipe(&usb, &byte, 1, dji_usb_monotonic_ns() + 1000000000, &error);
    errors += result != 0 || fixture.calls != 2;
    return errors + fixture.errors;
}
