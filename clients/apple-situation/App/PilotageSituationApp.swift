import PilotageMapLibreBinding
import PilotageSituationCore
import SwiftUI

@main
struct PilotageSituationApp: App {
    var body: some Scene {
        WindowGroup {
            SituationContentView()
        }
    }
}

private struct SituationContentView: View {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var model = SituationClientModel()

    var body: some View {
        ZStack(alignment: .top) {
            SituationMap(batch: model.display)
            VStack(spacing: 8) {
                RadioStatusView(source: model.radioSource)
                if let message = model.errorMessage {
                    Text(message)
                        .padding(12)
                        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
                }
            }
            .padding()
        }
        .task(id: scenePhase) {
            if scenePhase == .active {
                await model.activate()
            } else {
                await model.suspend()
            }
        }
    }
}

private struct SituationMap: UIViewRepresentable {
    let batch: DisplayBatch?

    func makeUIView(context: Context) -> SituationMapView {
        let styleJSON = (try? SituationStyleResource.load())
            ?? SituationStyleResource.fallbackJSON
        return SituationMapView(styleJSON: styleJSON)
    }

    func updateUIView(_ mapView: SituationMapView, context: Context) {
        if let batch {
            mapView.apply(batch)
        }
    }
}
