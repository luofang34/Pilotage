import PilotageRadioSource
import PilotageCore
import SwiftUI

/// Everything that is not the map.
///
/// The map answers where things are, and it answers badly when status text, layer switches
/// and errors sit on top of it. One drawer holds the rest: what the client is receiving,
/// which layers draw, and which recorded flight is open.
/// One answer about reception, in the words a reader needs and no more.
struct ReceptionSummary {
    let title: String
    let symbol: String
    let tint: Color
    let explanation: String
    /// Whether the state asks something of the reader, and so needs its detail shown.
    let wantsDetail: Bool
}

struct SituationMenuView: View {
    @ObservedObject var model: SituationClientModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                receptionSection
                flightsSection
                if let message = model.errorMessage {
                    Section("Problems") {
                        Text(message).font(.footnote)
                    }
                }
            }
            .navigationTitle("More")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    /// What the radios are doing, said before it is explained.
    ///
    /// A reader turning this on wants one of three answers: nothing is plugged in,
    /// something is and it is working, or something was and it stopped. The detail below
    /// is for the third, which is the only one that asks anything of them.
    private var receptionSection: some View {
        Section {
            Toggle("ADS-B In", isOn: $model.adsbEnabled)
            if model.adsbEnabled {
                Label(reception.title, systemImage: reception.symbol)
                    .foregroundStyle(reception.tint)
                if reception.wantsDetail {
                    RadioStatusView(source: model.radioSource)
                        .listRowInsets(EdgeInsets())
                }
            }
        } header: {
            Text("Reception")
        } footer: {
            Text(reception.explanation)
        }
    }

    /// The one answer the reception state gives, and whether it needs explaining.
    private var reception: ReceptionSummary {
        guard model.adsbEnabled else {
            return ReceptionSummary(
                title: "Off",
                symbol: "antenna.radiowaves.left.and.right.slash",
                tint: .secondary,
                explanation: "The client claims no receiver, and shows no traffic or "
                    + "weather from the air.",
                wantsDetail: false
            )
        }
        switch model.radioSource.availability {
        case .checking:
            return ReceptionSummary(
                title: "Looking for a receiver",
                symbol: "antenna.radiowaves.left.and.right",
                tint: .secondary,
                explanation: "The client is checking the driver and whatever is attached.",
                wantsDetail: false
            )
        case .unplugged:
            return ReceptionSummary(
                title: "No receiver attached",
                symbol: "cable.connector.slash",
                tint: .secondary,
                explanation: "Attach a receiver to the tablet. Until one is attached the "
                    + "map shows only what it already holds.",
                wantsDetail: false
            )
        case .ready:
            return ReceptionSummary(
                title: "Receiver attached",
                symbol: "cable.connector",
                tint: .primary,
                explanation: "A receiver is attached and has not sent anything yet.",
                wantsDetail: true
            )
        case .streaming:
            return ReceptionSummary(
                title: "Receiving",
                symbol: "antenna.radiowaves.left.and.right",
                tint: .green,
                explanation: "Traffic and weather on the map are coming from the air.",
                wantsDetail: true
            )
        case .suspended:
            return ReceptionSummary(
                title: "Paused",
                symbol: "pause.circle",
                tint: .secondary,
                explanation: "Reception stops while the application is not on screen.",
                wantsDetail: false
            )
        case .driverDisabled, .permissionDenied:
            return ReceptionSummary(
                title: "Driver switched off",
                symbol: "exclamationmark.triangle",
                tint: .orange,
                explanation: "The driver is installed but not allowed to run. It is "
                    + "turned on in Settings, under this application.",
                wantsDetail: true
            )
        default:
            return ReceptionSummary(
                title: "Receiver stopped",
                symbol: "exclamationmark.triangle.fill",
                tint: .orange,
                explanation: "The receiver was attached and stopped. A halted receiver "
                    + "is cleared by unplugging it and plugging it back in.",
                wantsDetail: true
            )
        }
    }

    private var flightsSection: some View {
        Section {
            if let flight = model.replayingFlight {
                Button(role: .destructive) {
                    model.stopReplay()
                } label: {
                    Label("Close \(flight.title)", systemImage: "stop.circle")
                }
            }
            recordButton
            if model.flights.isEmpty, !model.isRecording {
                Text("No recorded flight is on this device.")
                    .foregroundStyle(.secondary)
            }
            ForEach(model.flights) { flight in
                Button {
                    model.startReplay(flight)
                    dismiss()
                } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(flight.title)
                        Text(flight.subtitle)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .disabled(model.replayingFlight == flight)
                .swipeActions(edge: .trailing) {
                    // Refused while it is being replayed, so the swipe is not offered then
                    // rather than offered and then ignored.
                    if model.replayingFlight != flight {
                        Button(role: .destructive) {
                            model.deleteFlight(flight)
                        } label: {
                            Label("Delete", systemImage: "trash")
                        }
                    }
                }
            }
        } header: {
            Text("Flights")
        } footer: {
            Text(
                model.isRecording
                    ? "Everything the radios receive is being written to a new flight."
                    : "A recorded flight draws on the map in place of live reception. "
                        + "Swipe a flight to delete it."
            )
        }
    }

    /// Start or stop writing what the radios receive.
    @ViewBuilder private var recordButton: some View {
        if model.isRecording {
            Button(role: .destructive) {
                model.stopRecording()
            } label: {
                Label(
                    "Stop recording — \(model.recordedEvents) received",
                    systemImage: "stop.circle.fill"
                )
            }
        } else {
            Button {
                model.startRecording()
            } label: {
                Label("Record this flight", systemImage: "record.circle")
            }
            // Nothing arrives to record with the radios unclaimed, and a recording that
            // captures nothing is worse than a control that says why it cannot run.
            .disabled(!model.adsbEnabled || model.replayingFlight != nil)
        }
    }
}

/// The complete notice for every source the map draws.
struct MapAttributionView: View {
    let notices: [String]
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                if notices.isEmpty {
                    Text("The map is drawing no source that states a notice.")
                        .foregroundStyle(.secondary)
                }
                ForEach(notices, id: \.self) { notice in
                    Text(notice)
                        .font(.footnote)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .navigationTitle("Map data")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}

/// States that the map is a recording and not the air around the aircraft.
struct ReplayBannerView: View {
    let flight: Flight
    let stop: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "play.rectangle.fill")
            VStack(alignment: .leading, spacing: 1) {
                Text("Replay — \(flight.title)")
                    .font(.subheadline.weight(.semibold))
                Text("Not live reception")
                    .font(.caption)
            }
            Button("Exit", action: stop)
                .buttonStyle(.bordered)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.orange.opacity(0.85), in: Capsule())
        .foregroundStyle(.black)
    }
}
