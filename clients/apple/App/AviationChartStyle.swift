import Foundation

enum AviationChartError: LocalizedError {
    case invalid(String)

    var errorDescription: String? {
        switch self {
        case .invalid(let reason): "The installed chart cannot open. \(reason)"
        }
    }
}

struct AviationChartStyle: Equatable, Sendable {
    static let rendererCapabilities = ["ifr-chart-v1"]
    let installed: InstalledAviationRelease
    let json: String
    var id: String { installed.id }

    static func loadBlocking(_ installed: InstalledAviationRelease) throws -> Self {
        let styles = installed.release.artifacts.filter { $0.format == "map_style" }
        guard styles.count == 1, let artifact = styles.first, artifact.bytes <= 8 * 1024 * 1024 else {
            throw AviationChartError.invalid("One supported style file is required.")
        }
        let url = try artifactURL(artifact, installed: installed)
        let data = try Data(contentsOf: url)
        guard data.count == artifact.bytes else {
            throw AviationChartError.invalid("The style file size does not match the release.")
        }
        return try resolve(data, installed: installed)
    }

    static func resolve(_ data: Data, installed: InstalledAviationRelease) throws -> Self {
        guard var style = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              var metadata = style["metadata"] as? [String: Any],
              let artifacts = metadata["pilotage:resource-artifacts"] as? [[String: String]],
              !artifacts.isEmpty else {
            throw AviationChartError.invalid("The style has no installed resource list.")
        }
        var bindings: [[String: String]] = []
        var uris = Set<String>()
        for binding in artifacts {
            guard let uri = binding["uri"], validURI(uri), uris.insert(uri).inserted,
                  let path = binding["path"], let format = binding["format"],
                  let artifact = installed.release.artifacts.first(where: { $0.path == path }),
                  artifact.format == (format == "file" ? "resource" : format),
                  ["file", "pmtiles", "mbtiles"].contains(format) else {
                throw AviationChartError.invalid("A resource is absent or has an invalid binding.")
            }
            bindings.append(["uri": uri, "path": try artifactURL(artifact, installed: installed).path,
                             "format": format])
        }
        try validateSources(style, bindings: bindings)
        // Installed paths come from the store. A downloaded style cannot choose them.
        metadata["pilotage:resources"] = bindings
        style["metadata"] = metadata
        let resolved = try JSONSerialization.data(withJSONObject: style, options: [.sortedKeys])
        guard let json = String(data: resolved, encoding: .utf8) else {
            throw AviationChartError.invalid("The style is not UTF-8 text.")
        }
        return Self(installed: installed, json: json)
    }

    private static func artifactURL(
        _ artifact: AviationArtifact, installed: InstalledAviationRelease
    ) throws -> URL {
        let parts = artifact.path.split(separator: "/", omittingEmptySubsequences: false)
        guard !parts.isEmpty, parts.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." }),
              !artifact.path.contains("\\") else {
            throw AviationChartError.invalid("A resource path is invalid.")
        }
        let root = URL(fileURLWithPath: installed.directory, isDirectory: true).resolvingSymlinksInPath()
        let url = root.appendingPathComponent(artifact.path).resolvingSymlinksInPath()
        guard url.path.hasPrefix(root.path + "/") else {
            throw AviationChartError.invalid("A resource is outside the installed release.")
        }
        return url
    }

    private static func validURI(_ uri: String) -> Bool {
        guard uri.hasPrefix("pilotage://") else { return false }
        let path = uri.dropFirst("pilotage://".count)
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.")
        return path.split(separator: "/", omittingEmptySubsequences: false).allSatisfy {
            !$0.isEmpty && $0 != "." && $0 != ".." && $0.unicodeScalars.allSatisfy(allowed.contains)
        }
    }

    private static func validateSources(_ style: [String: Any], bindings: [[String: String]]) throws {
        guard style["glyphs"] == nil, style["imports"] == nil,
              let sources = style["sources"] as? [String: [String: Any]], !sources.isEmpty,
              let sprite = style["sprite"] as? String else {
            throw AviationChartError.invalid("The chart must use installed sources and symbols.")
        }
        let files = Set(bindings.filter { $0["format"] == "file" }.compactMap { $0["uri"] })
        guard files.contains(sprite + ".png"), files.contains(sprite + ".json") else {
            throw AviationChartError.invalid("The symbol image or index is absent.")
        }
        let tiles = Set(bindings.filter { $0["format"] != "file" }.compactMap { $0["uri"] }.map {
            $0 + "/{z}/{x}/{y}"
        })
        for source in sources.values {
            guard source["type"] as? String == "vector", source["url"] == nil,
                  let templates = source["tiles"] as? [String], templates.count == 1,
                  templates.allSatisfy(tiles.contains) else {
                throw AviationChartError.invalid("A tile source does not use an installed archive.")
            }
        }
    }
}
