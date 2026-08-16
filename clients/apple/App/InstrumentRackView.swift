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
    /// Whether the map shares the screen; the rack owns the toggle
    /// because the rack is what remains either way.
    @Binding var mapVisible: Bool
    @State private var connectPresented = false

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
                ScrollView {
                    VStack(spacing: 8) {
                        ForEach(Array(profile.tiles.enumerated()), id: \.offset) { _, tile in
                            tileView(tile, width: proxy.size.width)
                        }
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
    }

    private var header: some View {
        HStack(spacing: 10) {
            Menu {
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
            } label: {
                Label(profile.name, systemImage: "rectangle.stack")
                    .font(.callout.weight(.medium))
                    .lineLimit(1)
                    .layoutPriority(1)
            }
            Spacer()
            ConnectionChip(phase: model.phase) { connectPresented = true }
            Button {
                withAnimation { mapVisible.toggle() }
            } label: {
                Image(systemName: mapVisible
                    ? "rectangle.leadinghalf.inset.filled"
                    : "map")
            }
            .help(mapVisible ? "Hide the map" : "Show the map")
        }
        .foregroundStyle(.white)
    }

    @ViewBuilder
    private func tileView(_ tile: InstrumentTile, width: CGFloat) -> some View {
        switch tile {
        case .video(let source):
            // The native link does not carry media streams yet; the slot
            // states that rather than implying a camera exists.
            UnavailableTile(
                title: "Video · \(source)",
                reason: "this link does not carry media streams yet"
            )
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

    private var controlBar: some View {
        HStack(spacing: 10) {
            Image(systemName: model.controllerAttached
                ? "gamecontroller.fill"
                : "gamecontroller")
                .foregroundStyle(model.controllerAttached ? .green : .secondary)
            Spacer()
            if model.leaseHeld {
                Button("Release control", role: .destructive) { model.releaseLease() }
            } else {
                Button("Request control") { model.requestLease() }
                    .disabled(model.catalog == nil)
            }
        }
        .font(.callout)
        .foregroundStyle(.white)
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
