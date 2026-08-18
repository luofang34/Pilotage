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
        }
    }

    @ViewBuilder
    private var content: some View {
        switch tile {
        case .video(let source):
            if let id = InstrumentRackView.videoSourceIds[source],
               model.liveVideoSources.contains(id) {
                VideoSurfaceView(hub: model.videoHub, source: id)
                    .ignoresSafeArea()
            } else {
                Text("Video · \(source) — no frames from this session yet")
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
