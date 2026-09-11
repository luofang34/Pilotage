import Foundation

struct AviationProcedureChart: Decodable, Identifiable, Equatable, Sendable {
    let id: String
    let airport: String
    let name: String
    let chartCode: String
    let artifact: String
}

struct AviationProcedureCatalog: Sendable {
    let installed: InstalledAviationRelease
    let charts: [AviationProcedureChart]

    private struct Index: Decodable {
        let schemaVersion: Int
        let edition: String
        let charts: [AviationProcedureChart]
    }

    static func loadBlocking(_ installed: InstalledAviationRelease) throws -> Self {
        guard installed.release.product == .procedures,
              let index = installed.release.artifacts.first(where: { $0.path == "procedures.json" && $0.format == "resource" }) else {
            throw AviationChartError.invalid("The procedure index is absent.")
        }
        let url = try AviationChartStyle.artifactURL(index, installed: installed)
        return try decode(Data(contentsOf: url), installed: installed)
    }

    static func decode(_ data: Data, installed: InstalledAviationRelease) throws -> Self {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let index = try decoder.decode(Index.self, from: data)
        guard index.schemaVersion == 1, index.edition == installed.release.edition,
              !index.charts.isEmpty, Set(index.charts.map(\.id)).count == index.charts.count else {
            throw AviationChartError.invalid("The procedure index has an invalid edition or chart identity.")
        }
        let catalog = Self(installed: installed, charts: index.charts)
        for chart in index.charts {
            guard !chart.id.isEmpty, !chart.airport.isEmpty, !chart.name.isEmpty else {
                throw AviationChartError.invalid("A procedure chart has no identity.")
            }
            _ = try catalog.url(for: chart)
        }
        return catalog
    }

    func url(for chart: AviationProcedureChart) throws -> URL {
        guard let artifact = installed.release.artifacts.first(where: {
            $0.path == chart.artifact && $0.format == "pdf"
        }) else { throw AviationChartError.invalid("The procedure PDF is absent from this release.") }
        return try AviationChartStyle.artifactURL(artifact, installed: installed)
    }
}
