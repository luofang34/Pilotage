import CoreGraphics
import Foundation
import IndicateAppleDisplay

/// One committed composition frame that all panel producers share.
public final class PilotageInstrumentComposition: @unchecked Sendable {
    private let verifiedRuntime: VerifiedInstrumentRuntime
    private let lock = NSLock()
    private var panels: [UInt32: (scene: [UInt8], outcome: InstrumentRuntimePanelOutcome)] = [:]

    public init(verifiedRuntime: VerifiedInstrumentRuntime) {
        self.verifiedRuntime = verifiedRuntime
    }

    /// Accepts one state frame and invalidates the current scenes.
    public func writeState(_ bytes: [UInt8], acceptedAtMs: UInt64) throws {
        lock.lock()
        defer { lock.unlock() }
        try writeStateLocked(bytes, acceptedAtMs: acceptedAtMs)
    }

    private func writeStateLocked(_ bytes: [UInt8], acceptedAtMs: UInt64) throws {
        panels.removeAll(keepingCapacity: true)
        try verifiedRuntime.writeState(bytes, acceptedAtMs: acceptedAtMs)
    }

    /// Accepts one state frame and commits its composition in one lock
    /// scope, so a renderer on another thread never observes the window
    /// between an invalidated scene and its replacement. The two-step
    /// `writeState` + `compose` pair leaves that window open by
    /// construction; a display host paced by its own display link must
    /// use this call instead.
    @discardableResult
    public func ingest(
        _ bytes: [UInt8],
        acceptedAtMs: UInt64,
        pathHealthy: Bool
    ) throws -> UInt32 {
        lock.lock()
        defer { lock.unlock() }
        try writeStateLocked(bytes, acceptedAtMs: acceptedAtMs)
        return try composeLocked(nowMs: acceptedAtMs, pathHealthy: pathHealthy)
    }

    /// Produces and commits all composition panels with one runtime call.
    @discardableResult
    public func compose(nowMs: UInt64, pathHealthy: Bool) throws -> UInt32 {
        lock.lock()
        defer { lock.unlock() }
        return try composeLocked(nowMs: nowMs, pathHealthy: pathHealthy)
    }

    private func composeLocked(nowMs: UInt64, pathHealthy: Bool) throws -> UInt32 {
        panels.removeAll(keepingCapacity: true)
        let frame = verifiedRuntime.compositionFrame(
            nowMs: nowMs,
            pathHealthy: pathHealthy
        )
        try requireSuccess(frame.status)
        try requireSuccess(frame.alertStatus)

        var next: [UInt32: (scene: [UInt8], outcome: InstrumentRuntimePanelOutcome)] = [:]
        next.reserveCapacity(frame.panels.count)
        for outcome in frame.panels {
            try requireSuccess(outcome.status)
            let scene = try panelScene(outcome, in: frame.scene)
            guard next.updateValue((scene, outcome), forKey: outcome.panel) == nil else {
                throw ProducerFault(reason: .abiMismatch)
            }
        }
        guard !next.isEmpty else {
            throw ProducerFault(reason: .sceneFraming)
        }
        panels = next
        return frame.generation
    }

    /// Creates one panel producer over the shared committed frame.
    public func producer(panel: UInt32) -> PilotageInstrumentSceneProducer {
        PilotageInstrumentSceneProducer(composition: self, panel: panel)
    }

    fileprivate func frame(panel: UInt32, designFrame: CGRect) throws -> SceneFrame {
        lock.lock()
        defer { lock.unlock() }
        guard let committed = panels[panel] else {
            throw ProducerFault(reason: .notInitialized)
        }
        guard committed.outcome.frameWidth == Float(designFrame.width),
              committed.outcome.frameHeight == Float(designFrame.height) else {
            throw ProducerFault(reason: .abiMismatch)
        }
        return SceneFrame(
            bytes: committed.scene,
            generation: committed.outcome.generation
        )
    }
}

/// A panel producer over one committed composition frame.
public final class PilotageInstrumentSceneProducer: SceneProducing {
    private let composition: PilotageInstrumentComposition
    private let panel: UInt32

    fileprivate init(composition: PilotageInstrumentComposition, panel: UInt32) {
        self.composition = composition
        self.panel = panel
    }

    public func frame(designFrame: CGRect) throws -> SceneFrame {
        try composition.frame(panel: panel, designFrame: designFrame)
    }
}

private func requireSuccess(_ status: UInt32) throws {
    guard status == 0 else {
        let code = UInt16(exactly: status)
        let reason = code.flatMap(DisplayReason.init(rawValue:)) ?? .renderTrap
        throw ProducerFault(reason: reason == .ok ? .renderTrap : reason)
    }
}

private func panelScene(
    _ panel: InstrumentRuntimePanelOutcome,
    in scene: [UInt8]
) throws -> [UInt8] {
    guard let start = Int(exactly: panel.sceneOffset),
          let count = Int(exactly: panel.sceneLength),
          count > 0,
          start <= scene.count,
          count <= scene.count - start else {
        throw ProducerFault(reason: .sceneFraming)
    }
    return Array(scene[start ..< start + count])
}
