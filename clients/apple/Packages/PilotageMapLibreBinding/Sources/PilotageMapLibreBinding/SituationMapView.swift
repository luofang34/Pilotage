@preconcurrency import MapLibre
import PilotageCore
import UIKit

/// An iPadOS map that applies portable display batches.
@MainActor
public final class SituationMapView: UIView, @preconcurrency MLNMapViewDelegate {
    /// Receives an error that occurs after the map style loads.
    public var onOverlayError: ((Error) -> Void)?
    /// Receives the stable identity of a tapped display feature.
    public var onFeatureTapped: ((String) -> Void)?
    /// Receives the camera angles whenever the reader moves the map.
    ///
    /// Heading and tilt are the two things a reader has to be able to undo. The controls
    /// that undo them live in the application, so the application has to know the angles.
    public var onCameraChanged: ((SituationCamera) -> Void)?
    /// Receives this view once its style is loaded and its sources can be read.
    public var onStyleLoaded: ((SituationMapView) -> Void)?
    /// Reports that the reader moved the map with their hands.
    ///
    /// A map that is following the aircraft has to stop the moment a reader drags it, or
    /// the map fights them and then snaps back. Only a gesture counts: a move the client
    /// made itself is the following working.
    public var onMovedByReader: (() -> Void)?
    /// Maps portable base-layer identities to base-style layer identities.
    public var baseLayerIdentifiers: [String: String] = [:]

    private let mapView: MLNMapView
    private let overlay = SituationOverlay()
    private let ownshipMark = SituationOwnshipMark()
    private var pendingBatch: DisplayBatch?
    private var hasAppliedInitialCamera = false

    /// Viewing angle away from straight down, in degrees, taken once the style loads.
    ///
    /// The renderer drapes nothing over the elevation model, so shading and colour carry
    /// height on their own and only from a camera that looks across the ground. A pilot
    /// can still tilt the map by hand afterwards.
    public var initialPitchDegrees: CGFloat = 0

    /// Where the map opens, and how far out it may be pinched.
    ///
    /// Without a stated camera the map opens on the Atlantic at world zoom, which is a
    /// place nobody asked for. Without a stated floor the renderer keeps whatever minimum
    /// it happens to default to, and a pilot who pinches out to see the whole world finds
    /// the map stops before it gets there.
    public var initialCenter: CLLocationCoordinate2D?
    /// Zoom the map opens at.
    public var initialZoomLevel: Double = 6
    /// Furthest the map may be pinched out. Zero shows the whole world.
    public var minimumZoomLevel: Double = 0
    /// Closest the map may be pinched in.
    ///
    /// A raster-dem source keeps drawing past its highest zoom by stretching the tile it
    /// has. One step of that is a softer picture; several are a wash of colour that states
    /// detail the elevation model never carried. The floor belongs just above the archive,
    /// so the map stops where its data stops.
    public var maximumZoomLevel: Double = 14

