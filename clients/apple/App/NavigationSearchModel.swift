import Foundation
import PilotageCore

struct NavigationSource: Codable, Equatable, Sendable {
    let releaseId: String
    let authority: String
    let edition: String
    let sourceDigest: String
    let effectiveAt: Int64
    let expiresAt: Int64
}

struct NavigationPoint: Codable, Equatable, Sendable {
    let key: String
    let identifier: String
    let kind: String
    let name: String
    let region: String
    let latitudeDeg: Double
    let longitudeDeg: Double
}

struct NavigationMatch: Codable, Equatable, Identifiable, Sendable {
    let point: NavigationPoint
    let source: NavigationSource
    var id: String { source.releaseId + ":" + point.key }
}

enum PlanningCodec {
    static func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(type, from: Data(json.utf8))
    }

    static func encode<T: Encodable>(_ value: T) throws -> String {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        encoder.outputFormatting = [.sortedKeys]
        return String(decoding: try encoder.encode(value), as: UTF8.self)
    }
}

@MainActor
final class NavigationSearchModel: ObservableObject {
    @Published private(set) var sources: [InstalledAviationRelease] = []
    @Published private(set) var status = "Opening navigation data…"
    @Published private(set) var errorMessage: String?
    @Published private(set) var revision: UInt64 = 0
    @Published private(set) var isReady = false
    private let session = NavigationSearchSession()
    private var fingerprint: [String]?

    func configure(worker: AviationDataWorker, snapshot: AviationDataSnapshot) async throws {
        let candidates = Self.selectedSources(snapshot)
        let next = candidates.map { $0.id + ":" + ($0.release.artifacts.first?.sha256 ?? "") }
        guard fingerprint != next else { return }
        let session = session
        do {
            let requests = try candidates.map { installed -> NavigationIndexRequest in
                guard let artifact = installed.release.artifacts.first(where: { $0.format == "nav_sqlite" })
                    ?? installed.release.artifacts.first(where: { $0.format == "acnav" }),
                      let validity = installed.release.validity,
                      let url = installed.artifactURL(format: artifact.format) else {
                    throw AviationChartError.invalid("The navigation search artifact is absent.")
                }
                return NavigationIndexRequest(releaseId: installed.id, authority: installed.release.authority,
                    path: url.path, format: artifact.format,
                    effectiveAt: Int64(validity.effectiveAt.timeIntervalSince1970),
                    expiresAt: Int64(validity.expiresAt.timeIntervalSince1970), artifactDigest: artifact.sha256)
            }
            try await worker.run { data in
                for request in requests { _ = try data.verifyBlocking(releaseId: request.releaseId) }
                try session.replaceSourcesBlocking(sources: requests,
                    cacheDirectory: worker.root.appendingPathComponent("NavigationSearch").path)
            }
            sources = candidates
            fingerprint = next
            isReady = true
            revision &+= 1
            status = candidates.isEmpty ? "Install navigation data in Data to search airports and waypoints."
                : "Search \(candidates.count) installed navigation source\(candidates.count == 1 ? "" : "s") offline."
            errorMessage = nil
        } catch {
            errorMessage = error.localizedDescription
            throw error
        }
    }

    func search(_ query: String) async throws -> [NavigationMatch] {
        let session = session
        return try await Task.detached(priority: .userInitiated) {
            let json = try session.searchBlocking(query: query, limit: 60, now: Int64(Date().timeIntervalSince1970))
            return try PlanningCodec.decode([NavigationMatch].self, json)
        }.value
    }

    static func selectedSources(_ snapshot: AviationDataSnapshot, at now: Date = Date()) -> [InstalledAviationRelease] {
        NavigationReleasePolicy.selected(snapshot, at: now)
    }
}
