import Dispatch
import PilotageRadioSource
import PilotageSituationCore

struct RadioRuntimeEmission: @unchecked Sendable {
    let source: RadioSourceSnapshot
    let display: DisplayBatch?
    let errorMessage: String?
}

struct AeroLinkMaintenanceView: Sendable {
    let active: Bool
    let cycle: UInt64
    let reconnectRequested: Bool
    let connections: [AeroLinkConnectionHandle]
}

struct AeroLinkDrainConnection: Sendable {
    let handle: AeroLinkConnectionHandle
    let reconnectGeneration: UInt64
}

struct AeroLinkDrainView: Sendable {
    let active: Bool
    let cycle: UInt64
    let connections: [AeroLinkDrainConnection]
}

private struct LiveAeroLinkConnection: Sendable {
    let handle: AeroLinkConnectionHandle
    var availability: RadioAvailability
    var diagnostics: RadioDiagnostics
}

actor AeroLinkRadioState {
    private let session: PresentationSession
    private let domain: RadioDomainSession
    private let publish: @Sendable (RadioRuntimeEmission) async -> Void
    private var active = false
    private var cycle: UInt64 = 0
    private var connections: [UInt32: LiveAeroLinkConnection] = [:]
    private var retired: [AeroLinkConnectionHandle] = []
    private var degraded = RadioDegradedState()
    private var idleAvailability: RadioAvailability = .unplugged
    private var reconnectRequested = false
    private var lastError: String?

    init(
        session: PresentationSession,
        domain: RadioDomainSession,
        publish: @escaping @Sendable (RadioRuntimeEmission) async -> Void
    ) {
        self.session = session
        self.domain = domain
        self.publish = publish
    }

    func activate() async {
        cycle &+= 1
        active = true
        idleAvailability = .unplugged
        reconnectRequested = true
        do {
            let display = try session.currentDisplay(nowMicros: Self.monotonicMicros)
            lastError = nil
            await emit(display: display)
        } catch {
            lastError = error.localizedDescription
            await emit(display: nil)
        }
    }

    func suspend() async -> [AeroLinkConnectionHandle] {
        cycle &+= 1
        active = false
        reconnectRequested = false
        degraded.clearAll()
        var stopping = connections.values.map(\.handle)
        stopping.append(contentsOf: retired)
        connections.removeAll()
        retired.removeAll()
        do {
            try domain.reset()
            let display = try session.clearRadioRecords()
            lastError = nil
            await emit(display: display)
        } catch {
            lastError = error.localizedDescription
            await emit(display: nil)
        }
        return stopping
    }

    func maintenanceView() -> AeroLinkMaintenanceView {
        AeroLinkMaintenanceView(
            active: active,
            cycle: cycle,
            reconnectRequested: reconnectRequested,
            connections: connections.values.map(\.handle)
        )
    }

    func drainView() -> AeroLinkDrainView {
        let live: [AeroLinkDrainConnection] = connections.values.compactMap { connection in
            guard connection.availability == .streaming else { return nil }
            return AeroLinkDrainConnection(
                handle: connection.handle,
                reconnectGeneration: connection.diagnostics.reconnectGeneration
            )
        }
        return AeroLinkDrainView(active: active, cycle: cycle, connections: live)
    }

    func recordStatus(
        _ status: AeroLinkStatusValue,
        for handle: AeroLinkConnectionHandle,
        cycle expectedCycle: UInt64
    ) {
        guard isCurrent(handle, cycle: expectedCycle),
              var connection = connections[handle.key] else { return }
        let counters = connection.diagnostics
        connection.availability = status.availability
        connection.diagnostics = status.diagnostics
        connection.diagnostics.acceptedEvents = counters.acceptedEvents
        connection.diagnostics.rejectedInputs = counters.rejectedInputs
        connection.diagnostics.adsb1090GapSamples = counters.adsb1090GapSamples
        connection.diagnostics.uat978GapCount = counters.uat978GapCount
        connection.diagnostics.discardedUatBytes = counters.discardedUatBytes
        connection.diagnostics.drainLimitExhaustions = counters.drainLimitExhaustions
        connections[handle.key] = connection
        if status.availability == .ready || status.availability == .streaming,
           let band = handle.band {
            degraded.clear(band)
        }
    }

    func reject(
        _ handle: AeroLinkConnectionHandle,
        failure: AeroLinkFailure,
        cycle expectedCycle: UInt64
    ) async {
        guard isCurrent(handle, cycle: expectedCycle) else { return }
        connections.removeValue(forKey: handle.key)
        retired.append(handle)
        record(failure)
        reconnectRequested = true
        await emit(display: nil)
    }

    func merge(
        _ attempt: AeroLinkDiscoveryAttempt,
        cycle expectedCycle: UInt64
    ) async -> [AeroLinkConnectionHandle] {
        guard active, cycle == expectedCycle else {
            return attempt.prepared.map(\.handle) + attempt.discarded
        }
        var discarded = attempt.discarded
        if let failure = attempt.scanFailure {
            record(failure)
        } else if scanRetiresProcessFailure(
            hadOpenFailures: attempt.hadOpenFailures,
            hasScanError: false,
            hasReceiverFailures: !attempt.receiverFailures.isEmpty
        ) {
            degraded.clearUnscoped()
        }
        for failure in attempt.receiverFailures {
            record(failure)
        }
        for prepared in attempt.prepared {
            if connections[prepared.handle.key] != nil {
                discarded.append(prepared.handle)
                continue
            }
            connections[prepared.handle.key] = LiveAeroLinkConnection(
                handle: prepared.handle,
                availability: prepared.status.availability,
                diagnostics: prepared.status.diagnostics
            )
            if prepared.status.availability == .ready
                || prepared.status.availability == .streaming,
               let band = prepared.handle.band {
                degraded.clear(band)
            }
        }
        reconnectRequested = reconnectRequiredAfterScan(
            pending: reconnectRequested,
            hadOpenFailures: attempt.hadOpenFailures,
            hasScanError: attempt.scanFailure != nil,
            hasReceiverFailures: !attempt.receiverFailures.isEmpty
        )
        await emit(display: nil)
        return discarded
    }

    func takeRetired(cycle expectedCycle: UInt64) -> [AeroLinkConnectionHandle] {
        guard active, cycle == expectedCycle else { return [] }
        let stopping = retired
        retired.removeAll()
        return stopping
    }

    func finishMaintenance(
        cycle expectedCycle: UInt64,
        shouldContinueDiscovery: Bool,
        driverIsEnabled: Bool?,
        utcMillis: Int64,
        monotonicMicros: UInt64
    ) async {
        guard active, cycle == expectedCycle else { return }
        reconnectRequested = reconnectRequested || shouldContinueDiscovery
        idleAvailability = driverIsEnabled == false ? .driverDisabled : .unplugged
        do {
            let records = try domain.advanceTime(
                utcMillis: utcMillis,
                monotonicMicros: monotonicMicros
            )
            let display = try accept(records, monotonicMicros: monotonicMicros)
            lastError = nil
            await emit(display: display)
        } catch {
            lastError = error.localizedDescription
            await emit(display: nil)
        }
    }

    func commitDrain(
        _ result: AeroLinkDrainResult,
        from connection: AeroLinkDrainConnection,
        cycle expectedCycle: UInt64,
        utcMillis: Int64,
        monotonicMicros: UInt64
    ) async {
        guard result.hasConsumedTransfer else { return }
        let handle = connection.handle
        guard isCurrent(handle, cycle: expectedCycle),
              var live = connections[handle.key] else { return }
        live.diagnostics.acceptedEvents &+= result.accepted
        live.diagnostics.rejectedInputs &+= result.rejected
        live.diagnostics.adsb1090GapSamples = result.adsb1090GapSamples
        live.diagnostics.uat978GapCount = result.uat978GapCount
        live.diagnostics.discardedUatBytes = result.discardedUatBytes
        if result.limitExhausted {
            live.diagnostics.drainLimitExhaustions &+= 1
        }
        var display = DisplayAccumulator()
        var acceptedLine = false
        var rejectedLine = false
        for line in result.eventLines {
            do {
                let records = try domain.acceptReceptionEvent(
                    eventJson: line,
                    reconnectGeneration: connection.reconnectGeneration,
                    utcMillis: utcMillis,
                    monotonicMicros: monotonicMicros
                )
                try accept(records, monotonicMicros: monotonicMicros, into: &display)
                acceptedLine = true
            } catch {
                live.diagnostics.rejectedInputs &+= 1
                lastError = error.localizedDescription
                rejectedLine = true
            }
        }
        if acceptedLine && !rejectedLine {
            lastError = nil
        }
        connections[handle.key] = live
        await emit(display: display.batch)
    }

    private func record(_ failure: AeroLinkFailure) {
        if let band = failure.band {
            degraded.record(failure.availability, for: band)
        } else {
            degraded.recordUnscoped(failure.availability)
        }
    }

    private func accept(
        _ records: RadioRecordBatch,
        monotonicMicros: UInt64
    ) throws -> DisplayBatch? {
        var display = DisplayAccumulator()
        try accept(records, monotonicMicros: monotonicMicros, into: &display)
        return display.batch
    }

    private func accept(
        _ records: RadioRecordBatch,
        monotonicMicros: UInt64,
        into display: inout DisplayAccumulator
    ) throws {
        for record in records.trackRecords {
            display.append(try session.acceptTrackRecord(
                recordJson: record,
                nowMicros: monotonicMicros
            ))
        }
        for record in records.weatherRecords {
            display.append(try session.acceptWeatherRecord(
                recordJson: record,
                nowMicros: monotonicMicros
            ))
        }
    }

    private func isCurrent(
        _ handle: AeroLinkConnectionHandle,
        cycle expectedCycle: UInt64
    ) -> Bool {
        active && cycle == expectedCycle
            && connections[handle.key]?.handle.identity == handle.identity
    }

    private func emit(display: DisplayBatch?) async {
        let receivers = connections.values.compactMap { connection -> RadioReceiver? in
            guard let transport = connection.handle.transport,
                  let band = connection.handle.band else { return nil }
            return RadioReceiver(
                id: transport,
                band: band,
                availability: connection.availability,
                diagnostics: connection.diagnostics
            )
        }.sorted { $0.id.rawValue < $1.id.rawValue }
        let activeAvailability = Self.activeAvailability(for: receivers)
        let idle = connections.isEmpty && active ? idleAvailability : nil
        let availability = active
            ? degraded.effectiveAvailability(active: activeAvailability, idle: idle) ?? .checking
            : .suspended
        await publish(RadioRuntimeEmission(
            source: RadioSourceSnapshot(
                availability: availability,
                receivers: receivers,
                bandFailures: degraded.bandFailures
            ),
            display: display,
            errorMessage: lastError
        ))
    }

    private static func activeAvailability(
        for receivers: [RadioReceiver]
    ) -> RadioAvailability? {
        if receivers.contains(where: { $0.availability == .streaming }) {
            return .streaming
        }
        if receivers.contains(where: { $0.availability == .ready }) {
            return .ready
        }
        return receivers.isEmpty ? nil : .checking
    }

    private static var monotonicMicros: UInt64 {
        DispatchTime.now().uptimeNanoseconds / 1_000
    }
}

private struct DisplayAccumulator {
    private(set) var batch: DisplayBatch?
    private var changes: [DisplayPointChange] = []

    mutating func append(_ next: DisplayBatch) {
        changes.append(contentsOf: next.pointChanges)
        batch = next
        batch?.pointChanges = changes
    }
}
