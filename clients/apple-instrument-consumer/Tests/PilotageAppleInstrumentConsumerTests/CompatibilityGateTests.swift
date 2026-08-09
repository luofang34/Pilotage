import CoreGraphics
import Foundation
import IndicateAppleDisplay
import Testing
@testable import PilotageAppleInstrumentConsumer

private final class RuntimeDouble: InstrumentRuntimeServing {
    var renderCalls = 0
    var outcome = InstrumentRuntimeRenderOutcome(
        status: 0,
        scene: [1],
        frameWidth: 10,
        frameHeight: 10,
        generation: 1
    )

    func writeState(_: [UInt8]) throws {}

    func render(panel _: UInt32) -> InstrumentRuntimeRenderOutcome {
        renderCalls += 1
        return outcome
    }
}

private struct CompatibilityPin: Decodable {
    let stateAbiVersion: UInt32
    let sceneFormatVersion: UInt32
    let corpusVersion: UInt32
    let corpusDigest: String
    let registrySceneDigest: String
    let screenCompositionDigest: String
}

private func compatibilityPin() throws -> CompatibilityPin {
    let url = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .appendingPathComponent("instrument-compatibility.json")
    return try JSONDecoder().decode(CompatibilityPin.self, from: Data(contentsOf: url))
}

private func matchingIdentity() -> InstrumentRuntimeIdentity {
    InstrumentRuntimeIdentity(
        stateABI: AppleInstrumentCompatibilityGate.stateABI,
        sceneFormat: UInt32(SceneBackend.formatVersion),
        corpusVersion: UInt32(SceneBackend.conformanceCorpusVersion),
        corpusDigest: SceneBackend.conformanceCorpusDigest,
        registrySceneDigest: AppleInstrumentCompatibilityGate.registrySceneDigest,
        screenCompositionDigest: AppleInstrumentCompatibilityGate.screenCompositionDigest
    )
}

@Test("The Swift gate equals the Pilotage consumer pin")
func swiftGateMatchesConsumerPin() throws {
    let pin = try compatibilityPin()
    #expect(AppleInstrumentCompatibilityGate.stateABI == pin.stateAbiVersion)
    #expect(UInt32(SceneBackend.formatVersion) == pin.sceneFormatVersion)
    #expect(UInt32(SceneBackend.conformanceCorpusVersion) == pin.corpusVersion)
    #expect(SceneBackend.conformanceCorpusDigest == pin.corpusDigest)
    #expect(AppleInstrumentCompatibilityGate.registrySceneDigest == pin.registrySceneDigest)
    #expect(
        AppleInstrumentCompatibilityGate.screenCompositionDigest ==
            pin.screenCompositionDigest
    )
}

@Test("Each compatibility mismatch stops scene production")
func everyMismatchStopsBeforeProduction() {
    let expected = matchingIdentity()
    let cases: [(InstrumentCompatibilityField, InstrumentRuntimeIdentity)] = [
        (.stateABI, identity(expected, stateABI: expected.stateABI + 1)),
        (.sceneFormat, identity(expected, sceneFormat: expected.sceneFormat + 1)),
        (.corpusVersion, identity(expected, corpusVersion: expected.corpusVersion + 1)),
        (.corpusDigest, identity(expected, corpusDigest: "invalid")),
        (.registrySceneDigest, identity(expected, registrySceneDigest: "invalid")),
        (.screenCompositionDigest, identity(expected, screenCompositionDigest: "invalid")),
    ]

    for (field, mismatched) in cases {
        let runtime = RuntimeDouble()
        var initializationCalls = 0
        do {
            _ = try AppleInstrumentCompatibilityGate.verify(mismatched) {
                initializationCalls += 1
                return runtime
            }
            Issue.record("The gate accepted a mismatch in \(field.rawValue)")
        } catch let error as InstrumentCompatibilityError {
            guard case let .mismatch(actualField, _, _) = error else {
                Issue.record("The gate returned an unexpected error")
                continue
            }
            #expect(actualField == field)
        } catch {
            Issue.record("The gate returned an untyped error: \(error)")
        }
        #expect(initializationCalls == 0)
        #expect(runtime.renderCalls == 0)
    }
}

