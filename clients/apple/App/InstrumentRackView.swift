import IndicateAppleDisplay
import PilotageCore
import SwiftUI

/// The instrument rack: a configurable vertical stack of live tiles
/// beside the map, in the operator's chosen profile.
///
/// The rack is a working surface, not a page: it shares the screen with
/// the map, and the map itself can be stood down when the flight is all
/// instruments. Each tile resolves against the registry at paint time;
/// what this build cannot show states its reason in place.
struct InstrumentRackView: View {
    @ObservedObject var model: HostLinkModel
    @AppStorage("pilotageInstrumentProfile") private var profileId = "px4-flight"
    /// The tile promoted to the primary surface, empty for the map.
    /// The column offers the swap; the split view renders the result.
    @Binding var primaryTileId: String
    /// Whether the flight control unit strip stands over the map.
    @Binding var fcuShown: Bool
    @State private var connectPresented = false
    /// The video source the operator switched to, overriding the
    /// profile's own until the profile changes.
    @AppStorage("pilotageVideoSource") private var videoSourceOverride = ""
    /// The tile granted the whole rack column, when one is. One
    /// affordance, in place, reversible: the same button grants and
    /// returns the focus, and the map split stays a separate switch.
    @State private var focusedTileId: String?
    /// A reset press awaiting its confirmation; a simulation rewind
    /// mid-flight is one accidental thumb away on a touch screen.
    @State private var resetAsked = false
    /// The session log sheet, opened from the status line.
    @State private var logShown = false

    /// Fixed control-bar button widths. The bar must not reflow when a
    /// state swap changes a label (Release ↔ Request control), and a
    /// squeezed rack column must clip a button rather than fold its
    /// title into a one-letter-per-line column.
    private static let fcuButtonWidth: CGFloat = 44
    private static let resetButtonWidth: CGFloat = 48
    private static let leaseButtonWidth: CGFloat = 64

    /// Sources a vehicle can offer today. A source catalog will replace
    /// this list; the switcher's shape stays.
    static let videoSources = ["gimbal", "fpv", "chase"]

    /// The stack's total height per point of width: each tile's own
    /// height-over-width, summed over the tiles that actually render.
    static func aspectSum(of tiles: [InstrumentTile], model: HostLinkModel) -> CGFloat {
        tiles.reduce(CGFloat(0)) { sum, tile in
            switch tile {
            case .video:
                return sum + 9.0 / 16.0
            case .panel(let id):
                guard let choice = model.panelChoice(forTileId: id),
                      choice.descriptor.designWidth > 0
                else { return sum + 3.0 / 4.0 }
                return sum + CGFloat(choice.descriptor.designHeight)
                    / CGFloat(choice.descriptor.designWidth)
            }
        }
    }

    private var profile: InstrumentProfile {
        InstrumentProfile.selected(storedId: profileId)
    }

