import Foundation
import Metal
import QuartzCore
import simd

enum GlobeRenderError: LocalizedError {
    case missingStyle
    case startup
    case frame
    case presentation

    var errorDescription: String? {
        switch self {
        case .missingStyle: "The globe style is missing."
        case .startup: "The map renderer could not start."
        case .frame: "The map renderer could not draw a frame."
        case .presentation: "Metal could not present the map."
        }
    }
}

/// A Metal frame uses the same command queue as the map renderer.
final class GlobeRenderer {
    private let map: OpaquePointer
    private let queue: MTLCommandQueue
    private let width: Int
    private let height: Int
    private let style: AviationMapStyle
    private var mapMode = "terrain"
    private var modeRequestedAt: Double?
    private var switchMilliseconds: Double?
    private let instanceID = UUID().uuidString
    private var releaseID: String? { style.releases.first { $0.release.product.rawValue == mapMode }?.id }
    private var frames: UInt64 = 0

    init(width: Int, height: Int, style: AviationMapStyle) throws {
        self.style = style
        let cache = try FileManager.default.url(
            for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        ).appendingPathComponent("MapLibreGlobe", isDirectory: true)
        try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
        guard let map = maplibre_visionos_create(style.json, UInt32(width), UInt32(height), cache.path) else {
            throw GlobeRenderError.startup
        }
        guard let rawQueue = maplibre_visionos_command_queue(map) else {
            maplibre_visionos_destroy(map)
            throw GlobeRenderError.startup
        }
        self.map = map
        queue = Unmanaged<AnyObject>.fromOpaque(rawQueue).takeUnretainedValue() as! MTLCommandQueue
        self.width = width
        self.height = height
        maplibre_visionos_note("Pilotage map renderer started: \(mapMode), release \(releaseID ?? "online")")
    }

    func selectMode(_ mode: String, requestedAt: Double?) throws {
        guard maplibre_visionos_set_map_mode(map, mode) else { throw GlobeRenderError.startup }
        guard mode != mapMode else { return }
        mapMode = mode
        modeRequestedAt = requestedAt ?? CACurrentMediaTime()
    }

    func setTerrainVisible(_ visible: Bool) {
        maplibre_visionos_set_layer_visible(map, "pilotage-terrain-hillshade", visible)
    }

    func project(_ coordinates: [Double], into size: CGSize) -> [CGPoint?] {
        var screen = [Double](repeating: .nan, count: coordinates.count / 3 * 2)
        guard maplibre_visionos_project_locations(map, coordinates, coordinates.count / 3, &screen) else { return [] }
        return stride(from: 0, to: screen.count, by: 2).map { index in
            guard screen[index].isFinite, screen[index + 1].isFinite else { return nil }
            return CGPoint(x: screen[index] * size.width / Double(width),
                           y: screen[index + 1] * size.height / Double(height))
        }
    }

    deinit { maplibre_visionos_destroy(map) }

    func draw(camera: GlobeCamera, drawable: CAMetalDrawable) throws {
        let texturePointer = maplibre_visionos_render_chart(
            map, camera.latitude, camera.longitude, camera.chartZoom(width: width, height: height),
            camera.displayHeading, camera.displayPitch, CACurrentMediaTime()
        )
        guard let texturePointer else { throw GlobeRenderError.frame }
        let texture = Unmanaged<AnyObject>.fromOpaque(texturePointer).takeUnretainedValue() as! MTLTexture
        try present(texture, to: drawable)
        frames &+= 1
        let changed = modeRequestedAt != nil
        if let requested = modeRequestedAt {
            switchMilliseconds = (CACurrentMediaTime() - requested) * 1000
            modeRequestedAt = nil
        }
        if frames == 1 || changed || frames % 300 == 0 { recordEvidence(camera: camera) }
    }

    private func present(_ texture: MTLTexture, to drawable: CAMetalDrawable) throws {
        guard let command = queue.makeCommandBuffer(), let copy = command.makeBlitCommandEncoder() else {
            throw GlobeRenderError.presentation
        }
        copy.copy(
            from: texture, sourceSlice: 0, sourceLevel: 0, sourceOrigin: MTLOrigin(),
            sourceSize: MTLSize(width: width, height: height, depth: 1),
            to: drawable.texture, destinationSlice: 0, destinationLevel: 0, destinationOrigin: MTLOrigin()
        )
        copy.endEncoding()
        if let requested = modeRequestedAt {
            let instance = instanceID
            let mode = mapMode
            #if targetEnvironment(simulator)
            command.addCompletedHandler { _ in
                Self.recordPresentation(mode: mode, instance: instance, requested: requested,
                                        presented: CACurrentMediaTime(), measurement: "modeSwitchCompletedMilliseconds")
            }
            #else
            drawable.addPresentedHandler { presented in
                Self.recordPresentation(mode: mode, instance: instance, requested: requested,
                                        presented: presented.presentedTime, measurement: "modeSwitchPresentedMilliseconds")
            }
            #endif
        }
        command.present(drawable)
        command.commit()
    }

    private static func recordPresentation(mode: String, instance: String, requested: Double, presented: Double, measurement: String) {
        guard presented >= requested else { return }
        do {
            let evidence: [String: Any] = [
                "mapMode": mode, "rendererInstance": instance,
                measurement: (presented - requested) * 1000,
                "timestamp": Date().timeIntervalSince1970,
            ]
            let data = try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys])
            let documents = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask,
                                                        appropriateFor: nil, create: true)
            try data.write(to: documents.appendingPathComponent("map-presentation-\(mode).json"), options: .atomic)
        } catch {
            maplibre_visionos_note("Map presentation evidence could not be saved: \(error.localizedDescription)")
        }
    }

    private func recordEvidence(camera: GlobeCamera) {
        do {
            let revisionURL = Bundle.main.url(forResource: "GlobeRevision", withExtension: "txt")
            let revision = try revisionURL.map { try String(contentsOf: $0, encoding: .utf8) }
            let evidence: [String: Any] = [
                "renderer": "maplibre-rs", "projection": "globe", "revision": revision ?? "unknown",
                "framesSubmitted": frames, "width": width, "height": height,
                "latitude": camera.latitude, "longitude": camera.longitude,
                "distanceMeters": camera.distance, "timestamp": Date().timeIntervalSince1970,
                "mapMode": mapMode, "releaseID": releaseID ?? "online",
                "installedResources": releaseID != nil,
                "cameraSource": "map-view", "rendererInstance": instanceID,
                "modeSwitchSubmissionMilliseconds": switchMilliseconds ?? 0,
                "chartZoom": camera.chartZoom(width: width, height: height),
                "headingDegrees": camera.displayHeading, "pitchDegrees": camera.displayPitch,
            ]
            let data = try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys])
            let documents = try FileManager.default.url(
                for: .documentDirectory, in: .userDomainMask, appropriateFor: nil, create: true
            )
            try data.write(to: documents.appendingPathComponent("globe-renderer-evidence.json"), options: .atomic)
        } catch {
            maplibre_visionos_note("Globe evidence could not be saved: \(error.localizedDescription)")
        }
    }
}
