import PilotageCore
import SwiftUI

struct PositionlessTrafficView: View {
    let items: [DisplayTrafficListItem]
    let select: (String) -> Void

    var body: some View {
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text("Traffic without position")
                    .font(.headline)
                ForEach(items, id: \.id) { item in
                    Button {
                        select(item.id)
                    } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.title)
                                .font(.subheadline.weight(.semibold))
                            Text(item.summary)
                                .font(.caption)
                        }
                    }
                    .buttonStyle(.plain)
                }
            }
            .frame(maxWidth: 330, alignment: .leading)
            .padding(12)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
        }
    }
}
