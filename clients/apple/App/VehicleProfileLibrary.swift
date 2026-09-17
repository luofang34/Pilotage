import Foundation
import PilotageCore

struct VehicleProfile: Codable, Equatable, Identifiable, Sendable {
    var id = UUID().uuidString
    var revision: UInt64 = 1
    var name: String
    var registration: String?
    var kind = "aircraft"
    var cruiseSpeedKnots: Double
    var speedReference = "true_airspeed"
    var source: String?
    var title: String { registration.map { $0 + " · " + name } ?? name }
    var speedLabel: String { speedReference == "true_airspeed" ? "KTAS" : "kt ground speed" }
}

struct VehicleProfileDocument: Codable, Equatable, Sendable {
    var schemaVersion = 1
    var profiles: [VehicleProfile]

    static func validated(_ json: String) throws -> Self {
        try PlanningCodec.decode(Self.self, validateVehicleProfiles(documentJson: json))
    }
}

enum VehicleProfileError: LocalizedError {
    case unavailable, duplicate(String), tooLarge
    var errorDescription: String? {
        switch self {
        case .unavailable: "The vehicle library is not ready."
        case let .duplicate(name): "A different profile already uses the ID for \(name). Edit that profile or use a new ID."
        case .tooLarge: "The profile file exceeds 1 MB."
        }
    }
}

actor VehicleProfileStore {
    let url: URL
    init(url: URL) { self.url = url }
    func load() throws -> VehicleProfileDocument {
        guard FileManager.default.fileExists(atPath: url.path) else { return VehicleProfileDocument(profiles: []) }
        return try Self.read(url)
    }
    static func read(_ url: URL) throws -> VehicleProfileDocument {
        let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard size <= 1_048_576 else { throw VehicleProfileError.tooLarge }
        return try .validated(String(contentsOf: url, encoding: .utf8))
    }
    func save(_ document: VehicleProfileDocument) throws {
        let json = try PlanningCodec.encode(document)
        _ = try VehicleProfileDocument.validated(json)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(json.utf8).write(to: url, options: .atomic)
    }
}

@MainActor
final class VehicleProfileLibrary: ObservableObject {
    @Published private(set) var profiles: [VehicleProfile] = []
    @Published private(set) var errorMessage: String?
    @Published private(set) var busy = false
    private var store: VehicleProfileStore?

    func start(url: URL) async {
        guard store == nil else { return }
        do {
            let opened = VehicleProfileStore(url: url)
            profiles = try await opened.load().profiles
            store = opened
            errorMessage = nil
        } catch { errorMessage = error.localizedDescription }
    }

    func save(_ profile: VehicleProfile) async throws {
        var next = profiles
        if let index = next.firstIndex(where: { $0.id == profile.id }) {
            var updated = profile
            updated.revision = next[index].revision &+ 1
            next[index] = updated
        } else { next.append(profile) }
        try await persist(next)
    }

    func importProfiles(_ document: VehicleProfileDocument) async throws {
        var next = profiles
        for profile in document.profiles {
            if let existing = next.first(where: { $0.id == profile.id }) {
                guard existing == profile else { throw VehicleProfileError.duplicate(profile.name) }
            } else { next.append(profile) }
        }
        try await persist(next)
    }

    func remove(_ id: String) async throws { try await persist(profiles.filter { $0.id != id }) }

    private func persist(_ next: [VehicleProfile]) async throws {
        guard let store, !busy else { throw VehicleProfileError.unavailable }
        busy = true
        defer { busy = false }
        let document = VehicleProfileDocument(profiles: next)
        try await store.save(document)
        profiles = next.sorted { $0.title.localizedStandardCompare($1.title) == .orderedAscending }
        errorMessage = nil
    }
}
