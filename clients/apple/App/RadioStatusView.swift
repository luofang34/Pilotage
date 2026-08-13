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

    private func receiverView(_ receiver: RadioReceiver) -> some View {
        let values = receiver.diagnostics
        return VStack(alignment: .leading, spacing: 2) {
            Text("\(name(for: receiver.band)): \(stateName(receiver.availability))")
                .font(.subheadline.weight(.semibold))
            Text(
                "Queue \(values.queueDepth)/\(values.queueCapacity) · "
                    + "completed \(values.completedTransfers) / \(values.completedBytes) B · "
                    + "dropped \(values.droppedTransfers) / \(values.droppedBytes) B · "
                    + "I/O \(values.ioErrors) · generation \(values.reconnectGeneration)"
            )
            .font(.caption2.monospacedDigit())
            Text(
                "1090 gaps \(values.adsb1090GapSamples) · "
                    + "UAT gaps \(values.uat978GapCount) · "
                    + "UAT resync \(values.discardedUatBytes) B · "
                    + "drain limit \(values.drainLimitExhaustions)"
            )
            .font(.caption2.monospacedDigit())
        }
    }

    private var title: String {
        switch source.availability {
        case .checking: "Checking the AeroLink driver"
        case .permissionDenied(let detail):
            "Driver permission denied. Check the App ID profiles. \(detail)"
        case .driverDisabled:
            "Enable \(applicationName)'s AeroLink driver in Settings"
        case .unplugged: "No AeroLink receiver is attached"
        case .ready: "AeroLink receiver ready"
        case .streaming: "AeroLink reception live"
        case .suspended: "Radio reception stopped while the scene is inactive"
        case .underpowered: "Use a powered USB hub"
        case .enumerationFailure(let detail): "USB enumeration failed. \(detail)"
        case .endpointFailure(let detail): "USB endpoint failed. \(detail)"
        case .deviceRemoved: "AeroLink receiver was removed"
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
