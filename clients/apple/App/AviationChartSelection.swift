import Foundation
import PilotageCore

extension AviationDataWorker {
    func prepareMap(snapshot: AviationDataSnapshot, charts: [AviationChartStyle]) async throws -> AviationMapStyle {
        try await run { session in
            var geographic: [AviationProduct: InstalledAviationRelease] = [:]
            for product in [AviationProduct.terrain, .basemap] {
                guard let installed = snapshot.active(product) ?? snapshot.installed.first(where: {
                    $0.release.product == product && $0.artifactURL(format: "mbtiles") != nil
                }) else { throw AviationChartError.invalid("Install the \(product.title) package.") }
                try session.selectBlocking(request: DataSelectionRequest(
                    name: product.selectionName, releaseId: installed.id, pinned: false,
                    now: Int64(Date().timeIntervalSince1970), development: installed.release.channel == "development",
                    allowOutsideValidity: false, rendererCapabilities: AviationChartStyle.rendererCapabilities
                ))
                geographic[product] = installed
            }
            guard let terrain = geographic[.terrain], let coastline = geographic[.basemap],
                  let template = Bundle.main.url(forResource: "SituationStyle", withExtension: "json") else {
                throw AviationChartError.invalid("The geographic base is incomplete.")
            }
            return try AviationMapStyle.assemble(terrain: terrain, coastline: coastline, charts: charts,
                                                  template: Data(contentsOf: template))
        }
    }

    func selectChart(
        _ installed: InstalledAviationRelease, allowOutsideValidity: Bool
    ) async throws -> AviationChartStyle {
        try await run { session in
            _ = try session.verifyBlocking(releaseId: installed.id)
            let chart = try AviationChartStyle.loadBlocking(installed)
            try session.selectBlocking(request: DataSelectionRequest(
                name: installed.release.product.selectionName, releaseId: installed.id,
                pinned: false, now: Int64(Date().timeIntervalSince1970),
                development: installed.release.channel == "development",
                allowOutsideValidity: allowOutsideValidity,
                rendererCapabilities: AviationChartStyle.rendererCapabilities
            ))
            return chart
        }
    }
}

extension AviationProduct {
    var isChart: Bool { self == .ifrLow || self == .ifrHigh }
}
