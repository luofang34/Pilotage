import PilotageMapLibreBinding
import PilotageSituationCore
import SwiftUI

@main
struct PilotageSituationApp: App {
    var body: some Scene {
        WindowGroup {
            SituationContentView()
        }
    }
}

private struct SituationContentView: View {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var model = SituationClientModel()

    var body: some View {
        ZStack {
            SituationMap(batch: model.display, onFeatureTapped: model.selectTraffic)
            VStack {
                HStack(alignment: .top) {
                    VStack(spacing: 8) {
                        RadioStatusView(source: model.radioSource)
                        if let message = model.errorMessage {
                            Text(message)
                                .padding(12)
                                .background(
                                    .ultraThinMaterial,
                                    in: RoundedRectangle(cornerRadius: 8)
                                )
                        }
                    }
                    Spacer()
                    LayerControlsView(
                        layers: model.display?.layers ?? [],
                        setEnabled: model.setLayerEnabled
                    )
                }
                Spacer()
                HStack {
                    PositionlessTrafficView(
                        items: model.display?.positionlessTraffic ?? [],
                        select: model.selectTraffic
                    )
                    Spacer()
                }
            }
            .padding()
        }
        .task(id: scenePhase) {
            if scenePhase == .active {
                await model.activate()
            } else {
                await model.suspend()
            }
        }
        .sheet(
            isPresented: Binding(
                get: { model.selectedTraffic != nil },
                set: { presented in
                    if !presented {
                        model.clearTrafficSelection()
                    }
                }
            )
        ) {
            if let detail = model.selectedTraffic {
                TrafficDetailView(detail: detail)
            }
        }
    }
}

private struct SituationMap: UIViewRepresentable {
    let batch: DisplayBatch?
    let onFeatureTapped: (String) -> Void

    func makeUIView(context: Context) -> SituationMapView {
        let styleJSON = (try? SituationStyleResource.load())
            ?? SituationStyleResource.fallbackJSON
        let view = SituationMapView(styleJSON: styleJSON)
        view.initialPitchDegrees = 55
        view.baseLayerIdentifiers = ["terrain-base": "pilotage-terrain-hillshade"]
        view.onFeatureTapped = onFeatureTapped
        return view
    }

    func updateUIView(_ mapView: SituationMapView, context: Context) {
        mapView.onFeatureTapped = onFeatureTapped
        if let batch {
            mapView.apply(batch)
        }
    }
}