    var body: some View {
        VStack(spacing: 8) {
            header
            if let fault = model.instrumentFault {
                Label(fault, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .lineLimit(2)
            }
            GeometryReader { proxy in
                // The promoted tile is on the primary surface; neither
                // rack branch may mount a second copy of it.
                if let focused = profile.tiles.first(where: {
                    $0.id == focusedTileId && $0.id != primaryTileId
                }) {
                    VStack {
                        Spacer(minLength: 0)
                        tileView(focused, width: proxy.size.width)
                        Spacer(minLength: 0)
                    }
                } else {
                    // The tiles fit the MEASURED area: the width column
                    // estimate can drift without ever clipping a panel
                    // or leaving a dead band under the bar.
                    // The promoted tile lives on the primary surface; the
                    // column must not mount a second copy — one video
                    // source owns exactly one layer slot.
                    let shown = profile.tiles.filter { $0.id != primaryTileId }
                    let aspects = Self.aspectSum(of: shown, model: model)
                    let gaps = CGFloat(max(shown.count - 1, 0)) * 8
                    let fitted = aspects > 0
                        ? min(proxy.size.width, (proxy.size.height - gaps) / aspects)
                        : proxy.size.width
                    ScrollView {
                        VStack(spacing: 8) {
                            ForEach(Array(shown.enumerated()), id: \.offset) { _, tile in
                                tileView(tile, width: fitted)
                            }
                        }
                        .frame(maxWidth: .infinity)
                    }
                }
            }
            controlBar
        }
        .padding(10)
        .background(.black)
        .onAppear { model.prepareInstruments() }
        .sheet(isPresented: $connectPresented) {
            HostConnectSheet(model: model)
        }
        .alert(
            "Hand over control?",
            isPresented: Binding(
                get: { model.takeoverAsk != nil },
                set: { presented in if !presented { model.declineHandover() } }
            )
        ) {
            Button("Hand over") { model.confirmHandover() }
            Button("Keep control", role: .cancel) { model.declineHandover() }
        } message: {
            Text("Operator \(model.takeoverAsk?.fromPrincipal ?? 0) asks for "
                + (model.takeoverAsk?.scope ?? ""))
        }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Menu {
                // The menu picks what the rack SHOWS. Control schemes
                // (device profiles, flight modes) will be their own
                // chooser; this one never grows a second vocabulary.
                Section("Panel layout") {
                    ForEach(InstrumentProfile.builtIn) { candidate in
                        Button {
                            profileId = candidate.id
                        } label: {
                            if candidate.id == profile.id {
                                Label(candidate.name, systemImage: "checkmark")
                            } else {
                                Text(candidate.name)
                            }
                        }
                    }
                }
            } label: {
                Label(profile.name, systemImage: "rectangle.stack")
                    .font(.callout.weight(.medium))
                    .lineLimit(1)
                    .layoutPriority(1)
            }
            Spacer()
            ConnectionChip(phase: model.phase) { connectPresented = true }
        }
        .foregroundStyle(.white)
    }

