@preconcurrency import AeroLinkAppleClient
import Foundation
import PilotageRadioSource

struct AeroLinkConnectionHandle: @unchecked Sendable {
    let value: ALDriverConnection

    var key: UInt32 { value.receiverKind.rawValue }
    var identity: ObjectIdentifier { ObjectIdentifier(value) }
    var transport: RadioTransport? { Self.transport(for: value.receiverKind) }
    var band: RadioBand? { Self.band(for: value.receiverKind) }

    private static func transport(for kind: ALReceiverKind) -> RadioTransport? {
        switch kind {
        case .adsb1090: .adsb1090
        case .uat978Ftdi: .uat978Ftdi
        case .uat978Cdc: .uat978Cdc
        default: nil
        }
    }

    private static func band(for kind: ALReceiverKind) -> RadioBand? {
        switch kind {
        case .adsb1090: .adsb1090
        case .uat978Ftdi, .uat978Cdc: .uat978
        default: nil
        }
    }
}

struct AeroLinkStatusValue: Sendable {
    let availability: RadioAvailability
    let diagnostics: RadioDiagnostics

    init(_ status: ALReceiverStatus, current: RadioDiagnostics) {
        availability = Self.availability(for: status.driverState)
        var next = current
        next.queueDepth = status.queueDepth
        next.queueCapacity = status.queueCapacity
        next.completedTransfers = status.completedTransfers
        next.completedBytes = status.completedBytes
        next.droppedTransfers = status.droppedTransfers
        next.droppedBytes = status.droppedBytes
        next.ioErrors = status.ioErrors
        next.reconnectGeneration = status.reconnectGeneration
        diagnostics = next
    }

    static func reconnectFailure(for state: ALDriverState) -> RadioAvailability? {
        switch state {
        case .underpowered: .underpowered
        case .enumerationFailure: .enumerationFailure("USB enumeration failed")
        case .endpointFailure: .endpointFailure("USB endpoint stopped")
        case .deviceRemoved: .deviceRemoved
        default: nil
        }
    }

    private static func availability(for state: ALDriverState) -> RadioAvailability {
        switch state {
        case .ready: .ready
        case .streaming: .streaming
        case .underpowered: .underpowered
        case .enumerationFailure: .enumerationFailure("USB enumeration failed")
        case .endpointFailure: .endpointFailure("USB endpoint stopped")
        case .deviceRemoved: .deviceRemoved
        default: .checking
        }
    }
}

struct AeroLinkFailure: Sendable {
    let availability: RadioAvailability
    let band: RadioBand?

    static func classify(_ error: any Error, for handle: AeroLinkConnectionHandle?) -> Self {
        let cocoaError = error as NSError
        if ALDriverConnection.isPermissionError(cocoaError) {
            return Self(
                availability: .permissionDenied(error.localizedDescription),
                band: nil
            )
        }
        if ALDriverConnection.isUnderpoweredError(cocoaError) {
            return Self(availability: .underpowered, band: handle?.band)
        }
        if ALDriverConnection.isDeviceRemovedError(cocoaError) {
            return Self(availability: .deviceRemoved, band: handle?.band)
        }
        return Self(
            availability: .endpointFailure(error.localizedDescription),
            band: handle?.band
        )
    }
}

struct PreparedAeroLinkConnection: Sendable {
    let handle: AeroLinkConnectionHandle
    let status: AeroLinkStatusValue
}

struct AeroLinkDiscoveryAttempt: Sendable {
    var prepared: [PreparedAeroLinkConnection] = []
    var discarded: [AeroLinkConnectionHandle] = []
    var receiverFailures: [AeroLinkFailure] = []
    var hadOpenFailures = false
    var scanFailure: AeroLinkFailure?
}

struct AeroLinkDrainResult: Sendable {
    var eventLines: [String] = []
    var accepted: UInt64 = 0
    var rejected: UInt64 = 0
    var adsb1090GapSamples: UInt64 = 0
    var uat978GapCount: UInt64 = 0
    var discardedUatBytes: UInt64 = 0
    var limitExhausted = false
}
