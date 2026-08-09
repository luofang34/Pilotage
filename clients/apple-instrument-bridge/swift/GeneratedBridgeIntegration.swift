import CoreGraphics
import IndicateAppleDisplay
import PilotageAppleInstrumentConsumer
import pilotage_instrument_apple_bridge

private enum BridgeIntegrationError: Error {
    case insufficientPanels(actual: UInt32)
    case missingPanel(index: UInt32)
    case invalidLayerMask(index: UInt32, mask: UInt32)
    case invalidRequirements(index: UInt32)
    case emptyComposition
    case failedRender(index: UInt32, reason: UInt16)
    case missingImage(index: UInt32)
    case missingReport(index: UInt32)
    case unsatisfiedReport(index: UInt32)
}

@main
private enum GeneratedBridgeIntegration {
    static func main() throws {
        guard panelCount() >= 2 else {
            throw BridgeIntegrationError.insufficientPanels(actual: panelCount())
        }
        let verified = try GeneratedInstrumentRuntime.verifiedRuntime()
        let composition = PilotageInstrumentComposition(verifiedRuntime: verified)
        try composition.writeState([7, 0], acceptedAtMs: 100)
        guard try composition.compose(nowMs: 100, pathHealthy: true) > 0 else {
            throw BridgeIntegrationError.emptyComposition
        }

        try verifyPanel(index: 0, composition: composition, verified: verified)
        try verifyPanel(index: 1, composition: composition, verified: verified)
    }

    private static func verifyPanel(
        index: UInt32,
        composition: PilotageInstrumentComposition,
        verified: VerifiedInstrumentRuntime
    ) throws {
        guard let descriptor = panelDescriptor(index: index) else {
            throw BridgeIntegrationError.missingPanel(index: index)
        }
        guard let layerMask = UInt8(exactly: descriptor.requiredLayers) else {
            throw BridgeIntegrationError.invalidLayerMask(
                index: index,
                mask: descriptor.requiredLayers
            )
        }
        let size = CGSize(
            width: CGFloat(descriptor.designWidth),
            height: CGFloat(descriptor.designHeight)
        )
        guard let requirements = PanelRequirements(
            id: descriptor.id,
            title: descriptor.title,
            criticalLayerMask: layerMask,
            frameMin: size,
            frameMax: size,
            canonicalFrame: size
        ) else {
            throw BridgeIntegrationError.invalidRequirements(index: index)
        }
        let display = PanelDisplay(
            requirements: requirements,
            producer: composition.producer(panel: index),
            atlas: verified.glyphAtlas
        )
        let outcome = display.render(pixelWidth: 480, pixelHeight: 360, nowMs: 100)
        guard !outcome.showingFailure, outcome.reason == .ok else {
            throw BridgeIntegrationError.failedRender(
                index: index,
                reason: outcome.reason.rawValue
            )
        }
        guard outcome.image != nil else {
            throw BridgeIntegrationError.missingImage(index: index)
        }
        guard let report = outcome.report else {
            throw BridgeIntegrationError.missingReport(index: index)
        }
        guard report.satisfies(requirements) else {
            throw BridgeIntegrationError.unsatisfiedReport(index: index)
        }
    }
}
