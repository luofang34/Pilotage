import CoreLocation
import PilotageMapLibreBinding
import PilotageCore
import SwiftUI

struct GlobeMap: UIViewRepresentable {
    let style: AviationMapStyle
    let mode: String
    let requestedAt: Double?
    let batch: DisplayBatch?
    let onFeatureTapped: (String) -> Void
    let onCameraChanged: (SituationCamera) -> Void
    let onReady: (SituationMapCommands) -> Void
    let onMovedByReader: () -> Void

    func makeUIView(context: Context) -> GlobeMapView {
        let view = GlobeMapView()
        if let distance = LaunchRequest.globeDistance { view.changeCamera { $0.distance = distance } }
        view.configure(style: style, mode: mode, requestedAt: requestedAt)
        view.batch = batch
        view.onFeatureTapped = onFeatureTapped
        view.onCameraChanged = onCameraChanged
        view.onMovedByReader = onMovedByReader
        DispatchQueue.main.async { [weak view] in
            guard let view else { return }
            view.changeCamera { _ in }
            onReady(SituationMapCommands(
                resetHeading: { view.changeCamera { $0.heading = 0 } },
                resetPitch: { view.changeCamera { $0.pitch = 0 } },
                centre: { coordinate, _ in view.centre(on: coordinate, frame: false) },
                centreAndFrame: { coordinate, _ in view.centre(on: coordinate, frame: true) },
                setHeading: { heading, _ in view.changeCamera { $0.heading = heading } }
            ))
        }
        return view
    }

    func updateUIView(_ view: GlobeMapView, context: Context) {
        view.configure(style: style, mode: mode, requestedAt: requestedAt)
        view.batch = batch
        view.onFeatureTapped = onFeatureTapped
        view.onCameraChanged = onCameraChanged
        view.onMovedByReader = onMovedByReader
    }

    static func dismantleUIView(_ view: GlobeMapView, coordinator: ()) {
        view.stop()
    }
}
