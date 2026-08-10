@preconcurrency import MapLibre
import PilotageSituationCore
import UIKit

/// An iPadOS map that applies portable display batches.
@MainActor
public final class SituationMapView: UIView, @preconcurrency MLNMapViewDelegate {
    /// Receives an error that occurs after the map style loads.
    public var onOverlayError: ((Error) -> Void)?
    /// Receives the stable identity of a tapped display feature.
    public var onFeatureTapped: ((String) -> Void)?
    /// Maps portable base-layer identities to base-style layer identities.
    public var baseLayerIdentifiers: [String: String] = [:]

    private let mapView: MLNMapView
    private let overlay = SituationOverlay()
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

    /// Create a map with the specified base style JSON.
    public init(frame: CGRect = .zero, styleJSON: String) {
        mapView = MLNMapView(frame: .zero, styleJSON: styleJSON)
        super.init(frame: frame)
        mapView.translatesAutoresizingMaskIntoConstraints = false
        mapView.delegate = self
        addSubview(mapView)
        addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(handleTap)))
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

    /// Apply one complete display batch when the base style is ready.
    public func apply(_ batch: DisplayBatch) {
        pendingBatch = batch
        applyPendingBatch()
    }

    /// Apply retained display values after a base style load.
    public func mapView(_ mapView: MLNMapView, didFinishLoading style: MLNStyle) {
        applyInitialCamera()
        applyPendingBatch()
    }

    /// Place the camera once, and never against a pilot who has already moved it.
    ///
    /// A style reload raises this callback again, and a second tilt would throw away the
    /// view the pilot chose.
    private func applyInitialCamera() {
        guard !hasAppliedInitialCamera else { return }
        hasAppliedInitialCamera = true
        mapView.minimumZoomLevel = minimumZoomLevel
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
