import Foundation
import PilotageCore

@MainActor
final class AviationDataModel: ObservableObject {
    @Published private(set) var snapshot = AviationDataSnapshot.empty
    @Published private(set) var available: [AviationRelease] = []
    @Published private(set) var busy = false
    @Published private(set) var status = "Opening installed data…"
    @Published private(set) var errorMessage: String?
    @Published private(set) var progress: DataTransferProgress?
    private var worker: AviationDataWorker?
    private var publisherJSON: String?
    private var started = false

    var updatesConfigured: Bool { publisherJSON != nil }

    func start() async {
        guard !started else { return }
        started = true
        await perform("Checking installed data…") {
            let worker = try await AviationDataWorker.open()
            self.worker = worker
            if let url = Bundle.main.url(forResource: "AviationPublisher", withExtension: "json") {
                self.publisherJSON = try String(contentsOf: url, encoding: .utf8)
            }
            if let examples = Bundle.main.url(forResource: "AviationDataExamples", withExtension: nil) {
                do {
                    try await worker.importExamples(from: examples)
                } catch {
                    try await self.reload()
                    throw error
                }
            }
            try await self.reload()
            if let publisher = self.publisherJSON {
                do {
                    let json = try await worker.run {
                        try $0.catalogBlocking(publisherJson: publisher, now: Int64(Date().timeIntervalSince1970))
                    }
                    self.available = try decodeAviationData(AviationCatalog.self, from: json).releases
                } catch {
                    self.available = []
                }
            }
        }
    }

    func refresh() async {
        guard let worker, let publisher = publisherJSON else { return }
        await perform("Checking for updates…") {
            let json = try await worker.run {
                try $0.refreshBlocking(publisherJson: publisher, now: Int64(Date().timeIntervalSince1970))
            }
            self.available = try decodeAviationData(AviationCatalog.self, from: json).releases
        }
    }

    func download(_ release: AviationRelease) async {
        guard let worker, let publisher = publisherJSON else { return }
        let observer = AviationTransferObserver { [weak self] progress in
            Task { @MainActor in self?.progress = progress }
        }
        await perform("Downloading \(release.product.title)…") {
            try await worker.run {
                try $0.downloadBlocking(
                    publisherJson: publisher, releaseId: release.id,
                    now: Int64(Date().timeIntervalSince1970),
                    availableBytes: worker.availableBytesBlocking(), observer: observer
                )
            }
            try await self.reload()
        }
    }

    func verify(_ installed: InstalledAviationRelease) async {
        guard let worker else { return }
        await perform("Checking \(installed.release.product.title)…", success: "All package files passed verification.") {
            _ = try await worker.run { try $0.verifyBlocking(releaseId: installed.id) }
        }
    }

    func remove(_ installed: InstalledAviationRelease) async {
        guard let worker else { return }
        await perform("Removing \(installed.release.product.title)…") {
            _ = try await worker.run { try $0.removeBlocking(releaseId: installed.id) }
            try await self.reload()
        }
    }

    func cancel() { worker?.cancel() }

    func retainedReason(_ installed: InstalledAviationRelease) -> String? {
        if let selection = snapshot.selections.first(where: { $0.root == installed.id }) {
            return selection.pinned ? "Retained for a flight or replay" : "Selected"
        }
        if let owner = snapshot.installed.first(where: { $0.release.dependencies.contains(installed.id) }) {
            return "Required by \(owner.release.product.title)"
        }
        return nil
    }

    private func reload() async throws {
        guard let worker else { return }
        let json = try await worker.run { try $0.snapshotCachedBlocking() }
        snapshot = try decodeAviationData(AviationDataSnapshot.self, from: json)
    }

    private func perform(_ message: String, success: String = "Installed data is available offline.",
        operation: () async throws -> Void) async
    {
        guard !busy else { return }
        busy = true
        status = message
        errorMessage = nil
        progress = nil
        do {
            try await operation()
            status = snapshot.installed.isEmpty ? "No data package is installed." : success
        } catch {
            errorMessage = error.localizedDescription
            status = "The data operation could not finish."
        }
        busy = false
        progress = nil
    }
}
