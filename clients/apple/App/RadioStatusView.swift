import PilotageRadioSource
import SwiftUI
import UIKit

struct RadioStatusView: View {
    let source: RadioSourceSnapshot

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(title, systemImage: symbol)
                .font(.headline)
            if source.availability == .driverDisabled {
                Button("Open \(applicationName) Settings") {
                    openApplicationSettings()
                }
            }
            ForEach(source.bandFailures) { failure in
                Text("\(name(for: failure.id)): \(failure.detail)")
                    .font(.caption)
            }
            ForEach(source.receivers) { receiver in
                receiverView(receiver)
            }
        }
        .padding(12)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
    }

    /// One band, in the words a pilot uses about it.
    ///
    /// The counters underneath are for a fault being chased, not for a flight, so they
    /// are folded away. A reader who opens them has already decided something is wrong.
    private func receiverView(_ receiver: RadioReceiver) -> some View {
        DisclosureGroup {
            Text(diagnosticLine(receiver))
                .font(.caption2.monospacedDigit())
                .foregroundStyle(.secondary)
        } label: {
            HStack(spacing: 8) {
                Text(name(for: receiver.band))
                    .font(.subheadline.weight(.semibold))
                Spacer(minLength: 8)
                Text(plainState(receiver.availability))
                    .font(.subheadline)
                    .foregroundStyle(tint(receiver.availability))
            }
        }
    }

    /// What a band is doing, said the way a person would say it.
    private func plainState(_ availability: RadioAvailability) -> String {
        switch availability {
        case .streaming: "Receiving"
        case .ready: "Attached, nothing yet"
        case .checking: "Looking"
        case .unplugged: "Not attached"
        case .suspended: "Paused"
        case .underpowered: "Not enough power"
        case .deviceRemoved: "Unplugged"
        case .driverDisabled, .permissionDenied: "Driver off"
        case .enumerationFailure, .endpointFailure: "Stopped"
        }
    }

    private func tint(_ availability: RadioAvailability) -> Color {
        switch availability {
        case .streaming: .green
        case .ready, .checking, .suspended, .unplugged: .secondary
        default: .orange
        }
    }

    /// The counters, for a fault being chased.
    private func diagnosticLine(_ receiver: RadioReceiver) -> String {
        let values = receiver.diagnostics
        return "Queue \(values.queueDepth)/\(values.queueCapacity) · "
            + "completed \(values.completedTransfers) / \(values.completedBytes) B · "
            + "dropped \(values.droppedTransfers) / \(values.droppedBytes) B · "
            + "I/O \(values.ioErrors) · generation \(values.reconnectGeneration)\n"
            + "1090 gaps \(values.adsb1090GapSamples) · "
            + "UAT gaps \(values.uat978GapCount) · "
            + "UAT resync \(values.discardedUatBytes) B · "
            + "drain limit \(values.drainLimitExhaustions)"
    }

    private var title: String {
        switch source.availability {
        case .checking: "Looking for a receiver"
        case .permissionDenied(let detail):
            "Driver permission denied. Check the App ID profiles. \(detail)"
        case .driverDisabled:
            "Turn on the \(applicationName) driver in Settings"
        case .unplugged: "No receiver attached"
        case .ready: "Receiver attached"
        case .streaming: "USB receiver"
        case .suspended: "Radio reception stopped while the scene is inactive"
        case .underpowered: "Use a powered USB hub"
        case .enumerationFailure(let detail): "USB enumeration failed. \(detail)"
        case .endpointFailure(let detail): "USB endpoint failed. \(detail)"
        case .deviceRemoved: "Receiver unplugged"
        }
    }

    private var symbol: String {
        switch source.availability {
        case .streaming: "antenna.radiowaves.left.and.right"
        case .ready, .checking: "antenna.radiowaves.left.and.right.slash"
        case .suspended: "pause.circle"
        default: "exclamationmark.triangle"
        }
    }

    private func name(for band: RadioBand) -> String {
        switch band {
        case .adsb1090: "1090 MHz"
        case .uat978: "978 MHz"
        }
    }

    private func stateName(_ availability: RadioAvailability) -> String {
        switch availability {
        case .checking: "checking"
        case .ready: "ready"
        case .streaming: "streaming"
        case .suspended: "suspended"
        case .permissionDenied: "permission denied"
        case .driverDisabled: "driver disabled"
        case .unplugged: "unplugged"
        case .underpowered: "underpowered"
        case .enumerationFailure: "enumeration failure"
        case .endpointFailure: "endpoint failure"
        case .deviceRemoved: "removed"
        }
    }

    private var applicationName: String {
        (Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String)
            ?? (Bundle.main.object(forInfoDictionaryKey: "CFBundleName") as? String)
            ?? "Pilotage Situation"
    }

    private func openApplicationSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }
}
