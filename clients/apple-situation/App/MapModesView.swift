import PilotageSituationCore
import SwiftUI

/// One way of drawing the ground.
///
/// A mode is a base map, not a layer: exactly one is drawn, and the layers below the tiles
/// are what sits on top of it. Terrain is the only one this build carries. Satellite, VFR
/// and IFR are the same shape of thing and arrive as data rather than as new screens.
struct MapMode: Identifiable, Equatable {
    let id: String
    let title: String
    let symbol: String

    /// The base maps this build can draw.
    ///
    /// Built from what exists rather than from what is planned. A tile for a mode the
    /// client cannot draw is a promise the map does not keep.
    static let available: [MapMode] = [
        MapMode(id: "terrain", title: "Terrain", symbol: "mountain.2.fill"),
    ]
}

/// Choose how the ground is drawn and what sits on it.
///
/// The panel Apple's map uses for the same job: the base map as tiles across the top, the
/// layers as switches under them, and the credit for the data at the foot of it, where a
/// reader who wants it can find it and a reader who does not is not made to read it.
struct MapModesView: View {
    let modes: [MapMode]
    @Binding var selectedModeID: String
    let layers: [DisplayLayerControl]
    let setLayerEnabled: (String, Bool) -> Void
    let attributions: [String]
    let close: () -> Void
    @State private var attributionPresented = false

    var body: some View {
        VStack(spacing: 20) {
            header
            modeTiles
            if !layers.isEmpty {
                layerSwitches
            }
            attributionFooter
        }
        .padding(Metrics.panelPadding)
        .glassEffect(
            .regular,
            in: .rect(cornerRadius: Metrics.panelCorner)
        )
        // The panel is as big as what it holds. A fixed width leaves a column of empty
        // glass beside one tile, and a detent invites a drag the panel does not answer.
        .frame(width: Metrics.panelWidth)
        .fixedSize(horizontal: false, vertical: true)
        .sheet(isPresented: $attributionPresented) {
            MapAttributionView(notices: attributions)
        }
    }

    private var header: some View {
        ZStack {
            Text("Map Modes")
                .font(.title3.weight(.semibold))
            HStack {
                Spacer()
                Button(action: close) {
                    Image(systemName: "xmark")
                        .font(.system(size: 16, weight: .semibold))
                        // The same target as every other control. A close button smaller
                        // than the rest is the one a reader misses.
                        .frame(width: Metrics.control, height: Metrics.control)
                }
                .buttonStyle(.glass)
                .accessibilityLabel("Close map modes")
            }
        }
    }

    private var modeTiles: some View {
        HStack(alignment: .top, spacing: 16) {
            ForEach(modes) { mode in
                Button {
                    selectedModeID = mode.id
                } label: {
                    VStack(spacing: 8) {
                        Image(systemName: mode.symbol)
                            .font(.system(size: 26))
                            .frame(width: 74, height: 74)
                            .glassEffect(.regular, in: .rect(cornerRadius: 14))
                            .overlay {
                                RoundedRectangle(cornerRadius: 14)
                                    .stroke(
                                        selectedModeID == mode.id ? Color.accentColor : .clear,
                                        lineWidth: 3
                                    )
                            }
                        Text(mode.title)
                            .font(.footnote)
                    }
                }
                .buttonStyle(.plain)
                .accessibilityLabel(mode.title)
                .accessibilityAddTraits(selectedModeID == mode.id ? [.isSelected] : [])
            }
            Spacer(minLength: 0)
        }
    }

    private var layerSwitches: some View {
        VStack(spacing: 0) {
            ForEach(Array(layers.enumerated()), id: \.element.id) { index, layer in
                Toggle(
                    isOn: Binding(
                        get: { layer.enabled },
                        set: { setLayerEnabled(layer.id, $0) }
                    )
                ) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(layer.title)
                        Text(layer.sourceStateLabel)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .padding(.vertical, 10)
                .padding(.horizontal, 14)
                if index < layers.count - 1 {
                    Divider().padding(.leading, 14)
                }
            }
        }
        .glassEffect(.regular, in: .rect(cornerRadius: 16))
    }

    /// The credit for the data, at the foot of the panel that chose it.
    private var attributionFooter: some View {
        Button {
            attributionPresented = true
        } label: {
            Text(summary)
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Map data sources, \(summary)")
        .accessibilityHint("Shows the full notice for every source")
    }

    /// Name the providers without spelling out every source.
    private var summary: String {
        let providers = attributions.compactMap { notice -> String? in
            if notice.contains("Natural Earth") { return "Natural Earth" }
            if notice.contains("AWS Terrain Tiles") { return "AWS Terrain Tiles" }
            return nil
        }
        guard let first = providers.first else { return "Map data" }
        return providers.count > 1
            ? "© \(first) and other data providers"
            : "© \(first)"
    }
}