@Test("A verified runtime can paint through IndicateAppleDisplay")
func verifiedRuntimePaints() throws {
    let runtime = RuntimeDouble()
    let verified = try AppleInstrumentCompatibilityGate.verify(matchingIdentity()) { runtime }
    let producer = PilotageInstrumentSceneProducer(verifiedRuntime: verified, panel: 0)
    let requirements = PanelRequirements(
        id: "test",
        title: "Test",
        criticalLayers: [],
        frameMin: CGSize(width: 10, height: 10),
        frameMax: CGSize(width: 10, height: 10),
        canonicalFrame: CGSize(width: 10, height: 10)
    )
    let display = PanelDisplay(requirements: requirements, producer: producer)
    let outcome = display.render(pixelWidth: 10, pixelHeight: 10, nowMs: 0)
    #expect(!outcome.showingFailure)
    #expect(runtime.renderCalls == 1)
}

@Test("A frame mismatch is covered before paint")
func frameMismatchFailsClosed() throws {
    let runtime = RuntimeDouble()
    runtime.outcome = InstrumentRuntimeRenderOutcome(
        status: 0,
        scene: [1],
        frameWidth: 11,
        frameHeight: 10,
        generation: 1
    )
    let verified = try AppleInstrumentCompatibilityGate.verify(matchingIdentity()) { runtime }
    let producer = PilotageInstrumentSceneProducer(verifiedRuntime: verified, panel: 0)
    let requirements = PanelRequirements(
        id: "test",
        title: "Test",
        criticalLayers: [],
        frameMin: CGSize(width: 10, height: 10),
        frameMax: CGSize(width: 10, height: 10),
        canonicalFrame: CGSize(width: 10, height: 10)
    )
    let display = PanelDisplay(requirements: requirements, producer: producer)
    let outcome = display.render(pixelWidth: 10, pixelHeight: 10, nowMs: 0)
    #expect(outcome.showingFailure)
    #expect(outcome.reason == .abiMismatch)
}

@Test("Producer status codes remain typed through the Apple display")
func producerStatusCodesRemainTyped() throws {
    for (status, reason) in [
        (UInt32(11), DisplayReason.stateMalformed),
        (UInt32(12), DisplayReason.configInvalid),
    ] {
        let runtime = RuntimeDouble()
        runtime.outcome = InstrumentRuntimeRenderOutcome(
            status: status,
            scene: [],
            frameWidth: 10,
            frameHeight: 10,
            generation: 0
        )
        let verified = try AppleInstrumentCompatibilityGate.verify(matchingIdentity()) { runtime }
        let producer = PilotageInstrumentSceneProducer(verifiedRuntime: verified, panel: 0)
        do {
            _ = try producer.frame(designFrame: CGRect(x: 0, y: 0, width: 10, height: 10))
            Issue.record("The producer accepted status \(status)")
        } catch let fault as ProducerFault {
            #expect(fault.reason == reason)
        } catch {
            Issue.record("The producer returned an untyped error: \(error)")
        }
    }
}

private func identity(
    _ base: InstrumentRuntimeIdentity,
    stateABI: UInt32? = nil,
    sceneFormat: UInt32? = nil,
    corpusVersion: UInt32? = nil,
    corpusDigest: String? = nil,
    registrySceneDigest: String? = nil,
    screenCompositionDigest: String? = nil
) -> InstrumentRuntimeIdentity {
    InstrumentRuntimeIdentity(
        stateABI: stateABI ?? base.stateABI,
        sceneFormat: sceneFormat ?? base.sceneFormat,
        corpusVersion: corpusVersion ?? base.corpusVersion,
        corpusDigest: corpusDigest ?? base.corpusDigest,
        registrySceneDigest: registrySceneDigest ?? base.registrySceneDigest,
        screenCompositionDigest: screenCompositionDigest ?? base.screenCompositionDigest
    )
}
