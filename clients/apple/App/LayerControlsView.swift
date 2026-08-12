import PilotageCore
import SwiftUI

struct LayerControlsView: View {
    let layers: [DisplayLayerControl]
    let setEnabled: (String, Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Layers")
                .font(.headline)
            ForEach(layers, id: \.id) { layer in
                Toggle(
                    isOn: Binding(
                        get: { layer.enabled },
                        set: { setEnabled(layer.id, $0) }
                    )
                ) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(layer.title)
                            .font(.subheadline.weight(.semibold))
                        Text("\(layer.sourceStateLabel): \(layer.sourceDetail)")
                            .font(.caption)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .frame(maxWidth: 330)
        .padding(12)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
    }
}
