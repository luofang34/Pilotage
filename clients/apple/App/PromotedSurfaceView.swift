import IndicateAppleDisplay
import PilotageCore
import SwiftUI

/// One tile granted the primary surface: the video feed or the panel
/// the operator swapped in place of the map, rendered wall to wall.
/// The way back is one floating control, placed by the same rule as
/// every other map control.
struct PromotedSurfaceView: View {
    @ObservedObject var model: HostLinkModel
    let tile: InstrumentTile
    /// Returns the primary surface to the map.
    let restoreMap: () -> Void
    /// The operator's source choice, shared with the rack tile: the
    /// promoted surface shows the SAME selection, and switching here
    /// works the same as switching there — a full screen that locks
    /// its camera would send the operator back to the rack to change
    /// views.
    @AppStorage("pilotageVideoSource") private var videoSourceOverride = ""

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            content
            Button(action: restoreMap) {
                Label("Map", systemImage: "map")
                    .font(.callout.weight(.medium))
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(.ultraThinMaterial, in: Capsule())
            }
            .buttonStyle(.plain)
            .mapControlPlacement(.bottomTrailing)
            if case .video(let source) = tile {
                sourceSwitcher(profileSource: source)
                    .mapControlPlacement(.topTrailing)
            }
        }
    }

    /// The same switcher the rack tile offers: current source on its
    /// face, each candidate stating whether it has frames, and a pick
    /// steering the producer.
    private func sourceSwitcher(profileSource: String) -> some View {
        let shown = videoSourceOverride.isEmpty ? profileSource : videoSourceOverride
        return Menu {
            ForEach(InstrumentRackView.videoSources, id: \.self) { candidate in
                Button {
                    videoSourceOverride = candidate
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
                Text(shown)
                    .font(.caption.weight(.semibold))
                    .lineLimit(1)
            }
            .frame(minWidth: 44, minHeight: 44)
            .contentShape(Rectangle())
            .padding(.horizontal, 8)
            .background(.ultraThinMaterial, in: Capsule())
        }
        .buttonStyle(.plain)
        .foregroundStyle(.white)
    }

    private func sourceMenuTitle(_ name: String) -> String {
        guard let id = InstrumentRackView.videoSourceIds[name],
              model.liveVideoSources.contains(id)
        else { return "\(name) · no frames" }
        return "\(name) · live"
    }

    @ViewBuilder
    private var content: some View {
        switch tile {
        case .video(let source):
            let shown = videoSourceOverride.isEmpty ? source : videoSourceOverride
            if let id = InstrumentRackView.videoSourceIds[shown],
               model.liveVideoSources.contains(id) {
                VideoSurfaceView(hub: model.videoHub, source: id)
                    .ignoresSafeArea()
            } else {
                Text("Video · \(shown) — no frames from this session yet")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        case .panel(let id):
            if let choice = model.panelChoice(forTileId: id),
               let display = model.display(for: choice.index) {
                GeometryReader { proxy in
                    let aspect = CGFloat(choice.descriptor.designHeight)
                        / CGFloat(max(choice.descriptor.designWidth, 1))
                    let width = min(proxy.size.width, proxy.size.height / aspect)
                    VStack {
                        Spacer(minLength: 0)
                        HStack {
                            Spacer(minLength: 0)
                            InstrumentPanel(display: display)
                                .frame(width: width, height: width * aspect)
                            Spacer(minLength: 0)
                        }
                        Spacer(minLength: 0)
                    }
                }
            } else {
                Text("\(id.uppercased()) — no such panel in the linked registry")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }
}
