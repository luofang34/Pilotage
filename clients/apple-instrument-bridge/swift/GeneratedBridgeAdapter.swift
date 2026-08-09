import Foundation
import PilotageAppleInstrumentConsumer
import pilotage_instrument_apple_bridge

/// Adapts the generated UniFFI module to the Apple consumer contract.
public final class GeneratedInstrumentRuntime: InstrumentRuntimeServing {
    private let bridge: InstrumentBridge

    public static var identity: InstrumentRuntimeIdentity {
        let asset = controlledGlyphAsset()
        return identity(glyphRecordedHash: hex(asset.recordedHash))
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
    public static func verifiedRuntime() throws -> VerifiedInstrumentRuntime {
        let asset = controlledGlyphAsset()
        let atlas = try PilotageGlyphAtlas(asset: asset)
        let actual = identity(glyphRecordedHash: hex(asset.recordedHash))
        return try AppleInstrumentCompatibilityGate.verify(actual, glyphAtlas: atlas) {
            GeneratedInstrumentRuntime()
        }
    }

    private init() {
        bridge = InstrumentBridge()
    }

    public func writeState(_ bytes: [UInt8], acceptedAtMs: UInt64) throws {
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

    public func compositionFrame(
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

private func hex(_ bytes: [UInt8]) -> String {
    bytes.map { String(format: "%02x", $0) }.joined()
}
