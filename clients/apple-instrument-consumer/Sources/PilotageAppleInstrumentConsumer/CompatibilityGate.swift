import IndicateAppleDisplay

/// One value in the instrument compatibility identity.
public enum InstrumentCompatibilityField: String, Sendable {
    case stateABI
    case sceneFormat
    case corpusVersion
    case corpusDigest
    case registrySceneDigest
    case screenCompositionDigest
    case glyphRecordedHash
}

/// A compatibility refusal that occurs before scene production.
public enum InstrumentCompatibilityError: Error, Equatable, Sendable {
    case mismatch(
        field: InstrumentCompatibilityField,
        actual: String,
        expected: String
    )
}

/// The identity exported by one linked Pilotage instrument runtime.
public struct InstrumentRuntimeIdentity: Equatable, Sendable {
    public let stateABI: UInt32
    public let sceneFormat: UInt32
    public let corpusVersion: UInt32
    public let corpusDigest: String
    public let registrySceneDigest: String
    public let screenCompositionDigest: String
    public let glyphRecordedHash: String

    public init(
        stateABI: UInt32,
        sceneFormat: UInt32,
        corpusVersion: UInt32,
        corpusDigest: String,
        registrySceneDigest: String,
        screenCompositionDigest: String,
        glyphRecordedHash: String
    ) {
        self.stateABI = stateABI
        self.sceneFormat = sceneFormat
        self.corpusVersion = corpusVersion
        self.corpusDigest = corpusDigest
        self.registrySceneDigest = registrySceneDigest
        self.screenCompositionDigest = screenCompositionDigest
        self.glyphRecordedHash = glyphRecordedHash
    }
}

/// One panel result in a runtime composition transaction.
public struct InstrumentRuntimePanelOutcome: Equatable, Sendable {
    public let panel: UInt32
    public let status: UInt32
    public let sceneOffset: UInt32
    public let sceneLength: UInt32
    public let frameWidth: Float
    public let frameHeight: Float
    public let generation: UInt32

    public init(
        panel: UInt32,
        status: UInt32,
        sceneOffset: UInt32,
        sceneLength: UInt32,
        frameWidth: Float,
        frameHeight: Float,
        generation: UInt32
    ) {
        self.panel = panel
        self.status = status
        self.sceneOffset = sceneOffset
        self.sceneLength = sceneLength
        self.frameWidth = frameWidth
        self.frameHeight = frameHeight
        self.generation = generation
    }
}

/// The scenes and status from one complete composition transaction.
public struct InstrumentRuntimeCompositionOutcome: Equatable, Sendable {
    public let status: UInt32
    public let scene: [UInt8]
    public let panels: [InstrumentRuntimePanelOutcome]
    public let generation: UInt32
    public let alertStatus: UInt32
    public let activeAlertCount: UInt32
    public let alertPathFaulted: Bool
    public let alertOverflow: Bool
    public let alertManagerGeneration: UInt32

    public init(
        status: UInt32,
        scene: [UInt8],
        panels: [InstrumentRuntimePanelOutcome],
        generation: UInt32,
        alertStatus: UInt32 = 0,
        activeAlertCount: UInt32 = 0,
        alertPathFaulted: Bool = false,
        alertOverflow: Bool = false,
        alertManagerGeneration: UInt32 = 0
    ) {
        self.status = status
        self.scene = scene
        self.panels = panels
        self.generation = generation
        self.alertStatus = alertStatus
        self.activeAlertCount = activeAlertCount
        self.alertPathFaulted = alertPathFaulted
        self.alertOverflow = alertOverflow
        self.alertManagerGeneration = alertManagerGeneration
    }
}

/// The narrow runtime surface that the Apple shell consumes.
public protocol InstrumentRuntimeServing: AnyObject {
    func writeState(_ bytes: [UInt8], acceptedAtMs: UInt64) throws
    func compositionFrame(nowMs: UInt64, pathHealthy: Bool) -> InstrumentRuntimeCompositionOutcome
}

/// A state write that the generated bridge refused.
public enum InstrumentStateWriteError: Error, Equatable, Sendable {
    case frameTooLarge(actual: UInt64, capacity: UInt64)
    case unexpectedStatus(UInt32)
}

/// A runtime that passed the complete compatibility gate.
public final class VerifiedInstrumentRuntime {
    let runtime: any InstrumentRuntimeServing
    public let glyphAtlas: PilotageGlyphAtlas

    fileprivate init(runtime: any InstrumentRuntimeServing, glyphAtlas: PilotageGlyphAtlas) {
        self.runtime = runtime
        self.glyphAtlas = glyphAtlas
    }

    public func writeState(_ bytes: [UInt8], acceptedAtMs: UInt64) throws {
        try runtime.writeState(bytes, acceptedAtMs: acceptedAtMs)
    }

    public func compositionFrame(
        nowMs: UInt64,
        pathHealthy: Bool
    ) -> InstrumentRuntimeCompositionOutcome {
        runtime.compositionFrame(nowMs: nowMs, pathHealthy: pathHealthy)
    }
}

/// Verifies the runtime and Apple display identities before runtime creation.
public enum AppleInstrumentCompatibilityGate {
    public static let stateABI: UInt32 = 7
    public static let registrySceneDigest =
        "f82d905643b48822de25665761ad3e29daa334d937f18b1e98a3e215353cb704"
    public static let screenCompositionDigest =
        "6761e8e1ed137e682530274c8f02353d2ab40e7142a36cd4321a6835323b463c"
    public static let glyphRecordedHash =
        "281eef6229feee417c7090d8c8ea79489c017cd1c02fc7234876b2a64a532158"

    public static func verify(
        _ actual: InstrumentRuntimeIdentity,
        glyphAtlas: PilotageGlyphAtlas,
        makeRuntime: () throws -> any InstrumentRuntimeServing
    ) throws -> VerifiedInstrumentRuntime {
        let expected = InstrumentRuntimeIdentity(
            stateABI: stateABI,
            sceneFormat: UInt32(SceneBackend.formatVersion),
            corpusVersion: UInt32(SceneBackend.conformanceCorpusVersion),
            corpusDigest: SceneBackend.conformanceCorpusDigest,
            registrySceneDigest: registrySceneDigest,
            screenCompositionDigest: screenCompositionDigest,
            glyphRecordedHash: glyphRecordedHash
        )
        let checks: [(InstrumentCompatibilityField, String, String)] = [
            (.stateABI, String(actual.stateABI), String(expected.stateABI)),
            (.sceneFormat, String(actual.sceneFormat), String(expected.sceneFormat)),
            (.corpusVersion, String(actual.corpusVersion), String(expected.corpusVersion)),
            (.corpusDigest, actual.corpusDigest, expected.corpusDigest),
            (.registrySceneDigest, actual.registrySceneDigest, expected.registrySceneDigest),
            (
                .screenCompositionDigest,
                actual.screenCompositionDigest,
                expected.screenCompositionDigest
            ),
            (.glyphRecordedHash, actual.glyphRecordedHash, expected.glyphRecordedHash),
        ]
        for (field, value, required) in checks where value != required {
            throw InstrumentCompatibilityError.mismatch(
                field: field,
                actual: value,
                expected: required
            )
        }
        return VerifiedInstrumentRuntime(runtime: try makeRuntime(), glyphAtlas: glyphAtlas)
    }
}
