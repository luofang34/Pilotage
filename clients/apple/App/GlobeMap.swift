import CoreLocation
import PilotageMapLibreBinding
import PilotageCore
import SwiftUI

struct GlobeMap: UIViewRepresentable {
    let style: AviationMapStyle
    let mode: String
    let requestedAt: Double?
    let batch: DisplayBatch?
    var plannedRoutes: [PlannedMapRoute] = []
    let onFeatureTapped: (String) -> Void
    let onCameraChanged: (SituationCamera) -> Void
    let onReady: (SituationMapCommands) -> Void
    let onMovedByReader: () -> Void

    func makeUIView(context: Context) -> GlobeMapView {
        let view = GlobeMapView()
        if let distance = LaunchRequest.globeDistance { view.changeCamera { $0.distance = distance } }
        if let pitch = LaunchRequest.globePitch { view.changeCamera { $0.pitch = pitch } }
        view.configure(style: style, mode: mode, requestedAt: requestedAt)
        view.batch = batch
        view.plannedRoutes = plannedRoutes
        view.onFeatureTapped = onFeatureTapped
        view.onCameraChanged = onCameraChanged
        view.onMovedByReader = onMovedByReader
        DispatchQueue.main.async { [weak view] in
            guard let view else { return }
            view.changeCamera { _ in }
            onReady(SituationMapCommands(
                resetHeading: { view.changeCamera(animated: true) { $0.heading = 0 } },
                setPitch: { pitch, animated in view.changeCamera(animated: animated) { $0.pitch = pitch } },
                centre: { coordinate, animated in view.centre(on: coordinate, frame: false, animated: animated) },
                centreAndFrame: { coordinate, animated in view.centre(on: coordinate, frame: true, animated: animated) },
                setHeading: { heading, animated in view.changeCamera(animated: animated) { $0.heading = heading } },
                fitRoute: { coordinates, animated in view.fitRoute(coordinates, animated: animated) }
            ))
        }
        return view
    }

    func updateUIView(_ view: GlobeMapView, context: Context) {
        view.plannedRoutes = plannedRoutes
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
