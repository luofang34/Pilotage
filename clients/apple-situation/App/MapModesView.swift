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
    /// Whether the panel carries its own width, or takes the width it is given.
    ///
    /// Beside the map it is a card of a set width. Brought up from the bottom edge it is
    /// as wide as the screen, because a sheet that stops short of the sides reads as a
    /// card that failed to load rather than as a deliberate width.
    var fixedWidth: Bool = true
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
        .frame(width: fixedWidth ? Metrics.panelWidth : nil)
        .frame(maxWidth: fixedWidth ? nil : .infinity)
        .fixedSize(horizontal: false, vertical: true)
        .sheet(isPresented: $attributionPresented) {
            MapAttributionView(notices: attributions)
        }
    }

    private var header: some View {
        ZStack {
            Text("Map Modes")
                .font(.title3.weight(.bold))
            HStack {
                Spacer()
                // The disc and the cross are set apart, because the button they copy has
                // a wider disc and a smaller cross than any one number gives.
                //
                // The role's own label answers to neither the image scale nor the control
                // size, so naming the symbol is what makes it move at all. The font then
                // sets the cross. The disc follows the label it is given, so a frame wider
                // than the label sets the disc and leaves the cross alone: the disc comes
                // out ten points above the frame. A frame narrower than the label does
                // nothing at all, because the control size holds a floor under the disc,
                // and that is what earlier frames were being swallowed by.
                Button(role: .close, action: close) {
                    Image(systemName: "xmark")
                        .font(.system(size: 23, weight: .regular))
                        .frame(width: 35, height: 35)
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .controlSize(.small)
                .foregroundStyle(.secondary)
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
                // Small and dim on purpose. The credit is owed and has to be there; a
                // reader looking at the map is not the one it is owed to.
                .font(.caption2)
                .foregroundStyle(.tertiary)
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


