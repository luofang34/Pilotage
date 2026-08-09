import CoreGraphics
import IndicateAppleDisplay

/// A scene producer that is available only after compatibility verification.
public final class PilotageInstrumentSceneProducer: SceneProducing {
    private let verifiedRuntime: VerifiedInstrumentRuntime
    private let panel: UInt32

    public init(verifiedRuntime: VerifiedInstrumentRuntime, panel: UInt32) {
        self.verifiedRuntime = verifiedRuntime
        self.panel = panel
    }

    public func frame(designFrame: CGRect) throws -> SceneFrame {
        let outcome = verifiedRuntime.runtime.render(panel: panel)
        guard outcome.status == 0 else {
            let code = UInt16(exactly: outcome.status)
            let reason = code.flatMap(DisplayReason.init(rawValue:)) ?? .renderTrap
            throw ProducerFault(reason: reason == .ok ? .renderTrap : reason)
        }
        guard outcome.frameWidth == Float(designFrame.width),
              outcome.frameHeight == Float(designFrame.height) else {
            throw ProducerFault(reason: .abiMismatch)
        }
        guard !outcome.scene.isEmpty else {
            throw ProducerFault(reason: .sceneFraming)
        }
        return SceneFrame(bytes: outcome.scene, generation: outcome.generation)
    }
}
