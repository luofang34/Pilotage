import Foundation

struct AviationMapStyle: Equatable, Sendable {
    let id: String
    let json: String
    let releases: [InstalledAviationRelease]

    static func assemble(
        terrain: InstalledAviationRelease, coastline: InstalledAviationRelease,
        charts: [AviationChartStyle], template: Data
    ) throws -> Self {
        guard var style = try JSONSerialization.jsonObject(with: template) as? [String: Any],
              var sources = style["sources"] as? [String: [String: Any]],
              var layers = style["layers"] as? [[String: Any]],
              let terrainURL = terrain.artifactURL(format: "mbtiles"),
              let coastlineURL = coastline.artifactURL(format: "mbtiles") else {
            throw AviationChartError.invalid("The geographic base is incomplete.")
        }
        var bindings = [
            ["uri": "pilotage://terrain", "path": terrainURL.path, "format": "mbtiles"],
            ["uri": "pilotage://coastline", "path": coastlineURL.path, "format": "mbtiles"],
        ]
        for (name, uri, maximum) in [("pilotage-terrain", "terrain", terrain.release.coverage.maxZoom),
                                     ("pilotage-coastline", "coastline", coastline.release.coverage.maxZoom)] {
            sources[name]?.removeValue(forKey: "url")
            sources[name]?["tiles"] = ["pilotage://\(uri)/{z}/{x}/{y}"]
            sources[name]?["minzoom"] = 0
            sources[name]?["maxzoom"] = maximum
        }
        for index in layers.indices where ["hillshade", "color-relief"].contains(layers[index]["type"] as? String ?? "") {
            layers[index]["metadata"] = ["maplibre:display-group": "terrain"]
        }
        style.removeValue(forKey: "glyphs")
        style["projection"] = ["type": "globe"]
        var spriteDigests: [String: String] = [:]
        for chart in charts.sorted(by: { $0.id < $1.id }) {
            try append(chart, sources: &sources, layers: &layers, bindings: &bindings, spriteDigests: &spriteDigests)
        }
        if !charts.isEmpty { style["sprite"] = "pilotage://symbols/point-sprites" }
        style["sources"] = sources
        style["layers"] = layers
        style["metadata"] = [
            "pilotage:resources": bindings,
            "maplibre:display-groups": ["terrain"] + charts.map { $0.installed.release.product.rawValue },
            "maplibre:active-display-group": "terrain",
        ]
        let releases = [terrain, coastline] + charts.map(\.installed)
        let bytes = try JSONSerialization.data(withJSONObject: style, options: [.sortedKeys])
        return Self(id: releases.map(\.id).sorted().joined(separator: ":"), json: String(decoding: bytes, as: UTF8.self), releases: releases)
    }

    private static func append(
        _ chart: AviationChartStyle, sources: inout [String: [String: Any]], layers: inout [[String: Any]],
        bindings: inout [[String: String]], spriteDigests: inout [String: String]
    ) throws {
        guard let style = try JSONSerialization.jsonObject(with: Data(chart.json.utf8)) as? [String: Any],
              let chartSources = style["sources"] as? [String: [String: Any]],
              let chartLayers = style["layers"] as? [[String: Any]],
              let metadata = style["metadata"] as? [String: Any],
              let resources = metadata["pilotage:resources"] as? [[String: String]],
              let artifacts = metadata["pilotage:resource-artifacts"] as? [[String: String]] else {
            throw AviationChartError.invalid("The chart has no resolved resource list.")
        }
        var uriMap: [String: String] = [:]
        for resource in resources {
            guard let uri = resource["uri"], let relative = artifacts.first(where: { $0["uri"] == uri })?["path"],
                  let artifact = chart.installed.release.artifacts.first(where: { $0.path == relative }) else {
                throw AviationChartError.invalid("A chart resource has no content digest.")
            }
            var binding = resource
            if resource["format"] == "file" {
                if let previous = spriteDigests[uri], previous != artifact.sha256 {
                    throw AviationChartError.invalid("Select chart editions with matching symbols.")
                }
                spriteDigests[uri] = artifact.sha256
                uriMap[uri] = uri
            } else {
                uriMap[uri] = "pilotage://archive/" + artifact.sha256
            }
            binding["uri"] = uriMap[uri]
            if !bindings.contains(where: { $0["uri"] == binding["uri"] }) { bindings.append(binding) }
        }
        let group = chart.installed.release.product.rawValue
        for sourceLayer in ["ocean", "land", "lakes"] {
            layers.append([
                "id": group + ":paper-" + sourceLayer, "type": "fill", "source": "pilotage-coastline",
                "source-layer": sourceLayer, "minzoom": 5,
                "metadata": ["maplibre:display-group": group], "paint": ["fill-color": "#f8f7f2"],
            ])
        }
        for (name, var source) in chartSources {
            guard let old = (source["tiles"] as? [String])?.first,
                  let replacement = uriMap.first(where: { old == $0.key + "/{z}/{x}/{y}" }) else {
                throw AviationChartError.invalid("A chart source has no archive binding.")
            }
            source["tiles"] = [replacement.value + "/{z}/{x}/{y}"]
            sources[group + ":" + name] = source
        }
        for var layer in chartLayers where layer["type"] as? String != "background" {
            guard let identifier = layer["id"] as? String else { continue }
            layer["id"] = group + ":" + identifier
            if let source = layer["source"] as? String { layer["source"] = group + ":" + source }
            var metadata = layer["metadata"] as? [String: Any] ?? [:]
            metadata["maplibre:display-group"] = group
            layer["metadata"] = metadata
            layers.append(layer)
        }
    }
}
