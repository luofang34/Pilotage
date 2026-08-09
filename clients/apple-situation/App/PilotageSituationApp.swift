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
    @StateObject private var model = SituationClientModel()

    var body: some View {
        ZStack(alignment: .top) {
            SituationMap(batch: model.display)
            if let message = model.errorMessage {
                Text(message)
                    .padding(12)
                    .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
                    .padding()
            }
        }
    }
}

@MainActor
private final class SituationClientModel: ObservableObject {
    @Published private(set) var display: DisplayBatch?
    @Published private(set) var errorMessage: String?
    private let session = PresentationSession()

    init() {
        do {
            display = try session.currentDisplay()
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

private struct SituationMap: UIViewRepresentable {
    let batch: DisplayBatch?

    func makeUIView(context: Context) -> SituationMapView {
        let styleURL = Bundle.main.url(forResource: "SituationStyle", withExtension: "json")
        return SituationMapView(styleURL: styleURL)
    }

    func updateUIView(_ mapView: SituationMapView, context: Context) {
        if let batch {
            mapView.apply(batch)
        }
    }
}