    @ViewBuilder
    private func tileView(_ tile: InstrumentTile, width: CGFloat) -> some View {
        switch tile {
        case .video(let source):
            let shown = videoSourceOverride.isEmpty ? source : videoSourceOverride
            let liveSource = selectedVideoId(for: shown)
            // The native link does not carry media streams yet; the slot
            // states that rather than implying a camera exists. The source
            // switcher and the enlarge control are the tile's own, so a
            // live feed changes what fills it, not how it is worked.
            ZStack(alignment: .topTrailing) {
                if let id = liveSource {
                    // The named source's own feed, never a stand-in. The
                    // frames flow hub-to-layer; this view never rebuilds
                    // for a picture.
                    VideoSurfaceView(hub: model.videoHub, source: id)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .overlay(alignment: .bottomLeading) {
                            Text("source \(id) · \(shown)")
                                .font(.caption2)
                                .padding(4)
                                .background(.black.opacity(0.5))
                        }
                        .overlay {
                            // A frozen last frame reads as a broken
                            // feed; a paused badge reads as what it
                            // is — the producer rendering another
                            // view.
                            if model.staleSources.contains(id) {
                                ZStack {
                                    Color.black.opacity(0.45)
                                    Label("paused — the producer is on another view",
                                          systemImage: "pause.circle")
                                        .font(.caption)
                                        .foregroundStyle(.white)
                                }
                            }
                        }
                } else {
                    UnavailableTile(
                        title: "Video · \(shown)",
                        reason: "no frames from this session yet"
                    )
                }
                HStack(spacing: 0) {
                    if model.gimbalCaptured {
                        // The quasimode holds the stick for THIS camera;
                        // the badge lives where the picture is.
                        Image(systemName: "camera.rotate.fill")
                            .foregroundStyle(.cyan)
                            .padding(6)
                    }
                    // The switcher names the CURRENT source and each
                    // candidate states whether it has frames: picking a
                    // dead source is allowed (never silently redirected)
                    // but must read as "that source is dark", not as a
                    // switch that did not respond.
                    Menu {
                        ForEach(Self.videoSources, id: \.self) { candidate in
                            Button {
                                // The pick steers the producer (the
                                // simulator renders one camera at a
                                // time); the DISPLAY switches when the
                                // source's first frame confirms it.
                                model.selectVideoSource(named: candidate)
                            } label: {
                                if candidate == shown {
                                    Label(sourceMenuTitle(candidate), systemImage: "checkmark")
                                } else {
                                    Text(sourceMenuTitle(candidate))
                                }
                            }
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "video.badge.ellipsis")
                            Text(model.pendingVideoSource.map { "\($0)…" } ?? shown)
                                .font(.caption.weight(.semibold))
                                .lineLimit(1)
                        }
                        .foregroundStyle(model.pendingVideoSource == nil ? .white : .orange)
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(Rectangle())
                    }
                    Button {
                        withAnimation { focusedTileId = focusedTileId == tile.id ? nil : tile.id }
                    } label: {
                        Image(systemName: focusedTileId == tile.id
                            ? "arrow.down.right.and.arrow.up.left"
                            : "arrow.up.left.and.arrow.down.right")
                            .frame(minWidth: 44, minHeight: 44)
                            .contentShape(Rectangle())
                    }
                    Button {
                        withAnimation {
                            primaryTileId = primaryTileId == tile.id ? "" : tile.id
                            // A tile cannot be focused here and promoted
                            // there at once.
                            if focusedTileId == tile.id { focusedTileId = nil }
                        }
                    } label: {
                        Image(systemName: "rectangle.2.swap")
                            .frame(minWidth: 44, minHeight: 44)
                            .contentShape(Rectangle())
                    }
                    .help("Swap with the primary surface")
                }
                .font(.callout)
                .foregroundStyle(.white)
            }
            .frame(width: width, height: width * 9 / 16)
        case .panel(let id):
            if let choice = model.panelChoice(forTileId: id),
               let display = model.display(for: choice.index) {
                InstrumentPanel(display: display)
                    .frame(
                        width: width,
                        height: width * CGFloat(choice.descriptor.designHeight)
                            / CGFloat(max(choice.descriptor.designWidth, 1))
                    )
            } else {
                UnavailableTile(
                    title: id.uppercased(),
                    reason: model.instrumentFault == nil
                        ? "no such panel in the linked registry"
                        : "instrument runtime unavailable"
                )
                .frame(width: width, height: width * 3 / 4)
            }
        }
    }

    /// The one binding table between source names and wire ids, the same
    /// values the browser's video routing pins. A source must never be
    /// silently redirected to another camera, so an absent source shows
    /// its absence rather than whichever feed arrived first.
    static let videoSourceIds: [String: UInt8] = [
        "fpv": 0,
        "chase": 1,
        "gimbal": 2,
    ]

    private func selectedVideoId(for name: String) -> UInt8? {
        guard let id = Self.videoSourceIds[name] else { return nil }
        return model.liveVideoSources.contains(id) ? id : nil
    }

    /// One menu row's title: the source name plus whether the session
    /// is delivering frames for it right now.
    private func sourceMenuTitle(_ name: String) -> String {
        guard let id = selectedVideoId(for: name) else { return "\(name) · no frames" }
        return model.staleSources.contains(id) ? "\(name) · paused" : "\(name) · live"
    }

    /// One row and one caption: the bar must never eat into the
    /// instruments it serves. The telegraph is the row's centerpiece;
    /// everything narrational lives in the caption or moved out — pad
    /// hints to the connect sheet, the capture badge onto the video
    /// tile itself.
    private var controlBar: some View {
        VStack(spacing: 4) {
            // Row one, flight authority, every slot permanent: the
            // lease chip and the arm telegraph are ALWAYS present —
            // the telegraph reads the flight controller's state for
            // an observer and becomes the control under a lease, so
            // gaining or losing authority never adds, removes, or
            // moves a single element.
            HStack(spacing: 8) {
                Image(systemName: model.controllerAttached
                    ? "gamecontroller.fill"
                    : "gamecontroller")
                    .foregroundStyle(model.controllerAttached ? .green : .secondary)
                leaseChip
                ArmTelegraphControl(model: model, interactive: model.leaseHeld)
                Spacer(minLength: 0)
            }
            .font(.callout)
            // Row two, session tools: what the catalog offers is
            // fixed for the session, so this row's shape never
            // changes mid-flight either.
            HStack(spacing: 8) {
                if model.catalog?.offersFlightControl == true {
                    // The autopilot face exists only for a host that
                    // commands a flight computer; a plan-input panel
                    // never grows one.
                    barChip("FCU", width: Self.fcuButtonWidth,
                            tint: fcuShown ? .cyan : Color(white: 0.7),
                            filled: fcuShown) {
                        withAnimation { fcuShown.toggle() }
                    }
                }
                if model.catalog?.offersSimReset == true {
                    // Only a simulator host advertises the lifecycle
                    // reset; a real vehicle's bar never grows a
                    // button that could not mean anything there.
                    barChip("Reset", width: Self.resetButtonWidth, tint: .red, filled: false) {
                        resetAsked = true
                    }
                }
                Spacer(minLength: 0)
            }
            .font(.callout)
            // The status line owns exactly one caption row at a fixed
            // height whether or not it has anything to say: a line
            // that appears by pushing the instruments around teaches
            // the operator to distrust the layout. It always shows
            // the newest session line and opens the full log.
            Button {
                logShown = true
            } label: {
                Text(barCaption?.text ?? model.status)
                    .font(.caption2.monospaced())
                    .foregroundStyle(barCaption?.warning == true ? .orange : .secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, minHeight: 16, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .foregroundStyle(.white)
        .confirmationDialog(
            "Reset the simulation?",
            isPresented: $resetAsked,
            titleVisibility: .visible
        ) {
            Button("Reset the simulation", role: .destructive) { model.resetSim() }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The vehicle returns to its parking spot and the flight controller restarts.")
        }
        .sheet(isPresented: $logShown) {
            StatusLogSheet(model: model)
        }
    }

    /// One fixed-width bar chip: caption-bold title, rounded fill, a
    /// full-height hit area regardless of its visual size.
    private func barChip(
        _ title: String,
        width: CGFloat,
        tint: Color,
        filled: Bool,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Text(title)
                .font(.caption.weight(.bold))
                .lineLimit(1)
                .frame(width: width)
                .padding(.vertical, 4)
                .background(
                    RoundedRectangle(cornerRadius: 5)
                        .fill(filled ? tint.opacity(0.3) : Color(white: 0.2))
                )
                .frame(minHeight: 40)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(tint)
    }


    /// The lease control's three faces, one fixed-width chip: blue to
    /// ask, amber while the host owes an answer, red to stand down.
    @ViewBuilder
    private var leaseChip: some View {
        let (title, tint): (String, Color) = model.leaseHeld
            ? ("Release", .red)
            : model.leaseAsked ? ("Asked…", .orange) : ("Request", .cyan)
        Button {
            if model.leaseHeld {
                model.releaseLease()
            } else if !model.leaseAsked {
                model.requestLease()
            }
        } label: {
            Text(title)
                .font(.caption.weight(.bold))
                .lineLimit(1)
                .frame(width: Self.leaseButtonWidth)
                .padding(.vertical, 4)
                .background(RoundedRectangle(cornerRadius: 5).fill(tint.opacity(0.3)))
        }
        .buttonStyle(.plain)
        .foregroundStyle(tint)
        .disabled(model.catalog == nil || model.leaseAsked)
    }

    /// The one caption line under the bar, present only when the
    /// telegraph has a grievance. The link figures are diagnostics and
    /// live on the connection sheet, not under the operator's thumbs.
    private var barCaption: (text: String, warning: Bool)? {
        if model.armPhase == 2 {
            return ("arm refused: \(model.armDetail)", true)
        }
        if model.armPhase == 3 {
            return ("vehicle disarmed itself — lever is back on SAFE", true)
        }
        return nil
    }
}

/// The arm order telegraph: a two-position lever the operator sets and
/// a lamp only the flight controller's own report moves. Amber between
/// order and answer; the lever never re-sends on its own.
private struct ArmTelegraphControl: View {
    @ObservedObject var model: HostLinkModel
    /// Whether the operator holds the authority to move the levers.
    /// Without it the telegraph is a gauge: it shows the flight
    /// controller's own armed state, at reduced emphasis, in the same
    /// place it will be a control once authority is granted.
    let interactive: Bool

    var body: some View {
        // The levers carry the whole story: pressing one is the
        // operator's intent, amber is an order the flight controller
        // has not answered, green is the controller confirming that
        // state. No separate lamp — the answer lives where the order
        // was given.
        HStack(spacing: 0) {
            lever("SAFE", ordersArmed: false)
            lever("ARM", ordersArmed: true)
        }
        .background(Capsule().fill(Color(white: 0.18)))
        .opacity(interactive ? 1.0 : 0.55)
    }

    /// One lever width for both positions: SAFE and ARM must read as
    /// the two ends of one control, and the capsule must not change
    /// shape when the weight of the selected title differs.
    private static let leverWidth: CGFloat = 56

    private func lever(_ title: String, ordersArmed: Bool) -> some View {
        // Under authority the highlight tracks the operator's ORDER;
        // as a gauge it tracks the flight controller's ANSWER — an
        // observer sees the vehicle's state, never a stale order.
        let selected = interactive
            ? model.armOrdered == ordersArmed
            : model.armConfirmed == (ordersArmed ? 2 : 1)
        return Button {
            if ordersArmed { model.arm() } else { model.disarm() }
        } label: {
            Text(title)
                .font(.callout.weight(selected ? .bold : .regular))
                .lineLimit(1)
                .frame(width: Self.leverWidth)
                .padding(.vertical, 5)
                .background(
                    Capsule().fill(selected ? leverTint(ordersArmed: ordersArmed) : .clear)
                )
        }
        .buttonStyle(.plain)
        .disabled(!interactive)
    }

    private func leverTint(ordersArmed: Bool) -> Color {
        // Amber is an unanswered order; green is the flight
        // controller's own report confirming the ordered state; gray
        // is a selection the controller has not (or no longer)
        // confirmed. Red stays reserved for what has actually gone
        // wrong.
        if interactive && model.armPhase == 1 { return .orange }
        let confirmed = model.armConfirmed == (ordersArmed ? 2 : 1)
        return confirmed ? .green : Color(white: 0.35)
    }

}

/// A tile slot this build cannot fill, saying why in place. It never
/// paints a picture that implies the data exists.
private struct UnavailableTile: View {
    let title: String
    let reason: String

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 8)
                .fill(Color(white: 0.12))
            VStack(spacing: 6) {
                Text(title)
                    .font(.headline)
                Text(reason)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .foregroundStyle(.white)
            .padding(8)
        }
    }
}

