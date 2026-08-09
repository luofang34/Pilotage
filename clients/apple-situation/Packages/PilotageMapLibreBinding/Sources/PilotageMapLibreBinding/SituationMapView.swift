@preconcurrency import MapLibre
import PilotageSituationCore
import UIKit

/// An iPadOS map that applies portable display batches.
@MainActor
public final class SituationMapView: UIView, @preconcurrency MLNMapViewDelegate {
    /// Receives an error that occurs after the map style loads.
    public var onOverlayError: ((Error) -> Void)?

    private let mapView: MLNMapView
    private let overlay = SituationOverlay()
    private var pendingBatch: DisplayBatch?

    /// Create a map with the specified base style when it is available.
    public init(frame: CGRect = .zero, styleURL: URL?) {
        mapView = MLNMapView(frame: .zero, styleURL: styleURL)
        super.init(frame: frame)
        mapView.translatesAutoresizingMaskIntoConstraints = false
        mapView.delegate = self
        addSubview(mapView)
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
        applyPendingBatch()
    }

    private func applyPendingBatch() {
        guard let batch = pendingBatch, let style = mapView.style else {
            return
        }
        do {
            try overlay.apply(batch, to: style)
        } catch {
            onOverlayError?(error)
        }
    }
}