    /// Create a map with the specified base style JSON.
    public init(frame: CGRect = .zero, styleJSON: String) {
        mapView = MLNMapView(frame: .zero, styleJSON: styleJSON)
        super.init(frame: frame)
        mapView.translatesAutoresizingMaskIntoConstraints = false
        mapView.delegate = self
        configureOrnaments()
        addSubview(mapView)
        addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(handleTap)))
        for recogniser in mapView.gestureRecognizers ?? [] {
            recogniser.addTarget(self, action: #selector(handleMapGesture))
        }
        NSLayoutConstraint.activate([
            mapView.leadingAnchor.constraint(equalTo: leadingAnchor),
            mapView.trailingAnchor.constraint(equalTo: trailingAnchor),
            mapView.topAnchor.constraint(equalTo: topAnchor),
            mapView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    @objc private func handleMapGesture(_ recogniser: UIGestureRecognizer) {
        guard recogniser.state == .began || recogniser.state == .changed else { return }
        onMovedByReader?()
    }

    /// Decide which of the renderer's own controls stay on the map.
    ///
    /// The renderer's licence does not ask for its wordmark, and its own header says so, so
    /// the map does not carry it. Its compass goes because the client draws heading itself:
    /// track-up against north-up is a mode a pilot flies by, not an ornament.
    ///
    /// Its attribution control goes as well, and the notices do not. The sources each state
    /// one and a licence is not satisfied by a control nobody opens; the client shows them
    /// where a reader is already choosing what the map draws.
    private func configureOrnaments() {
        mapView.showsLogoView = false
        mapView.showsCompassView = false
        mapView.showsAttributionButton = false
    }

    /// The notice each source of this style asks to be shown.
    ///
    /// Read from the style rather than written twice, so a source added without its notice
    /// cannot appear on the map.
    public var sourceAttributions: [String] {
        guard let sources = mapView.style?.sources else { return [] }
        return sources
            .compactMap { ($0 as? MLNTileSource)?.attributionInfos }
            .flatMap { $0 }
            .map { $0.title.string }
            .reduce(into: [String]()) { unique, title in
                let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty, !unique.contains(trimmed) {
                    unique.append(trimmed)
                }
            }
    }

    /// Current camera angles.
    public var camera: SituationCamera {
        SituationCamera(
            headingDegrees: mapView.direction,
            pitchDegrees: mapView.camera.pitch
        )
    }

    /// Put a coordinate in the middle of the map, keeping zoom, heading and tilt.
    public func centre(on coordinate: CLLocationCoordinate2D, animated: Bool = true) {
        mapView.setCenter(coordinate, animated: animated)
    }

    /// How wide the map reads when a reader asks to be shown where they are.
    ///
    /// Sixty miles is about twenty minutes ahead in a light aircraft, which is the
    /// distance a decision is made over. Zoomed further out the aircraft is a dot among
    /// country; further in and the next airfield is off the screen.
    public static let ownshipWidthNauticalMiles: Double = 60

    /// Centre on a position and set the width of ground the map shows.
    ///
    /// The zoom is computed from the width rather than stored as a level, because a level
    /// means a different distance at every latitude and on every size of window.
    public func centre(
        on coordinate: CLLocationCoordinate2D,
        widthNauticalMiles width: Double,
        animated: Bool = true
    ) {
        let metres = width * 1852
        let points = Double(max(mapView.bounds.width, 1))
        let latitude = coordinate.latitude * .pi / 180
        // Web mercator: the ground each point covers shrinks with the cosine of latitude
        // and halves with every zoom level.
        let metresPerPointAtZoomZero = 156_543.033_928 * cos(latitude) / mapView.contentScaleFactor
        let zoom = log2(metresPerPointAtZoomZero * points / metres)
        let clamped = min(max(zoom, mapView.minimumZoomLevel), mapView.maximumZoomLevel)
        // The direction is carried through so that centring does not also turn the map:
        // where the aircraft is and which way it points are two separate answers.
        mapView.setCenter(
            coordinate,
            zoomLevel: clamped,
            direction: mapView.direction,
            animated: animated
        )
    }

    /// How long a turn or a tilt the reader asked for takes to land.
    ///
    /// The renderer's own default eases for long enough to read as lag on a control that
    /// should feel like a switch.
    private static let cameraMoveDuration: TimeInterval = 0.22

    /// Face the map along a direction, keeping position, zoom and tilt.
    public func setHeading(_ degrees: Double, animated: Bool = true) {
        let camera = mapView.camera
        camera.heading = degrees
        move(to: camera, animated: animated)
    }

    /// Turn the map back to north, keeping everything else.
    public func resetHeading(animated: Bool = true) {
        setHeading(0, animated: animated)
    }

    /// Look straight down, keeping heading and position.
    public func resetPitch(animated: Bool = true) {
        let camera = mapView.camera
        camera.pitch = 0
        move(to: camera, animated: animated)
    }

    private func move(to camera: MLNMapCamera, animated: Bool) {
        guard animated else {
            mapView.setCamera(camera, animated: false)
            return
        }
        mapView.setCamera(
            camera,
            withDuration: Self.cameraMoveDuration,
            animationTimingFunction: CAMediaTimingFunction(name: .easeOut)
        )
    }

    /// Apply one complete display batch when the base style is ready.
    public func apply(_ batch: DisplayBatch) {
        pendingBatch = batch
        applyPendingBatch()
    }

    /// Apply retained display values after a base style load.
    public func mapView(_ mapView: MLNMapView, didFinishLoading style: MLNStyle) {
        applyInitialCamera()
        applyPendingBatch()
        onCameraChanged?(camera)
        onStyleLoaded?(self)
    }

    /// Report the camera after every move the reader makes.
    public func mapView(_ mapView: MLNMapView, regionDidChangeAnimated animated: Bool) {
        onCameraChanged?(camera)
    }

    /// Report the camera while the reader is still moving it, so a control tracks the drag.
    public func mapViewRegionIsChanging(_ mapView: MLNMapView) {
        onCameraChanged?(camera)
    }

    /// Place the camera once, and never against a pilot who has already moved it.
    ///
    /// A style reload raises this callback again, and a second tilt would throw away the
    /// view the pilot chose.
    private func applyInitialCamera() {
        guard !hasAppliedInitialCamera else { return }
        hasAppliedInitialCamera = true
        mapView.minimumZoomLevel = minimumZoomLevel
        mapView.maximumZoomLevel = maximumZoomLevel
        let camera = mapView.camera
        if initialPitchDegrees > 0 {
            camera.pitch = min(initialPitchDegrees, mapView.maximumPitch)
        }
        if let initialCenter {
            camera.centerCoordinate = initialCenter
        }
        mapView.setCamera(camera, animated: false)
        if initialCenter != nil {
            mapView.setZoomLevel(initialZoomLevel, animated: false)
        }
    }

    private func applyPendingBatch() {
        guard let batch = pendingBatch, let style = mapView.style else {
            return
        }
        do {
            applyBaseLayerVisibility(batch.layers, to: style)
            try overlay.apply(batch, to: style)
            // The aircraft's own mark is drawn after the traffic it flies among, so it
            // is never hidden underneath one of them.
            ownshipMark.apply(batch.ownship, to: style)
        } catch {
            onOverlayError?(error)
        }
    }

    private func applyBaseLayerVisibility(
        _ layers: [DisplayLayerControl],
        to style: MLNStyle
    ) {
        for layer in layers {
            guard let identifier = baseLayerIdentifiers[layer.id] else { continue }
            style.layer(withIdentifier: identifier)?.isVisible = layer.enabled
        }
    }

    @objc private func handleTap(_ gesture: UITapGestureRecognizer) {
        guard gesture.state == .ended, !overlay.interactiveLayerIdentifiers.isEmpty else {
            return
        }
        let features = mapView.visibleFeatures(
            at: gesture.location(in: mapView),
            styleLayerIdentifiers: overlay.interactiveLayerIdentifiers
        )
        guard let identifier = features.compactMap({ $0.identifier as? String }).first else {
            return
        }
        onFeatureTapped?(identifier)
    }
}
