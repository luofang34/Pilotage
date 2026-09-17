import PilotageCore
import SwiftUI

struct AeroLinkApplianceView: View {
    let snapshot: AeroLinkApplianceSnapshot
    let isRecentered: Bool
    let recenter: () -> Void
    let clearRecenter: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Label(stateLabel, systemImage: stateSymbol)
                .foregroundStyle(stateTint)
            if let name = snapshot.name {
                Text(name).font(.subheadline.weight(.semibold))
            }
            if let navigation = snapshot.navigation {
                navigationView(navigation)
            }
            diagnostics
        }
    }

    @ViewBuilder
    private func navigationView(_ navigation: AeroLinkApplianceNavigation) -> some View {
        if let roll = navigation.rollDegrees, let pitch = navigation.pitchDegrees {
            Text(
                "Bank \(roll.formatted(.number.precision(.fractionLength(1))))° · "
                    + "Pitch \(pitch.formatted(.number.precision(.fractionLength(1))))°"
            )
            .monospacedDigit()
            HStack {
                Button("Recenter AHRS", action: recenter)
                if isRecentered {
                    Button("Clear Recenter", action: clearRecenter)
                }
            }
        }
        if let altitude = navigation.pressureAltitudeFeet {
            Text("Pressure altitude \(altitude.formatted(.number.precision(.fractionLength(0)))) ft")
                .monospacedDigit()
        }
        if let heading = navigation.headingDegrees,
           let reference = navigation.headingReference {
            Text(
                "\(headingLabel(reference)) "
                    + "\(heading.formatted(.number.precision(.fractionLength(1))))°"
            )
            .monospacedDigit()
        } else if let track = navigation.groundTrackDegreesTrue {
            Text("GPS track \(track.formatted(.number.precision(.fractionLength(1))))° true")
                .monospacedDigit()
        } else {
            Text("Heading unavailable").foregroundStyle(.secondary)
        }
    }

    private var diagnostics: some View {
        DisclosureGroup("Link counters") {
            Text(
                "\(snapshot.bytesConsumed) B · \(snapshot.validFrames) valid frames · "
                    + "\(snapshot.trafficReports) traffic reports · "
                    + "\(snapshot.crcErrors) CRC errors · "
                    + "\(snapshot.invalidFrames) invalid frames · "
                    + "\(snapshot.deferredUplinkMessages) deferred UAT uplinks · "
                    + "\(snapshot.unsupportedMessages) unsupported messages"
            )
            .font(.caption2.monospacedDigit())
            .foregroundStyle(.secondary)
        }
    }

    private func headingLabel(_ reference: Gdl90HeadingReferenceValue) -> String {
        switch reference {
        case .trueNorth: "True heading"
        case .magneticNorth: "Magnetic heading"
        }
    }

    private var stateLabel: String {
        switch snapshot.state {
        case .off: "Bluetooth appliance is off"
        case .checking: "Looking for an AeroLink appliance"
        case .connecting: "Connecting"
        case .ready: "Connected"
        case .streaming: "Receiving over Bluetooth"
        case .unavailable(let detail): detail
        }
    }

    private var stateSymbol: String {
        switch snapshot.state {
        case .streaming: "antenna.radiowaves.left.and.right"
        case .ready, .connecting: "dot.radiowaves.left.and.right"
        case .checking: "magnifyingglass"
        case .off: "pause.circle"
        case .unavailable: "exclamationmark.triangle"
        }
    }

    private var stateTint: Color {
        switch snapshot.state {
        case .streaming: .green
        case .unavailable: .orange
        default: .secondary
        }
    }
}
