import CoreGraphics
import CryptoKit
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

private final class BlockingRuntime: InstrumentRuntimeServing, @unchecked Sendable {
    private let lock = NSLock()
    private let compositionEntered = DispatchSemaphore(value: 0)
    private let compositionMayFinish = DispatchSemaphore(value: 0)
    private var outcome: InstrumentRuntimeCompositionOutcome
    private var mustBlock = false

    init(outcome: InstrumentRuntimeCompositionOutcome) {
        self.outcome = outcome
    }

    func blockNextComposition(with outcome: InstrumentRuntimeCompositionOutcome) {
        lock.lock()
        self.outcome = outcome
        mustBlock = true
        lock.unlock()
    }

    func waitUntilCompositionStarts() -> Bool {
        compositionEntered.wait(timeout: .now() + .seconds(5)) == .success
    }

    func releaseComposition() {
        compositionMayFinish.signal()
    }

    func writeState(_: [UInt8], acceptedAtMs _: UInt64) throws {}

    func compositionFrame(
        nowMs _: UInt64,
        pathHealthy _: Bool
    ) -> InstrumentRuntimeCompositionOutcome {
        lock.lock()
        let result = outcome
        let block = mustBlock
        mustBlock = false
        lock.unlock()
        if block {
            compositionEntered.signal()
            if compositionMayFinish.wait(timeout: .now() + .seconds(5)) != .success {
                Issue.record("The blocked runtime did not receive a release event")
            }
        }
        return result
    }
}

private final class ConcurrentResultProbe: @unchecked Sendable {
    private let lock = NSLock()
    private var frame: SceneFrame?
    private var reason: DisplayReason?
    private var compositionFailure: String?

    func store(frame: SceneFrame) {
        lock.lock()
        self.frame = frame
        lock.unlock()
    }

    func store(reason: DisplayReason) {
        lock.lock()
        self.reason = reason
        lock.unlock()
    }

    func store(compositionFailure: String) {
        lock.lock()
        self.compositionFailure = compositionFailure
        lock.unlock()
    }

    var value: (frame: SceneFrame?, reason: DisplayReason?, compositionFailure: String?) {
        lock.lock()
        defer { lock.unlock() }
        return (frame, reason, compositionFailure)
    }
}

private struct CompatibilityPin: Decodable {
    let stateAbiVersion: UInt32
    let sceneFormatVersion: UInt32
    let corpusVersion: UInt32
    let corpusDigest: String
    let registrySceneDigest: String
    let screenCompositionDigest: String
    let glyphRecordedHash: String
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
        screenCompositionDigest: AppleInstrumentCompatibilityGate.screenCompositionDigest,
        glyphRecordedHash: AppleInstrumentCompatibilityGate.glyphRecordedHash
    )
}

