import Foundation
import PilotageAppleInstrumentConsumer
import pilotage_instrument_apple_bridge

/// Adapts the generated UniFFI module to the Apple consumer contract.
public final class GeneratedInstrumentRuntime: InstrumentRuntimeServing {
    private let bridge: InstrumentBridge

    public static var identity: InstrumentRuntimeIdentity {
        InstrumentRuntimeIdentity(
            stateABI: stateAbiVersion(),
            sceneFormat: sceneFormatVersion(),
            corpusVersion: corpusVersion(),
            corpusDigest: corpusDigestHex(),
            registrySceneDigest: sceneDigestHex(),
            screenCompositionDigest: compositionDigestHex()
        )
    }

    /// Verifies the linked identity before it creates the Rust runtime.
    public static func verifiedRuntime() throws -> VerifiedInstrumentRuntime {
        try AppleInstrumentCompatibilityGate.verify(identity) {
            GeneratedInstrumentRuntime()
        }
    }

    private init() {
        bridge = InstrumentBridge()
    }

    public func writeState(_ bytes: [UInt8]) throws {
        let outcome = bridge.writeState(bytes: Data(bytes))
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

    public func render(panel: UInt32) -> InstrumentRuntimeRenderOutcome {
        let outcome = bridge.render(panel: panel)
        return InstrumentRuntimeRenderOutcome(
            status: outcome.status,
            scene: Array(outcome.scene),
            frameWidth: outcome.frameWidth,
            frameHeight: outcome.frameHeight,
            generation: outcome.generation
        )
    }
}
