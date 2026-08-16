import Foundation
import PilotageAppleInstrumentConsumer
import PilotageCore

/// Adapts the linked UniFFI bridge to the Apple consumer contract.
///
/// The application links one static library carrying both generated
/// surfaces, so the bridge symbols arrive through `PilotageCore` rather
/// than a standalone module; everything else is the consumer's own
/// before-paint gate (ADR-0032).
final class LinkedInstrumentRuntime: InstrumentRuntimeServing {
    private let bridge: InstrumentBridge

    static var identity: InstrumentRuntimeIdentity {
        let asset = controlledGlyphAsset()
        return identity(glyphRecordedHash: hexString(asset.recordedHash))
    }

    private static func identity(glyphRecordedHash: String) -> InstrumentRuntimeIdentity {
        InstrumentRuntimeIdentity(
            stateABI: stateAbiVersion(),
            sceneFormat: sceneFormatVersion(),
            corpusVersion: corpusVersion(),
            corpusDigest: corpusDigestHex(),
            registrySceneDigest: sceneDigestHex(),
            screenCompositionDigest: compositionDigestHex(),
            glyphRecordedHash: glyphRecordedHash
        )
    }

    private static func controlledGlyphAsset() -> InstrumentGlyphAsset {
        let asset = glyphAsset()
        return InstrumentGlyphAsset(
            canonical: Array(asset.canonical),
            recordedHash: Array(asset.recordedHash)
        )
    }

    /// Verifies the linked identity before it creates the Rust runtime.
    /// A mismatch throws, and nothing paints (ADR-0032's tuple gate).
    static func verifiedRuntime() throws -> VerifiedInstrumentRuntime {
        let asset = controlledGlyphAsset()
        let atlas = try PilotageGlyphAtlas(asset: asset)
        let actual = identity(glyphRecordedHash: hexString(asset.recordedHash))
        return try AppleInstrumentCompatibilityGate.verify(actual, glyphAtlas: atlas) {
            LinkedInstrumentRuntime()
        }
    }

    private init() {
        bridge = InstrumentBridge()
    }

    func writeState(_ bytes: [UInt8], acceptedAtMs: UInt64) throws {
        let outcome = bridge.writeState(bytes: Data(bytes), acceptedAtMs: acceptedAtMs)
        switch outcome.status {
        case 0:
            return
        case 1:
            throw InstrumentStateWriteError.frameTooLarge(
                actual: outcome.actual,
                capacity: outcome.capacity
            )
        default:
            throw InstrumentStateWriteError.unexpectedStatus(outcome.status)
        }
    }

    func compositionFrame(
        nowMs: UInt64,
        pathHealthy: Bool
    ) -> InstrumentRuntimeCompositionOutcome {
        let outcome = bridge.compositionFrame(nowMs: nowMs, pathHealthy: pathHealthy)
        let panels = outcome.panels.map { panel in
            InstrumentRuntimePanelOutcome(
                panel: panel.panel,
                status: panel.status,
                sceneOffset: panel.sceneOffset,
                sceneLength: panel.sceneLen,
                frameWidth: panel.frameWidth,
                frameHeight: panel.frameHeight,
                generation: panel.generation
            )
        }
        return InstrumentRuntimeCompositionOutcome(
            status: outcome.status,
            scene: Array(outcome.scene),
            panels: panels,
            generation: outcome.generation,
            alertStatus: outcome.alertStatus,
            activeAlertCount: outcome.activeAlertCount,
            alertPathFaulted: outcome.alertPathFaulted,
            alertOverflow: outcome.alertOverflow,
            alertManagerGeneration: outcome.alertManagerGeneration
        )
    }
}

private func hexString(_ bytes: [UInt8]) -> String {
    bytes.map { String(format: "%02x", $0) }.joined()
}
