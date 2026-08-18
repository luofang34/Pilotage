import QuartzCore
import SwiftUI
import UIKit

/// Routes decoded video frames straight into layers, entirely outside
/// SwiftUI's invalidation. Publishing a picture through `@Published`
/// re-evaluated the whole rack — map included — up to ninety times a
/// second; the interface thread stalled for seconds, the demand timer
/// with it, and the host's silence watchdog took the lease. Here a
/// frame is one coalesced `layer.contents` swap and nothing else hears
/// about it.
final class VideoFrameHub: @unchecked Sendable {
    private let lock = NSLock()
    private var latest: [UInt8: UIImage] = [:]
    private var layers: [UInt8: CALayer] = [:]
    private var dirty: Set<UInt8> = []
    private var flushScheduled = false

    /// Accepts one decoded frame from any thread; the layer swap lands
    /// in one coalesced main-queue pass, latest picture wins.
    func publish(_ image: UIImage, source: UInt8) {
        lock.lock()
        latest[source] = image
        dirty.insert(source)
        let schedule = !flushScheduled
        flushScheduled = true
        lock.unlock()
        if schedule {
            DispatchQueue.main.async { [weak self] in self?.flush() }
        }
    }

    /// Binds a tile's layer to a source and paints the newest picture
    /// it may have missed.
    func attach(layer: CALayer, source: UInt8) {
        lock.lock()
        layers[source] = layer
        let image = latest[source]
        lock.unlock()
        layer.contents = image?.cgImage
    }

    /// Unbinds a layer from a source — but only the layer that owns
    /// the slot. A stale view tearing down must never evict the live
    /// one that replaced it.
    func detach(layer: CALayer, source: UInt8) {
        lock.lock()
        if layers[source] === layer {
            layers.removeValue(forKey: source)
        }
        lock.unlock()
    }

    private func flush() {
        lock.lock()
        let work = dirty.compactMap { source in
            layers[source].map { ($0, latest[source]) }
        }
        dirty.removeAll()
        flushScheduled = false
        lock.unlock()
        for (layer, image) in work {
            layer.contents = image?.cgImage
        }
    }
}
/// Hosts one video source's picture in a plain layer the frame hub
/// paints directly: no picture ever crosses SwiftUI, so a sixty-hertz
/// feed re-evaluates nothing but its own layer contents.
struct VideoSurfaceView: UIViewRepresentable {
    let hub: VideoFrameHub
    let source: UInt8

    final class Coordinator {
        var hub: VideoFrameHub?
        var source: UInt8 = 0
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.layer.contentsGravity = .resizeAspect
        context.coordinator.hub = hub
        context.coordinator.source = source
        hub.attach(layer: view.layer, source: source)
        return view
    }

    func updateUIView(_ view: UIView, context: Context) {
        if context.coordinator.source != source {
            context.coordinator.hub?.detach(layer: view.layer, source: context.coordinator.source)
        }
        context.coordinator.hub = hub
        context.coordinator.source = source
        hub.attach(layer: view.layer, source: source)
    }

    static func dismantleUIView(_ uiView: UIView, coordinator: Coordinator) {
        // A departing tile releases its slot, so the hub never paints
        // into a layer that left the hierarchy.
        coordinator.hub?.detach(layer: uiView.layer, source: coordinator.source)
    }
}
