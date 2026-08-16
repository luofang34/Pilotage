import CoreLocation
import PilotageMapLibreBinding
import PilotageCore
import SwiftUI

@main
struct PilotageApp: App {
    var body: some Scene {
        WindowGroup {
            SituationContentView()
        }
    }
}

/// What a launch asked the application to do before a hand touched it.
///
/// Only in a debug build. A screen that can be opened from outside is a way in, and the
/// shipped application has no reason to offer one. It exists so a window at an awkward
/// size can be photographed and measured without somebody holding the tablet.
enum LaunchRequest {
    static var openMapModes: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-OpenMapModes")
        #else
        false
        #endif
    }

    /// Open the Instruments destination and connect with the persisted
    /// facts, so a headless harness can photograph a live panel.
    static var openInstruments: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-OpenInstruments")
        #else
        false
        #endif
    }
}

private struct SituationContentView: View {
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @StateObject private var model = SituationClientModel()
    @StateObject private var hostLink = HostLinkModel()
    @State private var menuPresented = false
    @State private var instrumentsPresented = LaunchRequest.openInstruments
    @State private var camera = SituationCamera(headingDegrees: 0, pitchDegrees: 0)
    @State private var mapCommands: SituationMapCommands?
    @State private var modesPresented = LaunchRequest.openMapModes
    /// How tall the panel wants to be, so a sheet can be exactly that tall.
    @State private var modesHeight: CGFloat = 360
    /// How tall the window is, so a sheet cannot ask to be taller than one.
    @State private var windowHeight: CGFloat = 800
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
                    ownship.follow = .idle
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
                    follow: ownship.follow,
                    namespace: mapControlNamespace,
                    resetHeading: { mapCommands?.resetHeading() },
                    resetPitch: { mapCommands?.resetPitch() },
                    cycleFollow: cycleFollow,
                    modesPresented: $modesPresented,
                    modesGrowFromControls: modesFitBesideTheMap,
                    modesContent: { mapModes(fixedWidth: true) }
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
        .onGeometryChange(for: CGFloat.self) { proxy in
            proxy.size.height
        } action: { height in
            windowHeight = height
        }
        .task {
            // The controls report what they are doing through the model, so a run can be
            // read from the file it leaves rather than from the screen.
            model.currentOwnship = { [ownship] in
                (
                    ownship.fix,
                    ownship.heading,
                    ownship.follow,
                    ownship.deviceAuthorisation,
                    ownship.deviceLocationEnabled
                )
            }
            model.onOwnship = { [ownship] reported in
                ownship.observeAircraft(reported.map(OwnshipFix.init))
            }
            ownship.startIfPermitted()
            model.refreshEvidence()
        }
        .onReceive(
            NotificationCenter.default.publisher(
                for: UIDevice.orientationDidChangeNotification
            )
        ) { _ in
            ownship.refreshOrientation()
        }
        // Following is a mode, not a jump: the camera keeps up as the aircraft moves,
        // without the ease a press gets, because a course correction eased on every
        // reading leaves the map permanently behind the aircraft.
        .onChange(of: ownship.fix) { _, _ in applyFollow(animated: false) }
        .onChange(of: ownship.heading) { _, _ in applyFollow(animated: false) }
        .onChange(of: ownship.follow) { _, _ in model.refreshEvidence() }
        // The file records what changed about the answer, not every degree of it.
        .onChange(of: ownship.heading?.source) { _, _ in model.refreshEvidence() }
        .onChange(of: ownship.fix == nil) { _, _ in model.refreshEvidence() }
        .onChange(of: ownship.deviceAuthorisation) { _, _ in model.refreshEvidence() }
        // Narrow, the panel would cover the map it describes, so it comes up from the
        // bottom edge at the full width instead, which is where a reader's thumb is and
        // what the platform does with anything that cannot fit beside its subject.
        .sheet(isPresented: Binding(
            get: { modesPresented && !modesFitBesideTheMap },
            set: { presented in if !presented { modesPresented = false } }
        )) {
            // A sheet is sized by its detent, not by asking it to fit: left to itself it
            // takes the whole screen and centres the panel in it, which is the empty band
            // above and below that read as a second surface. The panel's own height is
            // intrinsic, so measuring it cannot chase the sheet it is setting.
            // Short, the panel wants more height than the window has, and a detent taller
            // than its window crops the panel at both ends rather than shrinking it. The
            // ask is capped at the window and the content scrolls for the rest, which it
            // only does when there is a rest.
            ScrollView {
                mapModes(fixedWidth: false, drawsSurface: false)
                    .onGeometryChange(for: CGFloat.self) { proxy in
                        proxy.size.height
                    } action: { height in
                        modesHeight = height
                    }
            }
            .scrollBounceBehavior(.basedOnSize)
            .presentationDetents([.height(min(modesHeight, windowHeight * 0.92))])
            .presentationBackground(.regularMaterial)
            .presentationDragIndicator(.hidden)
        }
        .sheet(isPresented: $menuPresented) {
            SituationMenuView(model: model, hostLink: hostLink)
        }
        .fullScreenCover(isPresented: $instrumentsPresented) {
            NavigationStack {
                InstrumentsView(model: hostLink)
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Done") { instrumentsPresented = false }
                        }
                    }
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

private extension SituationContentView {
    /// Whether the panel can sit beside the map rather than over it.
    ///
    /// The platform's own answer to "is there room for two things across", which is the
    /// question being asked. A width in points would have to be picked and then re-picked
    /// for every window size a reader can drag to.
    var modesFitBesideTheMap: Bool { horizontalSizeClass == .regular }

