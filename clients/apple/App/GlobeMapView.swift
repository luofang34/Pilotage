import CoreLocation
import MetalKit
import OSLog
import PilotageMapLibreBinding
import PilotageCore
import simd

@MainActor
final class GlobeMapView: MTKView, MTKViewDelegate, UIGestureRecognizerDelegate {
    var onCameraChanged: ((SituationCamera) -> Void)?
    var onMovedByReader: (() -> Void)?
    var onFeatureTapped: ((String) -> Void)?
    var batch: DisplayBatch? { didSet { overlay.batch = batch } }
    private let overlay = GlobeSituationOverlay()
    private var camera = GlobeCamera()
    private var cameraMotion: GlobeCameraMotion?
    private var renderer: GlobeRenderer?
    private var style: AviationMapStyle?
    private var mode = "terrain"
    private var modeRequestedAt: Double?
    private var failed = false
    private let message = UILabel()
    private let logger = Logger(subsystem: "org.luofang.pilotage", category: "Globe")

    init() {
        super.init(frame: .zero, device: MTLCreateSystemDefaultDevice())
        delegate = self
        colorPixelFormat = .bgra8Unorm
        framebufferOnly = false
        preferredFramesPerSecond = 30
        autoResizeDrawable = false
        backgroundColor = .black
        addSubview(overlay)
        message.text = "Loading globe…"
        message.textColor = .white
        message.numberOfLines = 0
        message.textAlignment = .center
        message.translatesAutoresizingMaskIntoConstraints = false
        addSubview(message)
        NSLayoutConstraint.activate([
            message.centerXAnchor.constraint(equalTo: centerXAnchor),
            message.centerYAnchor.constraint(equalTo: centerYAnchor),
            message.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor, multiplier: 0.8),
        ])
        let drag = UIPanGestureRecognizer(target: self, action: #selector(pan))
        drag.maximumNumberOfTouches = 1
        addGestureRecognizer(drag)
        let zoom = UIPinchGestureRecognizer(target: self, action: #selector(pinch))
        zoom.delegate = self
        addGestureRecognizer(zoom)
        let rotation = UIRotationGestureRecognizer(target: self, action: #selector(rotate))
        rotation.delegate = self
        addGestureRecognizer(rotation)
        let tilt = UIPanGestureRecognizer(target: self, action: #selector(tilt))
        tilt.minimumNumberOfTouches = 2
        addGestureRecognizer(tilt)
        let tap = UITapGestureRecognizer(target: self, action: #selector(zoomIn))
        tap.numberOfTapsRequired = 2
        addGestureRecognizer(tap)
        let selection = UITapGestureRecognizer(target: self, action: #selector(selectFeature))
        selection.require(toFail: tap)
        addGestureRecognizer(selection)
        accessibilityLabel = "Globe map"
        accessibilityHint = "Drag to move. Pinch to zoom. Drag with two fingers to tilt."
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { super.init(coder: coder) }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        (gestureRecognizer is UIPinchGestureRecognizer && otherGestureRecognizer is UIRotationGestureRecognizer)
            || (gestureRecognizer is UIRotationGestureRecognizer && otherGestureRecognizer is UIPinchGestureRecognizer)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        overlay.frame = bounds
        guard bounds.width > 0, bounds.height > 0 else { return }
        let scale = min(2, 1536 / max(bounds.width, bounds.height))
        let size = CGSize(width: floor(bounds.width * scale), height: floor(bounds.height * scale))
        if drawableSize != size { drawableSize = size }
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {
        renderer = nil
        failed = false
    }

    func draw(in view: MTKView) {
        guard !failed, window != nil, drawableSize.width > 1, drawableSize.height > 1 else { return }
        updateCameraMotion()
        do {
            if renderer == nil {
                guard let style else { return }
                renderer = try GlobeRenderer(
                    width: Int(drawableSize.width), height: Int(drawableSize.height), style: style
                )
                try renderer?.selectMode(mode, requestedAt: modeRequestedAt)
            }
            guard let drawable = currentDrawable else { return }
            renderer?.setTerrainVisible(batch?.layers.first { $0.id == "terrain-base" }?.enabled ?? true)
            try renderer?.draw(camera: camera, drawable: drawable)
            overlay.project = { [weak self] coordinates in
                guard let self else { return [] }
                return renderer?.project(coordinates, into: bounds.size) ?? []
            }
            overlay.heading = camera.displayHeading
            overlay.setNeedsDisplay()
            message.isHidden = true
        } catch {
            failed = true
            message.isHidden = false
            message.text = "The map could not load.\n\(error.localizedDescription)"
            logger.error("Globe renderer: \(error.localizedDescription, privacy: .public)")
        }
    }

    func configure(style: AviationMapStyle, mode: String, requestedAt: Double?) {
        if self.style?.id != style.id {
            self.style = style
            renderer = nil
            failed = false
            message.text = "Loading map data…"
            message.isHidden = false
        }
        guard self.mode != mode else { return }
        self.mode = mode
        modeRequestedAt = requestedAt
        do {
            try renderer?.selectMode(mode, requestedAt: requestedAt)
        } catch {
            message.text = error.localizedDescription
            message.isHidden = false
        }
        accessibilityLabel = AviationProduct(rawValue: mode)?.title ?? "Map"
    }

    func stop() {
        isPaused = true
        delegate = nil
        renderer = nil
    }

    func changeCamera(animated: Bool = false, _ change: (inout GlobeCamera) -> Void) {
        var target = camera
        change(&target)
        if animated && !UIAccessibility.isReduceMotionEnabled {
            cameraMotion = GlobeCameraMotion(start: camera, target: target, startedAt: CACurrentMediaTime())
        } else {
            cameraMotion = nil
            camera = target
            publishCamera()
        }
    }

    private func updateCameraMotion() {
        guard let motion = cameraMotion else { return }
        let now = CACurrentMediaTime()
        camera = motion.value(at: now)
        if now >= motion.startedAt + GlobeCameraMotion.duration { cameraMotion = nil }
        publishCamera()
    }

    private func publishCamera() {
        onCameraChanged?(SituationCamera(
            headingDegrees: camera.displayHeading, pitchDegrees: camera.displayPitch,
            canTilt: camera.overviewWeight == 0
        ))
    }

    func centre(on coordinate: CLLocationCoordinate2D, frame: Bool, animated: Bool) {
        changeCamera(animated: animated) {
            $0.latitude = coordinate.latitude
            $0.longitude = coordinate.longitude
            if frame { $0.distance = 134_000 }
        }
    }

    @objc private func pan(_ gesture: UIPanGestureRecognizer) {
        guard gesture.numberOfTouches == 1 else { return }
        let delta = gesture.translation(in: self)
        gesture.setTranslation(.zero, in: self)
        onMovedByReader?()
        changeCamera { $0.pan(x: delta.x, y: delta.y, height: min(bounds.width, bounds.height)) }
    }

    @objc private func pinch(_ gesture: UIPinchGestureRecognizer) {
        onMovedByReader?()
        changeCamera { $0.zoom(by: gesture.scale) }
        gesture.scale = 1
    }

    @objc private func rotate(_ gesture: UIRotationGestureRecognizer) {
        onMovedByReader?()
        changeCamera { $0.heading -= gesture.rotation * 180 / .pi }
        gesture.rotation = 0
    }

    @objc private func tilt(_ gesture: UIPanGestureRecognizer) {
        let delta = gesture.translation(in: self)
        gesture.setTranslation(.zero, in: self)
        onMovedByReader?()
        changeCamera { $0.pitch = min(max($0.pitch + delta.y * 0.2, 0), 80) }
    }

    @objc private func zoomIn(_ gesture: UITapGestureRecognizer) {
        onMovedByReader?()
        changeCamera { $0.zoom(by: 2) }
    }

    @objc private func selectFeature(_ gesture: UITapGestureRecognizer) {
        guard let identifier = overlay.feature(at: gesture.location(in: overlay)) else { return }
        onFeatureTapped?(identifier)
    }
}
