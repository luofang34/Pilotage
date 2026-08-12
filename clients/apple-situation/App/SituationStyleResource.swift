import Foundation

enum SituationStyleResource {
    static let fallbackJSON = """
    {"version":8,"name":"Pilotage terrain unavailable","sources":{},"layers":[{"id":"background","type":"background","paint":{"background-color":"#0b1721"}}]}
    """

    private static let coastlineArchiveToken = "__PILOTAGE_COASTLINE_MBTILES_URL__"
    private static let terrainArchiveToken = "__PILOTAGE_TERRAIN_MBTILES_URL__"

    /// Closest zoom the map allows.
    ///
    /// A raster-dem source draws past its deepest tile by stretching the one it has. Two
    /// doublings keep a close view usable while the shape on screen still comes from
    /// measured ground; past that the picture is invention. The value is read from the
    /// manifest that ships beside the archive, so a plan that gains a closer band raises
    /// the ceiling with it instead of leaving a second number to change by hand.
    /// Doublings of stretch allowed past the deepest tile.
    static let overzoomSteps: Double = 2

    static var maximumZoomLevel: Double {
        guard let url = Bundle.main.url(
            forResource: "SituationTerrain.manifest",
            withExtension: "json"
        ),
            let data = try? Data(contentsOf: url),
            let manifest = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let bands = manifest["bands"] as? [[String: Any]] else { return 14 }
        let deepest = bands.compactMap { $0["max_zoom"] as? Double }.max() ?? 13
        return deepest + overzoomSteps
    }

    /// The notice each source in the style asks to be shown.
    ///
    /// Read from the style document the map is built from, so a source added without its
    /// notice cannot appear on the map. Taken from the document rather than from the
    /// loaded map because the panel that shows these is opened before the map has
    /// finished loading as often as after, and a credit that depends on timing is a
    /// credit that is sometimes missing.
    static func attributions(bundle: Bundle = .main) -> [String] {
        guard let url = bundle.url(forResource: "SituationStyle", withExtension: "json"),
              let data = try? Data(contentsOf: url),
              let style = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let sources = style["sources"] as? [String: Any] else { return [] }
        return sources.values
            .compactMap { ($0 as? [String: Any])?["attribution"] as? String }
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .sorted()
    }

    static func load(bundle: Bundle = .main) throws -> String {
        guard let styleURL = bundle.url(forResource: "SituationStyle", withExtension: "json") else {
            throw SituationStyleResourceError.missingStyle
        }
        guard let coastlineArchiveFileURL = bundle.url(
            forResource: "SituationCoastline",
            withExtension: "mbtiles"
        ) else {
            throw SituationStyleResourceError.missingCoastlineArchive
        }
        guard let terrainArchiveFileURL = bundle.url(
            forResource: "SituationTerrain",
            withExtension: "mbtiles"
        ) else {
            throw SituationStyleResourceError.missingTerrainArchive
        }
        let data = try Data(contentsOf: styleURL)
        let object = try JSONSerialization.jsonObject(with: data)
        guard var style = object as? [String: Any],
              var sources = style["sources"] as? [String: Any],
              var coastline = sources["pilotage-coastline"] as? [String: Any],
              coastline["url"] as? String == coastlineArchiveToken,
              var terrain = sources["pilotage-terrain"] as? [String: Any],
              terrain["url"] as? String == terrainArchiveToken else {
            throw SituationStyleResourceError.invalidTemplate
        }
        coastline["url"] = try archiveURL(for: coastlineArchiveFileURL).absoluteString
        terrain["url"] = try archiveURL(for: terrainArchiveFileURL).absoluteString
        sources["pilotage-coastline"] = coastline
        sources["pilotage-terrain"] = terrain
        style["sources"] = sources
#if PILOTAGE_MAPLIBRE_TERRAIN
        style["terrain"] = [
            "source": "pilotage-terrain",
            "exaggeration": 1.0,
        ]
#endif
        let resolved = try JSONSerialization.data(withJSONObject: style, options: [.sortedKeys])
        guard let json = String(data: resolved, encoding: .utf8) else {
            throw SituationStyleResourceError.invalidEncoding
        }
        return json
    }

    private static func archiveURL(for fileURL: URL) throws -> URL {
        guard fileURL.isFileURL,
              var components = URLComponents(
                  url: fileURL.absoluteURL,
                  resolvingAgainstBaseURL: false
              ) else {
            throw SituationStyleResourceError.invalidArchiveURL
        }
        components.scheme = "mbtiles"
        guard let archiveURL = components.url, archiveURL.path.hasPrefix("/") else {
            throw SituationStyleResourceError.invalidArchiveURL
        }
        return archiveURL
    }
}

private enum SituationStyleResourceError: LocalizedError {
    case missingStyle
    case missingCoastlineArchive
    case missingTerrainArchive
    case invalidTemplate
    case invalidEncoding
    case invalidArchiveURL

    var errorDescription: String? {
        switch self {
        case .missingStyle:
            "The application bundle has no SituationStyle.json resource."
        case .missingCoastlineArchive:
            "The application bundle has no SituationCoastline.mbtiles resource."
        case .missingTerrainArchive:
            "The application bundle has no SituationTerrain.mbtiles resource."
        case .invalidTemplate:
            "SituationStyle.json has an invalid archive token."
        case .invalidEncoding:
            "The resolved situation style is not UTF-8 text."
        case .invalidArchiveURL:
            "The terrain archive does not have an absolute file path."
        }
    }
}
