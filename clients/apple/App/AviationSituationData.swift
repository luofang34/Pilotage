import Foundation
import PilotageCore

struct AviationSituationData: Sendable {
    let terrainID: String
    let terrainURL: URL
    let navigationID: String?
    let navigationCycle: Data?
}

extension AviationDataWorker {
    func selectNavigation(_ installed: InstalledAviationRelease, allowOutsideValidity: Bool) async throws {
        try await run { session in
            _ = try session.verifyBlocking(releaseId: installed.id)
            guard installed.release.product == .navdata,
                  let url = installed.artifactURL(format: "acnav") else {
                throw AviationChartError.invalid("The navigation cycle is absent.")
            }
            // Decode before selection so an unreadable cycle cannot replace usable data.
            _ = try PresentationSession().loadWeatherStationsFromCycle(cycleBytes: Data(contentsOf: url))
            try session.selectBlocking(request: DataSelectionRequest(
                name: AviationProduct.navdata.selectionName, releaseId: installed.id, pinned: false,
                now: Int64(Date().timeIntervalSince1970), development: installed.release.channel == "development",
                allowOutsideValidity: allowOutsideValidity, rendererCapabilities: []
            ))
        }
    }

    func situationData(snapshot: AviationDataSnapshot) async throws -> AviationSituationData {
        try await run { session in
            guard let terrain = snapshot.active(.terrain), let url = terrain.artifactURL(format: "mbtiles") else {
                throw AviationChartError.invalid("Select installed terrain data.")
            }
            _ = try session.verifyBlocking(releaseId: terrain.id)
            let navigation = snapshot.active(.navdata)
            var cycle: Data?
            if let navigation {
                _ = try session.verifyBlocking(releaseId: navigation.id)
                guard let url = navigation.artifactURL(format: "acnav") else {
                    throw AviationChartError.invalid("The selected navigation cycle is absent.")
                }
                cycle = try Data(contentsOf: url)
            }
            return AviationSituationData(terrainID: terrain.id, terrainURL: url,
                                         navigationID: navigation?.id, navigationCycle: cycle)
        }
    }
}

@MainActor
final class SituationDataConsumer {
    let session = PresentationSession()
    private(set) var installed: AviationSituationData?
    var terrainArchivePath: String? { installed?.terrainURL.path }

    func load(_ data: AviationSituationData) async throws -> DisplayBatch {
        let session = session
        let terrainChanged = installed?.terrainID != data.terrainID
        let navigationChanged = installed?.navigationID != data.navigationID
        let batch = try await Task.detached(priority: .utility) {
            if terrainChanged {
                try session.loadTerrainArchiveBlocking(archivePath: data.terrainURL.path)
            }
            if navigationChanged, let cycle = data.navigationCycle {
                return try session.loadWeatherStationsFromCycle(cycleBytes: cycle)
            }
            if navigationChanged {
                return try session.replaceWeatherStationPositions(positions: [])
            }
            return try session.currentDisplay(nowMicros: DispatchTime.now().uptimeNanoseconds / 1_000)
        }.value
        installed = data
        return batch
    }
}