private func syntheticGlyphAtlas() throws -> PilotageGlyphAtlas {
    let canonical: [UInt8] = [
        1, 0, 1, 1, 1, 1, 1, 0,
        65, 0, 0, 0, 1, 1,
    ]
    let digest = Array(SHA256.hash(data: Data(canonical)))
    return try PilotageGlyphAtlas(
        asset: InstrumentGlyphAsset(canonical: canonical, recordedHash: digest),
        expectedHash: hex(digest)
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

private func runtimeOutcome(byte: UInt8, generation: UInt32) -> InstrumentRuntimeCompositionOutcome {
    InstrumentRuntimeCompositionOutcome(
        status: 0,
        scene: [byte],
        panels: [
            InstrumentRuntimePanelOutcome(
                panel: 0,
                status: 0,
                sceneOffset: 0,
                sceneLength: 1,
                frameWidth: 10,
                frameHeight: 10,
                generation: generation
            ),
        ],
        generation: generation
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
    #expect(AppleInstrumentCompatibilityGate.glyphRecordedHash == pin.glyphRecordedHash)
}

@Test("Each compatibility mismatch stops composition production")
func everyMismatchStopsBeforeProduction() throws {
    let expected = matchingIdentity()
    let cases: [(InstrumentCompatibilityField, InstrumentRuntimeIdentity)] = [
        (.stateABI, identity(expected, stateABI: expected.stateABI + 1)),
        (.sceneFormat, identity(expected, sceneFormat: expected.sceneFormat + 1)),
        (.corpusVersion, identity(expected, corpusVersion: expected.corpusVersion + 1)),
        (.corpusDigest, identity(expected, corpusDigest: "invalid")),
        (.registrySceneDigest, identity(expected, registrySceneDigest: "invalid")),
        (.screenCompositionDigest, identity(expected, screenCompositionDigest: "invalid")),
        (.glyphRecordedHash, identity(expected, glyphRecordedHash: "invalid")),
    ]
    let atlas = try syntheticGlyphAtlas()

    for (field, mismatched) in cases {
        let runtime = RuntimeDouble()
        var initializationCalls = 0
        do {
            _ = try AppleInstrumentCompatibilityGate.verify(mismatched, glyphAtlas: atlas) {
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
    let verified = try AppleInstrumentCompatibilityGate.verify(
        matchingIdentity(),
        glyphAtlas: syntheticGlyphAtlas()
    ) { runtime }
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
    let verified = try AppleInstrumentCompatibilityGate.verify(
        matchingIdentity(),
        glyphAtlas: syntheticGlyphAtlas()
    ) { runtime }
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
    let verified = try AppleInstrumentCompatibilityGate.verify(
        matchingIdentity(),
        glyphAtlas: syntheticGlyphAtlas()
    ) { runtime }
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

@Test("A producer cannot observe a composition update in progress")
func producerWaitsForCompleteComposition() throws {
    let runtime = BlockingRuntime(outcome: runtimeOutcome(byte: 1, generation: 1))
    let verified = try AppleInstrumentCompatibilityGate.verify(
        matchingIdentity(),
        glyphAtlas: syntheticGlyphAtlas()
    ) { runtime }
    let composition = PilotageInstrumentComposition(verifiedRuntime: verified)
    try composition.compose(nowMs: 1, pathHealthy: true)
    runtime.blockNextComposition(with: runtimeOutcome(byte: 2, generation: 2))

    let producer = composition.producer(panel: 0)
    let probe = ConcurrentResultProbe()
    let compositionFinished = DispatchSemaphore(value: 0)
    let frameStarted = DispatchSemaphore(value: 0)
    let frameFinished = DispatchSemaphore(value: 0)

    DispatchQueue.global().async {
        do {
            _ = try composition.compose(nowMs: 2, pathHealthy: true)
        } catch {
            probe.store(compositionFailure: String(describing: error))
        }
        compositionFinished.signal()
    }
    #expect(runtime.waitUntilCompositionStarts())

    DispatchQueue.global().async {
        frameStarted.signal()
        do {
            probe.store(frame: try producer.frame(
                designFrame: CGRect(x: 0, y: 0, width: 10, height: 10)
            ))
        } catch let fault as ProducerFault {
            probe.store(reason: fault.reason)
        } catch {
            probe.store(reason: .renderTrap)
        }
        frameFinished.signal()
    }
    #expect(frameStarted.wait(timeout: .now() + .seconds(5)) == .success)
    let frameFinishedDuringUpdate = frameFinished.wait(
        timeout: .now() + .milliseconds(20)
    ) == .success

    runtime.releaseComposition()
    #expect(compositionFinished.wait(timeout: .now() + .seconds(5)) == .success)
    if !frameFinishedDuringUpdate {
        #expect(frameFinished.wait(timeout: .now() + .seconds(5)) == .success)
    }

    let result = probe.value
    #expect(!frameFinishedDuringUpdate)
    #expect(result.compositionFailure == nil)
    #expect(result.reason == nil)
    #expect(result.frame?.bytes == [2])
    #expect(result.frame?.generation == 2)
}

@Test("A failed transaction removes the previous composition")
func failedTransactionRemovesPreviousComposition() throws {
    let runtime = RuntimeDouble()
    let verified = try AppleInstrumentCompatibilityGate.verify(
        matchingIdentity(),
        glyphAtlas: syntheticGlyphAtlas()
    ) { runtime }
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
    let verified = try AppleInstrumentCompatibilityGate.verify(
        matchingIdentity(),
        glyphAtlas: syntheticGlyphAtlas()
    ) { runtime }
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
    screenCompositionDigest: String? = nil,
    glyphRecordedHash: String? = nil
) -> InstrumentRuntimeIdentity {
    InstrumentRuntimeIdentity(
        stateABI: stateABI ?? base.stateABI,
        sceneFormat: sceneFormat ?? base.sceneFormat,
        corpusVersion: corpusVersion ?? base.corpusVersion,
        corpusDigest: corpusDigest ?? base.corpusDigest,
        registrySceneDigest: registrySceneDigest ?? base.registrySceneDigest,
        screenCompositionDigest: screenCompositionDigest ?? base.screenCompositionDigest,
        glyphRecordedHash: glyphRecordedHash ?? base.glyphRecordedHash
    )
}
