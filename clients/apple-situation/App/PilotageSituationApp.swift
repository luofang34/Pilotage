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
    @State private var camera = SituationCamera(headingDegrees: 0, pitchDegrees: 0)
    @State private var mapCommands: SituationMapCommands?
    @State private var follow: FollowMode = .idle
    @State private var modesPresented = false
    @State private var selectedMapModeID = MapMode.available.first?.id ?? "terrain"
    @Namespace private var mapControlNamespace
    @StateObject private var ownship = OwnshipModel()

    var body: some View {
        // The map owns the screen. Status, layers, reception and flights live in the
        // drawer, because a map covered in text answers "where" worse than a bare one.
        ZStack {
            // The map is the document, so it runs to the glass. A safe-area inset here
            // leaves a black band above and below that reads as a broken screen rather
            // than as a margin. The controls above it keep the inset.
            SituationMap(
                batch: model.mapDisplay,
                onFeatureTapped: model.selectTraffic,
                onCameraChanged: { camera = $0 },
                onReady: { mapCommands = $0 },
                onAttributions: model.observeMapAttributions,
                onMovedByReader: {
                    // A hand on the map ends both the following and the panel: the reader
                    // has said what they want to look at.
                    follow = .idle
                    modesPresented = false
                }
            )
            .ignoresSafeArea()
            // Each floating control is placed against the safe area by the same rule, so
            // none of them sits at a different distance from an edge than the others.
            ZStack {
                if let flight = model.replayingFlight {
                    ReplayBannerView(flight: flight, stop: model.stopReplay)
                        .mapControlPlacement(.topLeading)
                }
                MapControlsView(
                    camera: camera,
                    ownship: ownship.fix,
                    canLocate: ownship.canLocate,
                    follow: follow,
                    namespace: mapControlNamespace,
                    resetHeading: { mapCommands?.resetHeading() },
                    resetPitch: { mapCommands?.resetPitch() },
                    cycleFollow: cycleFollow,
                    modesPresented: $modesPresented,
                    modesContent: {
                        MapModesView(
                            modes: MapMode.available,
                            selectedModeID: $selectedMapModeID,
                            layers: model.mapDisplay?.layers ?? [],
                            setLayerEnabled: model.setLayerEnabled,
                            attributions: model.mapAttributions,
                            close: { modesPresented = false }
                        )
                    }
                )
                .mapControlPlacement(.topTrailing)
                PositionlessTrafficView(
                    items: model.mapDisplay?.positionlessTraffic ?? [],
                    select: model.selectTraffic
                )
                .mapControlPlacement(.bottomLeading)
                menuButton
                    .mapControlPlacement(.bottomTrailing)
            }
        }
        .sheet(isPresented: $menuPresented) {
            SituationMenuView(model: model)
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

private extension SituationContentView {
    /// Step through not following, following, and turning with the aircraft.
    ///
    /// Following is a mode rather than a jump, so the map keeps up as the position moves
    /// and stops the moment the reader drags it.
    func cycleFollow() {
        // Pressing this is also how a reader asks for permission the first time.
        ownship.requestPositionIfNeeded()
        follow = follow.next
        applyFollow()
    }

    /// Put the camera where the current mode says it belongs.
    func applyFollow() {
        guard let fix = ownship.fix else { return }
        if follow.followsPosition {
            mapCommands?.centre(fix.coordinate)
        }
        switch follow {
        case .heading:
            // A heading if the device or the aircraft has one, and course over the ground
            // only when neither does. They differ in wind, and the map should turn to
            // where the aircraft points rather than where it is drifting.
            if let heading = ownship.heading {
                mapCommands?.setHeading(heading.trueDegrees)
            }
        case .centred:
            mapCommands?.resetHeading()
        case .idle:
            break
        }
    }

    var menuButton: some View {
        Button {
            model.reloadFlights()
            menuPresented = true
        } label: {
            Image(systemName: "line.3.horizontal")
                .font(Metrics.controlGlyph)
                .frame(width: Metrics.control, height: Metrics.control)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .glassEffect(.clear.interactive(), in: .circle)
        // A reader must not have to open the drawer to learn that a receiver died. An
        // empty map with no mark reads as clear air.
        .overlay(alignment: .topTrailing) {
            if model.hasAttention {
                Circle()
                    .fill(.orange)
                    .frame(width: 12, height: 12)
                    .overlay(Circle().stroke(.black.opacity(0.5)))
            }
        }
        .accessibilityLabel(
            model.hasAttention
                ? "Situation menu, reception needs attention"
                : "Situation menu"
        )
    }
}

/// What the controls can ask of the map.
///
/// The controls live in the view hierarchy above the map and must not hold the map itself,
/// so they hold this instead.
@MainActor
struct SituationMapCommands {
    let resetHeading: () -> Void
    let resetPitch: () -> Void
    let centre: (CLLocationCoordinate2D) -> Void
    let setHeading: (Double) -> Void
}

private struct SituationMap: UIViewRepresentable {
    let batch: DisplayBatch?
    let onFeatureTapped: (String) -> Void
    let onCameraChanged: (SituationCamera) -> Void
    let onReady: (SituationMapCommands) -> Void
    let onAttributions: ([String]) -> Void
    let onMovedByReader: () -> Void

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
        view.onCameraChanged = onCameraChanged
        view.onStyleLoaded = { onAttributions($0.sourceAttributions) }
        view.onMovedByReader = onMovedByReader
        // The handle is published after the view exists, and the publish is deferred so it
        // does not change application state while the view tree is being built.
        DispatchQueue.main.async { [weak view] in
            guard let view else { return }
            onReady(
                SituationMapCommands(
                    resetHeading: { view.resetHeading() },
                    resetPitch: { view.resetPitch() },
                    centre: { view.centre(on: $0) },
                    setHeading: { view.setHeading($0) }
                )
            )
        }
        return view
    }

    func updateUIView(_ mapView: SituationMapView, context: Context) {
        mapView.onFeatureTapped = onFeatureTapped
        mapView.onCameraChanged = onCameraChanged
        mapView.onMovedByReader = onMovedByReader
        if let batch {
            mapView.apply(batch)
        }
    }
}
