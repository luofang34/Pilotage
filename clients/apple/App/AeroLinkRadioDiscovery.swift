@preconcurrency import AeroLinkAppleClient
import Foundation
import SystemExtensions

final class AeroLinkDiscoveryGate: @unchecked Sendable {
    private let discovery = ALDriverDiscovery()
    private let lock = NSLock()

    func openConnections() -> (
        connections: [AeroLinkConnectionHandle],
        hadOpenFailures: Bool,
        failure: AeroLinkFailure?
    ) {
        lock.withLock {
            do {
                let connections = try discovery.openConnections().map {
                    AeroLinkConnectionHandle(value: $0)
                }
                return (
                    connections,
                    discovery.lastDiscoveryHadOpenFailures,
                    discovery.lastDiscoveryError.map {
                        AeroLinkFailure.classify($0, for: nil)
                    }
                )
            } catch {
                return (
                    [],
                    discovery.lastDiscoveryHadOpenFailures,
                    AeroLinkFailure.classify(error, for: nil)
                )
            }
        }
    }

    func shouldContinue(for connections: [AeroLinkConnectionHandle]) -> Bool {
        lock.withLock {
            discovery.shouldContinueDiscovery(for: connections.map(\.value))
        }
    }

    func driverIsEnabled() -> Bool? {
        guard let hostID = Bundle.main.bundleIdentifier,
              let driverID = Bundle.main.object(
                  forInfoDictionaryKey: "AeroLinkDriverBundleIdentifier"
              ) as? String else { return nil }
        do {
            let extensions = try OSSystemExtensionsWorkspace.shared
                .systemExtensions(forApplicationWithBundleID: hostID)
            return extensions.contains {
                $0.bundleIdentifier == driverID && $0.isEnabled
            }
        } catch {
            return nil
        }
    }
}
