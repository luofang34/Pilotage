import SwiftUI

struct AviationChartNotice: View {
    let release: AviationRelease
    let openData: () -> Void

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            Button(action: openData) {
                VStack(alignment: .leading, spacing: 3) {
                    Text("\(release.product.title) · \(release.edition) · \(release.validityLabel(at: context.date))")
                        .font(.caption.weight(.semibold))
                    if release.channel == "development" { Text("Development sample").font(.caption2) }
                    if !release.coverage.complete { Text("Partial coverage").font(.caption2) }
                }
                .padding(10)
                .glassEffect(.regular, in: .rect(cornerRadius: 12))
            }
            .buttonStyle(.plain)
            .accessibilityHint("Shows chart validity, coverage, and source details")
        }
    }
}
