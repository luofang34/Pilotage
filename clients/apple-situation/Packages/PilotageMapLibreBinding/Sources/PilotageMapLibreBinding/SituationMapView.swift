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
    private var hasAppliedInitialPitch = false

    /// Viewing angle away from straight down, in degrees, taken once the style loads.
    ///
    /// The renderer drapes nothing over the elevation model, so shading and colour carry
    /// height on their own and only from a camera that looks across the ground. A pilot
    /// can still tilt the map by hand afterwards.
    public var initialPitchDegrees: CGFloat = 0

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
        applyInitialPitch()
        applyPendingBatch()
    }

    /// Tilt the camera once, and never against a pilot who has already moved it.
    ///
    /// A style reload raises this callback again, and a second tilt would throw away the
    /// view the pilot chose.
    private func applyInitialPitch() {
        guard !hasAppliedInitialPitch, initialPitchDegrees > 0 else {
            return
        }
        hasAppliedInitialPitch = true
        let camera = mapView.camera
        camera.pitch = min(initialPitchDegrees, mapView.maximumPitch)
        mapView.setCamera(camera, animated: false)
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
