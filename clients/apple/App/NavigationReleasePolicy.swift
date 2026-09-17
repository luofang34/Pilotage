import Foundation
import CryptoKit
import PilotageCore

enum NavigationReleasePolicy {
    static func family(_ installed: InstalledAviationRelease) -> String {
        installed.release.authority + ":" + installed.release.coverage.bounds.map { String($0) }.joined(separator: ",")
    }

    static func selected(_ snapshot: AviationDataSnapshot, at now: Date) -> [InstalledAviationRelease] {
        let candidates = snapshot.installed.filter {
            $0.release.product == .navdata && $0.release.validity != nil
                && $0.release.validityLabel(at: now) != "Upcoming"
        }.sorted {
            let left = $0.release.validityLabel(at: now) == "Current"
            let right = $1.release.validityLabel(at: now) == "Current"
            if left != right { return left }
            if $0.release.validity?.effectiveAt != $1.release.validity?.effectiveAt {
                return ($0.release.validity?.effectiveAt ?? .distantPast) > ($1.release.validity?.effectiveAt ?? .distantPast)
            }
            if $0.release.revision != $1.release.revision { return $0.release.revision > $1.release.revision }
            return $0.id < $1.id
        }
        var families = Set<String>()
        return candidates.filter { families.insert(family($0)).inserted }.sorted { $0.id < $1.id }
    }

    static func removable(_ snapshot: AviationDataSnapshot, replacements: [InstalledAviationRelease],
                          retaining: Set<String>, at now: Date) -> [InstalledAviationRelease] {
        let selected = Set(snapshot.selections.map(\.root))
        let dependencies = Set(snapshot.installed.flatMap { $0.release.dependencies })
        return snapshot.installed.filter { old in
            old.release.product == .navdata && !retaining.contains(old.id)
                && !selected.contains(old.id) && !dependencies.contains(old.id)
                && replacements.contains { new in
                    new.id != old.id && family(new) == family(old)
                        && new.release.validityLabel(at: now) == "Current"
                        && (new.release.validity?.effectiveAt ?? .distantPast) >= (old.release.validity?.effectiveAt ?? .distantPast)
                }
        }
    }
}

extension AviationDataWorker {
    func renewNavigation(snapshot: AviationDataSnapshot, opened: [InstalledAviationRelease],
                         retaining: Set<String>, at now: Date) async throws -> AviationDataSnapshot {
        let active = snapshot.active(.navdata)
        let replacement = opened.first { candidate in
            active.map { NavigationReleasePolicy.family($0) == NavigationReleasePolicy.family(candidate) } ?? true
        }
        if let replacement, replacement.id != active?.id,
           !snapshot.selections.contains(where: { $0.name == AviationProduct.navdata.selectionName && $0.pinned }) {
            try await selectNavigation(replacement, allowOutsideValidity: replacement.release.validityLabel(at: now) == "Expired")
        }
        return try await run { session in
            let current = try decodeAviationData(AviationDataSnapshot.self, from: session.snapshotCachedBlocking())
            let removed = NavigationReleasePolicy.removable(current, replacements: opened, retaining: retaining, at: now)
            for old in removed {
                _ = try session.removeBlocking(releaseId: old.id)
                try self.removeNavigationCacheBlocking(releaseID: old.id)
            }
            return try decodeAviationData(AviationDataSnapshot.self, from: session.snapshotCachedBlocking())
        }
    }

    private func removeNavigationCacheBlocking(releaseID: String) throws {
        let directory = root.appendingPathComponent("NavigationSearch")
        guard FileManager.default.fileExists(atPath: directory.path) else { return }
        let digest = SHA256.hash(data: Data(releaseID.utf8)).map { String(format: "%02x", $0) }.joined()
        for file in try FileManager.default.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)
            where file.lastPathComponent.hasPrefix("nav-v1-" + digest + "-") && file.pathExtension == "sqlite" {
            try FileManager.default.removeItem(at: file)
        }
    }
}
