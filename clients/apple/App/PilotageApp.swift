import CoreLocation
import PilotageMapLibreBinding
import PilotageCore
import SwiftUI

@main
struct PilotageApp: App {
    init() {
        #if DEBUG
        FootprintProbe.start()
        #endif
    }

    var body: some Scene {
        WindowGroup {
            SituationContentView()
        }
    }
}

/// What a launch asked the application to do before a hand touched it.
///
/// Only in a debug build. A screen that can be opened from outside is a way in, and the
/// shipped application has no reason to offer one. It exists so a window at an awkward
/// size can be photographed and measured without somebody holding the tablet.
enum LaunchRequest {
    static var openMapModes: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-OpenMapModes")
        #else
        false
        #endif
    }

    /// Open the Instruments destination and connect with the persisted
    /// facts, so a headless harness can photograph a live panel.
    static var openInstruments: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-OpenInstruments")
        #else
        false
        #endif
    }

    /// Ask for control as soon as the session admits (harness only).
    static var autoControl: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-AutoControl")
        #else
        false
        #endif
    }

    /// Arm one second after control is held (harness only).
    static var autoArm: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-AutoArm")
        #else
        false
        #endif
    }

    /// Decode video but discard the image unpublished (harness bisect
    /// only).
    static var decodeNoPublish: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-DecodeNoPublish")
        #else
        false
        #endif
    }

    /// Drop every video frame before decode (harness bisect only).
    static var noVideoDecode: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-NoVideoDecode")
        #else
        false
        #endif
    }

    /// Climb through a takeoff window once the arm is accepted
    /// (harness only).
    static var autoClimb: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-AutoClimb")
        #else
        false
        #endif
    }
}

/// What the controls can ask of the map.
///
/// The controls live in the view hierarchy above the map and must not hold the map itself,
/// so they hold this instead.
@MainActor
struct SituationMapCommands {
    let resetHeading: () -> Void
    let resetPitch: () -> Void
    let centre: (CLLocationCoordinate2D, Bool) -> Void
    /// Centre and set how much ground is on screen, for a reader who asked to be found.
    let centreAndFrame: (CLLocationCoordinate2D, Bool) -> Void
    let setHeading: (Double, Bool) -> Void
}

struct SituationMap: UIViewRepresentable {
    let batch: DisplayBatch?
    let onFeatureTapped: (String) -> Void
    let onCameraChanged: (SituationCamera) -> Void
    let onReady: (SituationMapCommands) -> Void
    let onAttributions: ([String]) -> Void
    let onMovedByReader: () -> Void

    func makeUIView(context: Context) -> SituationMapView {
        let styleJSON = (try? SituationStyleResource.load())
            ?? SituationStyleResource.fallbackJSON
        let view = SituationMapView(styleJSON: styleJSON)
        view.initialPitchDegrees = 55
        // Open over the ground the terrain archive covers rather than on the Atlantic, and
        // let the map be pinched out until the whole world is on screen.
        view.initialCenter = CLLocationCoordinate2D(latitude: 40.5, longitude: -76.5)
        view.initialZoomLevel = 6
        view.minimumZoomLevel = 0
        view.maximumZoomLevel = SituationStyleResource.maximumZoomLevel
        view.baseLayerIdentifiers = ["terrain-base": "pilotage-terrain-hillshade"]
        view.onFeatureTapped = onFeatureTapped
        view.onCameraChanged = onCameraChanged
        view.onStyleLoaded = { onAttributions($0.sourceAttributions) }
        view.onMovedByReader = onMovedByReader
        // The handle is published after the view exists, and the publish is deferred so it
        // does not change application state while the view tree is being built.
        DispatchQueue.main.async { [weak view] in
            guard let view else { return }
            onReady(
                SituationMapCommands(
                    resetHeading: { view.resetHeading() },
                    resetPitch: { view.resetPitch() },
                    centre: { view.centre(on: $0, animated: $1) },
                    centreAndFrame: {
                        view.centre(
                            on: $0,
                            widthNauticalMiles: SituationMapView.ownshipWidthNauticalMiles,
                            animated: $1
                        )
                    },
                    setHeading: { view.setHeading($0, animated: $1) }
                )
            )
        }
        return view
    }

    func updateUIView(_ mapView: SituationMapView, context: Context) {
        mapView.onFeatureTapped = onFeatureTapped
        mapView.onCameraChanged = onCameraChanged
        mapView.onMovedByReader = onMovedByReader
        if let batch {
            mapView.apply(batch)
        }
    }
}
