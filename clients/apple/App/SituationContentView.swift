import CoreLocation
import PilotageMapLibreBinding
import PilotageCore
import SwiftUI

struct SituationContentView: View {
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.horizontalSizeClass) private var horizontalSizeClass
    @StateObject private var model = SituationClientModel()
    @StateObject private var hostLink = HostLinkModel()
    @State private var menuPresented = false
    /// Which regions show is the platform's own affordance: the split
    /// view's column state, never a custom toggle (ADR-0038).
    @State private var columnVisibility: NavigationSplitViewVisibility = .doubleColumn
    /// Which section the first sidebar level selects; the second level
    /// shows that section's content, in the two-level idiom of Mail.
    @AppStorage("pilotageSection") private var sectionRaw = OperatorSection.instruments.rawValue
    /// The tile promoted to the primary surface, empty for the map.
    @AppStorage("pilotagePrimarySurface") private var primaryTileId = ""
    /// Whether the flight control unit strip stands over the map.
    @AppStorage("pilotageFcuShown") private var fcuShown = false
    @AppStorage("pilotageInstrumentProfile") private var rackProfileId = "px4-flight"
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
    @StateObject private var missionPlan = MissionPlanModel()

    var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            List(selection: sectionSelection) {
                ForEach(OperatorSection.allCases) { section in
                    Label(section.title, systemImage: section.symbol)
                        .tag(section)
                }
            }
            .navigationTitle("Pilotage")
            .navigationSplitViewColumnWidth(min: 180, ideal: 220, max: 280)
        } content: {
            sectionContent
                .navigationSplitViewColumnWidth(min: 320, ideal: 400, max: 560)
        } detail: {
            primarySurface
                .toolbar(.hidden, for: .navigationBar)
        }
        .navigationSplitViewStyle(.balanced)
        .background(.black)
        .onGeometryChange(for: CGSize.self) { proxy in
            proxy.size
        } action: { size in
            windowHeight = size.height
        }
        .onAppear {
            if LaunchRequest.openInstruments {
                columnVisibility = .doubleColumn
                sectionRaw = OperatorSection.instruments.rawValue
                hostLink.connect(
                    url: UserDefaults.standard.string(forKey: "pilotageHostUrl") ?? "",
                    certificateSha256Hex:
                        UserDefaults.standard.string(forKey: "pilotageHostCertHash") ?? ""
                )
            }
        }
    }

    /// The panel collapse in Apple's own idiom: one floating glass
    /// circle whose arrows point the way the layout will move.
    private var columnsToggle: some View {
        Button {
            withAnimation {
                columnVisibility = columnVisibility == .detailOnly ? .all : .detailOnly
            }
        } label: {
            Image(systemName: columnVisibility == .detailOnly
                ? "arrow.up.left.and.arrow.down.right"
                : "arrow.down.right.and.arrow.up.left")
                .font(Metrics.controlGlyph)
                .frame(width: Metrics.control, height: Metrics.control)
                .glassEffect(.regular, in: Circle())
        }
        .buttonStyle(.plain)
    }

    private var sectionSelection: Binding<OperatorSection?> {
        // Nil must survive the round trip: a collapsed presentation
        // writes nil when the user navigates back, and coercing it
        // re-arms the selection under their finger.
        Binding(
            get: { OperatorSection(rawValue: sectionRaw) },
            set: { sectionRaw = $0?.rawValue ?? "" }
        )
    }

    /// The second sidebar level: the selected section's own content.
    @ViewBuilder
    private var sectionContent: some View {
        switch OperatorSection(rawValue: sectionRaw) ?? .instruments {
        case .instruments:
            InstrumentRackView(
                model: hostLink,
                primaryTileId: $primaryTileId,
                fcuShown: $fcuShown
            )
            .navigationTitle("Instruments")
            .navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(.black, for: .navigationBar)
            .toolbarBackground(.visible, for: .navigationBar)
        case .mission:
            MissionPlannerView(
                controllable: hostLink.catalog?.offersFlightControl == true,
                plan: missionPlan
            )
                .navigationTitle("Mission Planner")
                .navigationBarTitleDisplayMode(.inline)
        }
    }

    /// The primary surface: the map, or whichever tile the operator
    /// swapped into it. The way back floats where the leaving control
    /// sat.
    @ViewBuilder
    private var primarySurface: some View {
        let profile = InstrumentProfile.selected(storedId: rackProfileId)
        if let tile = profile.tiles.first(where: { $0.id == primaryTileId }) {
            PromotedSurfaceView(model: hostLink, tile: tile) {
                withAnimation { primaryTileId = "" }
            }
        } else {
            mapSurface
        }
    }

    private var mapSurface: some View {
        // The map owns its half. Status, layers, reception and flights live in the
        // drawer, because a map covered in text answers "where" worse than a bare one.
        mapStack
    }

    private var mapStack: some View {
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
                // One top row holds every top-center control, so nothing
                // can occupy the same slot twice. The toggle leads; the
                // strip centers in what remains.
                HStack(spacing: 12) {
                    columnsToggle
                    Spacer(minLength: 12)
                    if fcuShown, hostLink.catalog?.offersFlightControl == true {
                        FlightControlUnit { withAnimation { fcuShown = false } }
                    }
                    Spacer(minLength: 12)
                    Color.clear.frame(width: Metrics.control, height: 1)
                }
                .mapControlPlacement(.top)
                MissionPlannerBar(model: hostLink, plan: missionPlan)
                    .mapControlPlacement(.bottom)
            }
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
