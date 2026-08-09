import CoreGraphics
import Foundation
import IndicateAppleDisplay
import Testing
@testable import PilotageAppleInstrumentConsumer

private final class RuntimeDouble: InstrumentRuntimeServing {
    var compositionCalls = 0
    var writes: [(bytes: [UInt8], acceptedAtMs: UInt64)] = []
    var outcome = InstrumentRuntimeCompositionOutcome(
        status: 0,
        scene: [1],
        panels: [
            InstrumentRuntimePanelOutcome(
                panel: 0,
                status: 0,
                sceneOffset: 0,
                sceneLength: 1,
                frameWidth: 10,
                frameHeight: 10,
                generation: 1
            ),
        ],
        generation: 1
    )

    func writeState(_ bytes: [UInt8], acceptedAtMs: UInt64) throws {
        writes.append((bytes, acceptedAtMs))
    }

    func compositionFrame(
        nowMs _: UInt64,
        pathHealthy _: Bool
    ) -> InstrumentRuntimeCompositionOutcome {
        compositionCalls += 1
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

private func requirements(width: CGFloat = 10) -> PanelRequirements {
    PanelRequirements(
        id: "test",
        title: "Test",
        criticalLayers: [],
        frameMin: CGSize(width: width, height: 10),
        frameMax: CGSize(width: width, height: 10),
        canonicalFrame: CGSize(width: width, height: 10)
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

@Test("Each compatibility mismatch stops composition production")
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
        #expect(runtime.compositionCalls == 0)
    }
}

@Test("A verified composition can paint through IndicateAppleDisplay")
func verifiedCompositionPaints() throws {
    let runtime = RuntimeDouble()
    let verified = try AppleInstrumentCompatibilityGate.verify(matchingIdentity()) { runtime }
    let composition = PilotageInstrumentComposition(verifiedRuntime: verified)
    try composition.compose(nowMs: 100, pathHealthy: true)
    let display = PanelDisplay(
        requirements: requirements(),
        producer: composition.producer(panel: 0)
    )

    let outcome = display.render(pixelWidth: 10, pixelHeight: 10, nowMs: 100)
    #expect(!outcome.showingFailure)
    #expect(runtime.compositionCalls == 1)
}

@Test("One runtime call supplies every panel producer")
func oneRuntimeCallSuppliesEveryPanel() throws {
    let runtime = RuntimeDouble()
    runtime.outcome = InstrumentRuntimeCompositionOutcome(
        status: 0,
        scene: [1, 1],
        panels: [
            InstrumentRuntimePanelOutcome(
                panel: 0, status: 0, sceneOffset: 0, sceneLength: 1,
                frameWidth: 10, frameHeight: 10, generation: 7
            ),
            InstrumentRuntimePanelOutcome(
                panel: 1, status: 0, sceneOffset: 1, sceneLength: 1,
                frameWidth: 20, frameHeight: 10, generation: 7
            ),
        ],
        generation: 7
    )
    let verified = try AppleInstrumentCompatibilityGate.verify(matchingIdentity()) { runtime }
    let composition = PilotageInstrumentComposition(verifiedRuntime: verified)

    #expect(try composition.compose(nowMs: 500, pathHealthy: true) == 7)
    let first = try composition.producer(panel: 0).frame(
        designFrame: CGRect(x: 0, y: 0, width: 10, height: 10)
    )
    let second = try composition.producer(panel: 1).frame(
        designFrame: CGRect(x: 0, y: 0, width: 20, height: 10)
    )
    #expect(first.bytes == [1])
    #expect(second.bytes == [1])
    #expect(runtime.compositionCalls == 1)
}

@Test("State acceptance time reaches the runtime and clears cached scenes")
func stateAcceptanceInvalidatesCachedScenes() throws {
    let runtime = RuntimeDouble()
    let verified = try AppleInstrumentCompatibilityGate.verify(matchingIdentity()) { runtime }
    let composition = PilotageInstrumentComposition(verifiedRuntime: verified)
    try composition.compose(nowMs: 100, pathHealthy: true)

    try composition.writeState([7, 0], acceptedAtMs: 250)
    #expect(runtime.writes.count == 1)
    #expect(runtime.writes[0].bytes == [7, 0])
    #expect(runtime.writes[0].acceptedAtMs == 250)
    do {
        _ = try composition.producer(panel: 0).frame(
            designFrame: CGRect(x: 0, y: 0, width: 10, height: 10)
        )
        Issue.record("A state write left the previous scene available")
    } catch let fault as ProducerFault {
        #expect(fault.reason == .notInitialized)
    } catch {
        Issue.record("The producer returned an untyped error: \(error)")
    }
}

@Test("A failed transaction removes the previous composition")
func failedTransactionRemovesPreviousComposition() throws {
    let runtime = RuntimeDouble()
    let verified = try AppleInstrumentCompatibilityGate.verify(matchingIdentity()) { runtime }
    let composition = PilotageInstrumentComposition(verifiedRuntime: verified)
    try composition.compose(nowMs: 100, pathHealthy: true)
    runtime.outcome = InstrumentRuntimeCompositionOutcome(
        status: 11,
        scene: [],
        panels: [],
        generation: 1
    )

    do {
        try composition.compose(nowMs: 200, pathHealthy: true)
        Issue.record("The composition accepted a failed transaction")
    } catch let fault as ProducerFault {
        #expect(fault.reason == .stateMalformed)
    } catch {
        Issue.record("The composition returned an untyped error: \(error)")
    }
    do {
        _ = try composition.producer(panel: 0).frame(
            designFrame: CGRect(x: 0, y: 0, width: 10, height: 10)
        )
        Issue.record("The failed transaction left an old scene available")
    } catch let fault as ProducerFault {
        #expect(fault.reason == .notInitialized)
    } catch {
        Issue.record("The producer returned an untyped error: \(error)")
    }
}

@Test("A panel frame mismatch fails before paint")
func frameMismatchFailsClosed() throws {
    let runtime = RuntimeDouble()
    runtime.outcome = InstrumentRuntimeCompositionOutcome(
        status: 0,
        scene: [1],
        panels: [
            InstrumentRuntimePanelOutcome(
                panel: 0, status: 0, sceneOffset: 0, sceneLength: 1,
                frameWidth: 11, frameHeight: 10, generation: 1
            ),
        ],
        generation: 1
    )
    let verified = try AppleInstrumentCompatibilityGate.verify(matchingIdentity()) { runtime }
    let composition = PilotageInstrumentComposition(verifiedRuntime: verified)
    try composition.compose(nowMs: 0, pathHealthy: true)
    let display = PanelDisplay(
        requirements: requirements(),
        producer: composition.producer(panel: 0)
    )

    let outcome = display.render(pixelWidth: 10, pixelHeight: 10, nowMs: 0)
    #expect(outcome.showingFailure)
    #expect(outcome.reason == .abiMismatch)
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
