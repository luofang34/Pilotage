import Foundation
import CryptoKit
import PilotageCore

extension AviationDataWorker {
    func retainMission(_ id: String, sources: [NavigationSource], snapshot: AviationDataSnapshot) async throws {
        let prefix = "mission-" + String(missionRetentionDigest(id).prefix(32)) + "-"
        let unique = Set(sources.map(\.releaseId))
        let requests = try unique.sorted().map { releaseId -> DataSelectionRequest in
            guard let installed = snapshot.installed.first(where: { $0.id == releaseId }) else {
                throw AviationChartError.invalid("The mission requires navigation release \(releaseId).")
            }
            let suffix = missionRetentionDigest(releaseId)
            return DataSelectionRequest(name: prefix + suffix, releaseId: releaseId, pinned: true,
                now: Int64(Date().timeIntervalSince1970), development: installed.release.channel == "development",
                allowOutsideValidity: true, rendererCapabilities: [])
        }
        try await run { session in
            for request in requests { try session.selectBlocking(request: request) }
            let names = Set(requests.map(\.name))
            for selection in snapshot.selections where selection.name.hasPrefix(prefix) && !names.contains(selection.name) {
                try session.releaseSelectionBlocking(name: selection.name)
            }
        }
    }
}

private func missionRetentionDigest(_ value: String) -> String {
    SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
}
