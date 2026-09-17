import Foundation
import PilotageCore

enum AeroLinkApplianceConnectionState: Equatable {
    case off
    case checking
    case connecting
    case ready
    case streaming
    case unavailable(String)
}

struct AeroLinkApplianceNavigation: Equatable {
    let rollDegrees: Double?
    let pitchDegrees: Double?
    let rawRollDegrees: Double?
    let rawPitchDegrees: Double?
    let headingDegrees: Double?
    let headingReference: Gdl90HeadingReferenceValue?
    let pressureAltitudeFeet: Double?
    let verticalSpeedFeetPerMinute: Double?
    let latitudeDegrees: Double?
    let longitudeDegrees: Double?
    let groundTrackDegreesTrue: Double?
}

struct AeroLinkApplianceSnapshot: Equatable {
    var state: AeroLinkApplianceConnectionState
    var name: String?
    var identifier: String?
    var navigation: AeroLinkApplianceNavigation?
    var bytesConsumed: UInt64
    var validFrames: UInt64
    var crcErrors: UInt64
    var invalidFrames: UInt64
    var trafficReports: UInt64
    var deferredUplinkMessages: UInt64
    var unsupportedMessages: UInt64

    static let off = AeroLinkApplianceSnapshot(
        state: .off,
        name: nil,
        identifier: nil,
        navigation: nil,
        bytesConsumed: 0,
        validFrames: 0,
        crcErrors: 0,
        invalidFrames: 0,
        trafficReports: 0,
        deferredUplinkMessages: 0,
        unsupportedMessages: 0
    )
}

struct AeroLinkBLEConnection: Equatable {
    let name: String
    let identifier: String
    let sourceId: UInt32
    let reconnectGeneration: UInt64
}

enum AeroLinkBLEEvent {
    case state(AeroLinkApplianceConnectionState, AeroLinkBLEConnection?)
    case bytes(Data, AeroLinkBLEConnection)
}
