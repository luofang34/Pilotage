import Foundation
import PilotageCore

/// All file verification and database writes use one serial queue.
final class AviationDataWorker: @unchecked Sendable {
    private let queue = DispatchQueue(label: "org.luofang.pilotage.aviation-data", qos: .utility)
    private let session: AviationDataSession
    let root: URL

    private init(session: AviationDataSession, root: URL) {
        self.session = session
        self.root = root
    }

    static func open() async throws -> AviationDataWorker {
        try await Task.detached(priority: .utility) {
            var root = try FileManager.default.url(
                for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
            ).appendingPathComponent("AviationData", isDirectory: true)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try root.setResourceValues(values)
            return AviationDataWorker(session: try AviationDataSession.openBlocking(root: root.path), root: root)
        }.value
    }

    func run<T: Sendable>(
        _ operation: @escaping @Sendable (AviationDataSession) throws -> T
    ) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { [session] in
                continuation.resume(with: Result { try operation(session) })
            }
        }
    }

    func cancel() { session.cancelDownload() }

    func availableBytesBlocking() throws -> UInt64 {
        let values = try root.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
        let reserve: Int64 = 128 * 1024 * 1024
        return UInt64(max(0, (values.volumeAvailableCapacityForImportantUsage ?? 0) - reserve))
    }

    func importExamples(from directory: URL, navigationOnly: Bool = false) async throws {
        try await run { [self] session in
            let index = try Data(contentsOf: directory.appendingPathComponent("index.json"))
            let manifests = try JSONDecoder().decode([String].self, from: index)
            let receipt = root.appendingPathComponent("bundled-examples.json")
            var imported: Set<String> = []
            if FileManager.default.fileExists(atPath: receipt.path) {
                imported = try JSONDecoder().decode(Set<String>.self, from: Data(contentsOf: receipt))
            }
            for name in manifests {
                let url = directory.appendingPathComponent(name)
                let manifest = try String(contentsOf: url, encoding: .utf8)
                let release = try decodeAviationData(AviationRelease.self, from: manifest)
                if navigationOnly && release.product != .navdata { continue }
                guard !imported.contains(release.id) else { continue }
                try session.importBundledBlocking(
                    manifestJson: manifest, sourceDirectory: url.deletingLastPathComponent().path,
                    availableBytes: availableBytesBlocking()
                )
                imported.insert(release.id)
                try JSONEncoder().encode(imported).write(to: receipt, options: .atomic)
            }
        }
    }
}

final class AviationTransferObserver: DataDownloadObserver, @unchecked Sendable {
    private let receive: @Sendable (DataTransferProgress) -> Void
    init(receive: @escaping @Sendable (DataTransferProgress) -> Void) { self.receive = receive }
    func progress(progress: DataTransferProgress) { receive(progress) }
}
