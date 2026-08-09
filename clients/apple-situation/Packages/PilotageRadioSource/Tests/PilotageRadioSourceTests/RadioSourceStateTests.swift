import Testing

@testable import PilotageRadioSource

@Test("A failed band does not mask a live band")
func bandIndependence() {
    var state = RadioDegradedState()
    state.record(.underpowered, for: .uat978)

    #expect(state.effectiveAvailability(active: .streaming) == .streaming)
    #expect(state.bandFailures == [
        RadioBandFailure(id: .uat978, detail: "USB path is underpowered")
    ])
}

@Test("A failed last band explains the idle state")
func failedIdleBand() {
    var state = RadioDegradedState()
    state.record(.underpowered, for: .uat978)

    #expect(
        state.effectiveAvailability(active: nil, idle: .unplugged)
            == .underpowered
    )
}

@Test("A process failure has priority over live reception")
func processFailurePriority() {
    var state = RadioDegradedState()
    state.recordUnscoped(.permissionDenied("access denied"))

    #expect(
        state.effectiveAvailability(active: .streaming)
            == .permissionDenied("access denied")
    )
}

@Test("A live receiver masks a generic scan failure")
func liveReceiverMasksScanFailure() {
    var state = RadioDegradedState()
    state.recordUnscoped(.endpointFailure("partial scan failed"))

    #expect(state.effectiveAvailability(active: .streaming) == .streaming)
}

@Test("A clean reconnect clears only its band")
func reconnectClearsOneBand() {
    var state = RadioDegradedState()
    state.record(.deviceRemoved, for: .adsb1090)
    state.record(.underpowered, for: .uat978)

    state.clear(.adsb1090)

    #expect(state.bandFailures == [
        RadioBandFailure(id: .uat978, detail: "USB path is underpowered")
    ])
}

@Test("Suspension clears persistent failures")
func suspensionClearsFailures() {
    var state = RadioDegradedState()
    state.recordUnscoped(.endpointFailure("scan failed"))
    state.record(.deviceRemoved, for: .uat978)

    state.clearAll()

    #expect(state.effectiveAvailability(active: .suspended) == .suspended)
    #expect(state.bandFailures.isEmpty)
}

@Test("Only a clean scan retires a process failure")
func cleanScanRule() {
    #expect(scanRetiresProcessFailure(
        hadOpenFailures: false,
        hasScanError: false,
        hasReceiverFailures: false
    ))
    #expect(!scanRetiresProcessFailure(
        hadOpenFailures: false,
        hasScanError: false,
        hasReceiverFailures: true
    ))
    #expect(!scanRetiresProcessFailure(
        hadOpenFailures: true,
        hasScanError: false,
        hasReceiverFailures: false
    ))
}

@Test("A reconnect request raised during a scan survives its result")
func reconnectRequestSurvivesScan() {
    #expect(reconnectRequiredAfterScan(
        pending: true,
        hadOpenFailures: false,
        hasScanError: false,
        hasReceiverFailures: false
    ))
    #expect(reconnectRequiredAfterScan(
        pending: false,
        hadOpenFailures: true,
        hasScanError: false,
        hasReceiverFailures: false
    ))
    #expect(!reconnectRequiredAfterScan(
        pending: false,
        hadOpenFailures: false,
        hasScanError: false,
        hasReceiverFailures: false
    ))
}
