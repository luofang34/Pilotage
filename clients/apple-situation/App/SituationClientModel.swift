import PilotageRadioSource
import PilotageSituationCore
import SwiftUI

@MainActor
final class SituationClientModel: ObservableObject {
    @Published private(set) var display: DisplayBatch?
    @Published private(set) var errorMessage: String?
    @Published private(set) var radioSource = RadioSourceSnapshot(
        availability: .suspended,
        receivers: [],
        bandFailures: []
    )

    private let session: PresentationSession
    private let domain: RadioDomainSession?
    private let discovery = AeroLinkDiscoveryGate()
    private var runtime: AeroLinkRadioRuntime?
    private var maintenanceTask: Task<Void, Never>?
    private var drainTask: Task<Void, Never>?
    private var startupError: String?
    private var isActive = false

    init() {
        let session = PresentationSession()
        self.session = session
        var createdDomain: RadioDomainSession?
        var initialDisplay: DisplayBatch?
        do {
            initialDisplay = try session.currentDisplay()
            createdDomain = try RadioDomainSession()
        } catch {
            startupError = Self.join(startupError, error.localizedDescription)
        }
        domain = createdDomain
        display = initialDisplay
        errorMessage = startupError
    }

    func activate() async {
        guard !isActive, let domain else { return }
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
        await Task.detached(priority: .utility) {
            await runtime.suspend()
        }.value
        if let maintenance {
            await maintenance.value
        }
        if let drain {
            await drain.value
        }
    }

    private func apply(_ emission: RadioRuntimeEmission) {
        radioSource = emission.source
        if let display = emission.display {
            self.display = display
        }
        errorMessage = Self.join(startupError, emission.errorMessage)
    }

    private static func join(_ first: String?, _ second: String?) -> String? {
        switch (first, second) {
        case (.none, .none): nil
        case (.some(let value), .none), (.none, .some(let value)): value
        case (.some(let first), .some(let second)): "\(first)\n\(second)"
        }
    }
}