    /// The panel itself, wherever it is shown from.
    ///
    /// One view for both presentations, because a reader who learns it narrow should not
    /// have to learn it again wide.
    @ViewBuilder func mapModes(fixedWidth: Bool, drawsSurface: Bool = true) -> some View {
        MapModesView(
            modes: MapMode.available,
            selectedModeID: $selectedMapModeID,
            layers: model.mapDisplay?.layers ?? [],
            setLayerEnabled: model.setLayerEnabled,
            attributions: model.mapAttributions,
            close: { modesPresented = false },
            fixedWidth: fixedWidth,
            drawsSurface: drawsSurface
        )
    }

    /// Step through not following, following, and turning with the aircraft.
    ///
    /// Following is a mode rather than a jump, so the map keeps up as the position moves
    /// and stops the moment the reader drags it.
    func cycleFollow() {
        // Pressing this is also how a reader asks for permission the first time.
        ownship.requestPositionIfNeeded()
        let wasIdle = ownship.follow == .idle
        ownship.follow = ownship.follow.next
        // Only the press that starts following sets the width. A reader who has zoomed
        // out to look ahead and then presses again to turn the map with the aircraft has
        // not asked to be zoomed back in.
        if wasIdle, let fix = ownship.fix {
            mapCommands?.centreAndFrame(fix.coordinate, true)
        }
        applyFollow(animated: true)
    }

    /// Put the camera where the current mode says it belongs.
    func applyFollow(animated: Bool) {
        // Position and heading are separate answers. Coupling them meant a map that would
        // not turn with the aircraft until it also knew where the aircraft was, and indoors
        // that is most of the time.
        if ownship.follow.followsPosition, let fix = ownship.fix {
            mapCommands?.centre(fix.coordinate, animated)
        }
        switch ownship.follow {
        case .heading:
            // A heading if the device or the aircraft has one, and course over the ground
            // only when neither does. They differ in wind, and the map should turn to
            // where the aircraft points rather than where it is drifting.
            if let heading = ownship.heading {
                mapCommands?.setHeading(heading.trueDegrees, animated)
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
    let centre: (CLLocationCoordinate2D, Bool) -> Void
    /// Centre and set how much ground is on screen, for a reader who asked to be found.
    let centreAndFrame: (CLLocationCoordinate2D, Bool) -> Void
    let setHeading: (Double, Bool) -> Void
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
                    centre: { view.centre(on: $0, animated: $1) },
                    centreAndFrame: {
                        view.centre(
                            on: $0,
                            widthNauticalMiles: SituationMapView.ownshipWidthNauticalMiles,
                            animated: $1
                        )
                    },
                    setHeading: { view.setHeading($0, animated: $1) }
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
