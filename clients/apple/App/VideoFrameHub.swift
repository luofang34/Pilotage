import QuartzCore
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

    /// Unbinds whatever layer serves this source.
    func detach(source: UInt8) {
        lock.lock()
        layers.removeValue(forKey: source)
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