/// One glanceable statement of where the link stands. Tapping opens the
/// connection sheet — the chip is the door to the flow, not the flow.
struct ConnectionChip: View {
    let phase: HostLinkModel.Phase
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(spacing: 6) {
                Circle().fill(tint).frame(width: 8, height: 8)
                Text(label)
                    .font(.footnote.weight(.medium))
                    .lineLimit(1)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Capsule().fill(Color(white: 0.18)))
        }
        .buttonStyle(.plain)
        .foregroundStyle(.white)
    }

    private var label: String {
        // One word each: the chip is a glance, and the sheet carries the
        // host and scope in full.
        switch phase {
        case .idle: "Connect"
        case .connecting: "Connecting…"
        case .observing: "Observing"
        case .controlling: "Controlling"
        case .reconnecting: "Reconnecting…"
        case .stopped: "Stopped"
        }
    }

    private var tint: Color {
        switch phase {
        case .idle: .gray
        case .connecting, .reconnecting: .yellow
        case .observing: .green
        case .controlling: .blue
        case .stopped: .red
        }
    }
}

/// The session's own log in a standard sheet: one selectable text
/// block (a single selection copies any number of lines), timestamps
/// leading, newest last — the status line above shows the tail, this
/// shows the whole tape.
private struct StatusLogSheet: View {
    @ObservedObject var model: HostLinkModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                Text(joined)
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
            .navigationTitle("Session log")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private var joined: String {
        model.statusLog
            .map { "\(Self.clock.string(from: $0.at))  \($0.text)" }
            .joined(separator: "\n")
    }

    private static let clock: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "HH:mm:ss"
        return formatter
    }()
}
