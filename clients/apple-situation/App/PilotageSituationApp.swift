import CoreLocation
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
    @State private var menuPresented = false

    var body: some View {
        // The map owns the screen. Status, layers, reception and flights live in the
        // drawer, because a map covered in text answers "where" worse than a bare one.
        ZStack {
            // The map is the document, so it runs to the glass. A safe-area inset here
            // leaves a black band above and below that reads as a broken screen rather
            // than as a margin. The controls above it keep the inset.
            SituationMap(batch: model.mapDisplay, onFeatureTapped: model.selectTraffic)
                .ignoresSafeArea()
            VStack {
                HStack(alignment: .top) {
                    if let flight = model.replayingFlight {
                        ReplayBannerView(flight: flight, stop: model.stopReplay)
                    }
                    Spacer()
                    Button {
                        model.reloadFlights()
                        menuPresented = true
                    } label: {
                        Image(systemName: "line.3.horizontal")
                            .font(.title2)
                            .padding(12)
                            .background(.ultraThinMaterial, in: Circle())
                            // A reader must not have to open the drawer to learn that a
                            // receiver died. An empty map with no mark reads as clear air.
                            .overlay(alignment: .topTrailing) {
                                if model.hasAttention {
                                    Circle()
                                        .fill(.orange)
                                        .frame(width: 12, height: 12)
                                        .overlay(Circle().stroke(.black.opacity(0.5)))
                                }
                            }
                    }
                    .accessibilityLabel(
                        model.hasAttention
                            ? "Situation menu, reception needs attention"
                            : "Situation menu"
                    )
                }
                Spacer()
                HStack {
                    PositionlessTrafficView(
                        items: model.mapDisplay?.positionlessTraffic ?? [],
                        select: model.selectTraffic
                    )
                    Spacer()
                }
            }
            .padding()
        }
        .sheet(isPresented: $menuPresented) {
            SituationMenuView(model: model)
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
        // Open over the ground the terrain archive covers rather than on the Atlantic, and
        // let the map be pinched out until the whole world is on screen.
        view.initialCenter = CLLocationCoordinate2D(latitude: 40.5, longitude: -76.5)
        view.initialZoomLevel = 6
        view.minimumZoomLevel = 0
        view.maximumZoomLevel = SituationStyleResource.maximumZoomLevel
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
