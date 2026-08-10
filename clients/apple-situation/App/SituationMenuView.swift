import PilotageRadioSource
import PilotageSituationCore
import SwiftUI

/// Everything that is not the map.
///
/// The map answers where things are, and it answers badly when status text, layer switches
/// and errors sit on top of it. One drawer holds the rest: what the client is receiving,
/// which layers draw, and which recorded flight is open.
struct SituationMenuView: View {
    @ObservedObject var model: SituationClientModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                receptionSection
                flightsSection
                layersSection
                if let message = model.errorMessage {
                    Section("Problems") {
                        Text(message).font(.footnote)
                    }
                }
            }
            .navigationTitle("Situation")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private var receptionSection: some View {
        Section {
            Toggle("ADS-B In", isOn: $model.adsbEnabled)
            if model.adsbEnabled {
                RadioStatusView(source: model.radioSource)
                    .listRowInsets(EdgeInsets())
            }
        } header: {
            Text("Reception")
        } footer: {
            Text(
                model.adsbEnabled
                    ? "The client claims the attached receivers."
                    : "The client claims no receiver and shows no traffic or weather from the air."
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
            if model.flights.isEmpty {
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
            }
        } header: {
            Text("Flights")
        } footer: {
            Text("A recorded flight draws on the map in place of live reception.")
        }
    }

    private var layersSection: some View {
        Section {
            LayerControlsView(
                layers: model.mapDisplay?.layers ?? [],
                setEnabled: model.setLayerEnabled
            )
            .listRowInsets(EdgeInsets())
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
