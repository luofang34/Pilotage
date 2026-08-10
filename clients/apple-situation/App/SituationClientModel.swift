import Foundation
import PilotageRadioSource
import PilotageSituationCore
import SwiftUI

@MainActor
final class SituationClientModel: ObservableObject {
    @Published private(set) var display: DisplayBatch?
    @Published private(set) var errorMessage: String?
    @Published private(set) var radioSource: RadioSourceSnapshot
    @Published private(set) var selectedTraffic: DisplayTrafficDetail?

    private let session: PresentationSession
    private let domain: RadioDomainSession?
    private let terrainAvailable: Bool
    private let discovery = AeroLinkDiscoveryGate()
    private let evidenceWriter = SituationEvidenceWriter()
    private var runtime: AeroLinkRadioRuntime?
    private var maintenanceTask: Task<Void, Never>?
    private var drainTask: Task<Void, Never>?
    private var cleanupTask: (generation: UInt64, task: Task<Void, Never>)?
    private var cleanupGeneration: UInt64 = 0
    private var startupError: String?
    private var isActive = false

    init() {
        let source = RadioSourceSnapshot(
            availability: .suspended,
            receivers: [],
            bandFailures: []
        )
        let session = PresentationSession()
        self.session = session
        radioSource = source
        terrainAvailable = Bundle.main.url(
            forResource: "SituationTerrain",
            withExtension: "mbtiles"
        ) != nil
        var createdDomain: RadioDomainSession?
        var initialDisplay: DisplayBatch?
        do {
            initialDisplay = try session.observeSources(
                observation: PresentationSourceObservation(
                    source: source,
                    terrainAvailable: terrainAvailable
                ),
                nowMicros: Self.monotonicMicros
            )
            createdDomain = try RadioDomainSession()
        } catch {
            startupError = Self.join(startupError, error.localizedDescription)
        }
        domain = createdDomain
        display = initialDisplay
        errorMessage = startupError
    }

    func activate() async {
        recordEvidence()
        if let cleanup = cleanupTask {
            await cleanup.task.value
            if cleanupTask?.generation == cleanup.generation {
                cleanupTask = nil
            }
        }
        guard !Task.isCancelled, !isActive, let domain else { return }
        isActive = true
        let runtime: AeroLinkRadioRuntime
        if let current = self.runtime {
            runtime = current
        } else {
            runtime = AeroLinkRadioRuntime(
                session: session,
                domain: domain,
                discovery: discovery
            ) { [weak self] emission in
                await self?.apply(emission)
            }
            self.runtime = runtime
        }
        await runtime.activate()
        guard !Task.isCancelled, isActive else { return }
        maintenanceTask = Task.detached(priority: .utility) {
            await runtime.maintenanceLoop()
        }
        drainTask = Task.detached(priority: .userInitiated) {
            await runtime.drainLoop()
        }
    }

    func suspend() async {
        guard isActive, let runtime else { return }
        isActive = false
        let maintenance = maintenanceTask
        let drain = drainTask
        maintenance?.cancel()
        drain?.cancel()
        maintenanceTask = nil
        drainTask = nil
        cleanupGeneration &+= 1
        let generation = cleanupGeneration
        let cleanup = Task.detached(priority: .utility) {
            await runtime.suspend()
            if let maintenance {
                await maintenance.value
            }
            if let drain {
                await drain.value
            }
        }
        cleanupTask = (generation, cleanup)
        await cleanup.value
        if cleanupTask?.generation == generation {
            cleanupTask = nil
        }
    }

    private func apply(_ emission: RadioRuntimeEmission) {
        radioSource = emission.source
        var presentationError: String?
        do {
            let next = try session.observeSources(
                observation: PresentationSourceObservation(
                    source: emission.source,
                    terrainAvailable: terrainAvailable
                ),
                nowMicros: Self.monotonicMicros
            )
            applyDisplay(next)
        } catch {
            presentationError = error.localizedDescription
        }
        errorMessage = Self.join(
            startupError,
            Self.join(emission.errorMessage, presentationError)
        )
        // A run that never produces a batch still has to say why: a receiver state and a
        // disabled driver extension are the answer, and both change without a batch.
        recordEvidence()
    }

    func setLayerEnabled(id: String, enabled: Bool) {
        do {
            applyDisplay(try session.setLayerEnabled(layerId: id, enabled: enabled))
        } catch {
            errorMessage = Self.join(startupError, error.localizedDescription)
        }
    }

    func selectTraffic(id: String) {
        selectedTraffic = display?.trafficDetails.first { $0.id == id }
    }

    func clearTrafficSelection() {
        selectedTraffic = nil
    }

    private func applyDisplay(_ next: DisplayBatch) {
        display = next
        if let selected = selectedTraffic {
            selectedTraffic = next.trafficDetails.first { $0.id == selected.id }
        }
        recordEvidence()
    }

    private func recordEvidence() {
        evidenceWriter.record(
            SituationEvidence(
                batch: display,
                radioSource: radioSource,
                driverEnabled: discovery.driverIsEnabled(),
                terrainArchiveAvailable: terrainAvailable,
                errorMessage: errorMessage
            )
        )
    }

    private static func join(_ first: String?, _ second: String?) -> String? {
        switch (first, second) {
        case (.none, .none): nil
        case (.some(let value), .none), (.none, .some(let value)): value
        case (.some(let first), .some(let second)): "\(first)\n\(second)"
        }
    }

    private static var monotonicMicros: UInt64 {
        DispatchTime.now().uptimeNanoseconds / 1_000
    }
}
