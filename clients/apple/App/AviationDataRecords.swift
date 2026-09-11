import Foundation

enum AviationProduct: String, Codable, Sendable {
    case navdata, terrain, ifrLow = "ifr_low", ifrHigh = "ifr_high", procedures, basemap

    var title: String {
        switch self {
        case .navdata: "Navigation"
        case .terrain: "Terrain"
        case .ifrLow: "IFR Low"
        case .ifrHigh: "IFR High"
        case .procedures: "Procedures"
        case .basemap: "Base map"
        }
    }

    var selectionName: String { "active-" + rawValue.replacingOccurrences(of: "_", with: "-") }
}

struct AviationRelease: Decodable, Identifiable, Equatable, Sendable {
    let id: String
    let product: AviationProduct
    let authority: String
    let edition: String
    let revision: UInt64
    let channel: String
    let validity: AviationValidity?
    let coverage: AviationCoverage
    let artifacts: [AviationArtifact]
    let dependencies: [String]
    let rendererCapabilities: [String]
    let attributions: [String]

    var bytes: Int64 { artifacts.reduce(0) { $0 + $1.bytes } }

    func validityLabel(at date: Date = Date()) -> String {
        guard let validity else { return "No scheduled expiry" }
        if date < validity.effectiveAt { return "Upcoming" }
        if date >= validity.expiresAt { return "Expired" }
        return "Current"
    }
}

struct AviationValidity: Decodable, Equatable, Sendable {
    let effectiveAt: Date
    let expiresAt: Date
}

struct AviationCoverage: Decodable, Equatable, Sendable {
    let name: String
    let bounds: [Double]
    let minZoom: UInt8
    let maxZoom: UInt8
    let complete: Bool
    let exclusions: [String]
}

struct AviationArtifact: Decodable, Equatable, Sendable {
    let path: String
    let format: String
    let bytes: Int64
    let sha256: String
}

struct InstalledAviationRelease: Decodable, Identifiable, Equatable, Sendable {
    let release: AviationRelease
    let directory: String
    var id: String { release.id }

    func artifactURL(format: String) -> URL? {
        release.artifacts.first { $0.format == format }.map {
            URL(fileURLWithPath: directory, isDirectory: true).appendingPathComponent($0.path)
        }
    }
}

struct AviationSelection: Decodable, Equatable, Sendable {
    let name: String
    let root: String
    let pinned: Bool
}

struct AviationDataSnapshot: Decodable, Equatable, Sendable {
    let installed: [InstalledAviationRelease]
    let selections: [AviationSelection]
    static let empty = AviationDataSnapshot(installed: [], selections: [])

    func active(_ product: AviationProduct) -> InstalledAviationRelease? {
        guard let selection = selections.first(where: { $0.name == product.selectionName }) else { return nil }
        return installed.first { $0.id == selection.root }
    }
}

struct AviationCatalog: Decodable, Sendable {
    let releases: [AviationRelease]
    let expiresAt: Date
}

func decodeAviationData<T: Decodable>(_ type: T.Type, from json: String) throws -> T {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    decoder.dateDecodingStrategy = .secondsSince1970
    return try decoder.decode(type, from: Data(json.utf8))
}
