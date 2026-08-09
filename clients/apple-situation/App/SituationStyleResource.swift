import Foundation

enum SituationStyleResource {
    static let fallbackJSON = """
    {"version":8,"name":"Pilotage terrain unavailable","sources":{},"layers":[{"id":"background","type":"background","paint":{"background-color":"#0b1721"}}]}
    """

    private static let archiveToken = "__PILOTAGE_TERRAIN_MBTILES_URL__"

    static func load(bundle: Bundle = .main) throws -> String {
        guard let styleURL = bundle.url(forResource: "SituationStyle", withExtension: "json") else {
            throw SituationStyleResourceError.missingStyle
        }
        guard let archiveFileURL = bundle.url(
            forResource: "SituationTerrain",
            withExtension: "mbtiles"
        ) else {
            throw SituationStyleResourceError.missingArchive
        }
        let data = try Data(contentsOf: styleURL)
        let object = try JSONSerialization.jsonObject(with: data)
        guard var style = object as? [String: Any],
              var sources = style["sources"] as? [String: Any],
              var terrain = sources["pilotage-terrain"] as? [String: Any],
              terrain["url"] as? String == archiveToken else {
            throw SituationStyleResourceError.invalidTemplate
        }
        terrain["url"] = try archiveURL(for: archiveFileURL).absoluteString
        sources["pilotage-terrain"] = terrain
        style["sources"] = sources
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
    case missingArchive
    case invalidTemplate
    case invalidEncoding
    case invalidArchiveURL

    var errorDescription: String? {
        switch self {
        case .missingStyle:
            "The application bundle has no SituationStyle.json resource."
        case .missingArchive:
            "The application bundle has no SituationTerrain.mbtiles resource."
        case .invalidTemplate:
            "SituationStyle.json has no terrain archive token."
        case .invalidEncoding:
            "The resolved situation style is not UTF-8 text."
        case .invalidArchiveURL:
            "The terrain archive does not have an absolute file path."
        }
    }
}
