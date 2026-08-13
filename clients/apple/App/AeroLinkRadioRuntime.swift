@preconcurrency import AeroLinkAppleClient
import Foundation
import OSLog
import PilotageRadioSource
import PilotageCore

final class AeroLinkRadioRuntime: @unchecked Sendable {
    static let maximumTransfersPerCycle = 32
    static let drainInterval = Duration.milliseconds(20)
    static let maintenanceInterval = Duration.seconds(1)

    private let discovery: AeroLinkDiscoveryGate
    private let state: AeroLinkRadioState
    private let logger: Logger

    init(
        session: PresentationSession,
        domain: RadioDomainSession,
        discovery: AeroLinkDiscoveryGate,
        publish: @escaping @Sendable (RadioRuntimeEmission) async -> Void
    ) {
        self.discovery = discovery
        state = AeroLinkRadioState(
            session: session,
            domain: domain,
            publish: publish
        )
        logger = Logger(
            subsystem: Bundle.main.bundleIdentifier ?? "org.luofang.pilotage",
            category: "radio"
        )
    }

    /// Take every reception event as it arrives, or stop taking them.
    func observeReceptionLines(_ observer: (@Sendable ([String]) -> Void)?) async {
        await state.observeReceptionLines(observer)
    }

    func activate() async {
        await state.activate()
    }

    func suspend() async {
        let stopping = await state.suspend()
        stop(stopping, reason: "suspend")
    }

    func maintenanceLoop() async {
        while !Task.isCancelled {
            let view = await state.maintenanceView()
            if view.active {
                await maintenanceCycle(view)
            }
            do {
                try await Task.sleep(for: Self.maintenanceInterval)
            } catch {
                return
            }
        }
    }

    func drainLoop() async {
        while !Task.isCancelled {
            let view = await state.drainView()
            if view.active {
                for connection in view.connections {
                    if Task.isCancelled { return }
                    do {
                        let result = try drain(connection.handle)
                        let now = Self.timestamps()
                        await state.commitDrain(
                            result,
                            from: connection,
                            cycle: view.cycle,
                            utcMillis: now.utcMillis,
                            monotonicMicros: now.monotonicMicros
                        )
                    } catch {
                        await state.reject(
                            connection.handle,
                            failure: AeroLinkFailure.classify(
                                error,
                                for: connection.handle
                            ),
                            cycle: view.cycle
                        )
                    }
                }
            }
            do {
                try await Task.sleep(for: Self.drainInterval)
            } catch {
                return
            }
        }
    }

    private func maintenanceCycle(_ initial: AeroLinkMaintenanceView) async {
        let retired = await state.takeRetired(cycle: initial.cycle)
        stop(retired, reason: "reconnect")
        await refresh(initial.connections, cycle: initial.cycle)

        var current = await state.maintenanceView()
        guard current.active, current.cycle == initial.cycle else { return }
        let shouldScan = current.reconnectRequested
            || discovery.shouldContinue(for: current.connections)
        if shouldScan {
            let attempt = prepareDiscovery(existing: current.connections)
            let discarded = await state.merge(attempt, cycle: current.cycle)
            stop(discarded, reason: "discovery discard")
        }

        let newlyRetired = await state.takeRetired(cycle: initial.cycle)
        stop(newlyRetired, reason: "receiver failure")
        current = await state.maintenanceView()
        guard current.active, current.cycle == initial.cycle else { return }
        let now = Self.timestamps()
        await state.finishMaintenance(
            cycle: current.cycle,
            shouldContinueDiscovery: discovery.shouldContinue(for: current.connections),
            driverIsEnabled: discovery.driverIsEnabled(),
            utcMillis: now.utcMillis,
            monotonicMicros: now.monotonicMicros
        )
    }

    private func refresh(
        _ connections: [AeroLinkConnectionHandle],
        cycle: UInt64
    ) async {
        for handle in connections {
            if Task.isCancelled { return }
            do {
                let status = try prepare(handle)
                if let failure = AeroLinkStatusValue.reconnectFailure(
                    for: status.driverState
                ) {
                    await state.reject(
                        handle,
                        failure: AeroLinkFailure(
                            availability: failure,
                            band: handle.band
                        ),
                        cycle: cycle
                    )
                } else {
                    await state.recordStatus(
                        AeroLinkStatusValue(status, current: RadioDiagnostics()),
                        for: handle,
                        cycle: cycle
                    )
                }
            } catch {
                logger.error(
                    "receiver maintenance failed: \(error.localizedDescription, privacy: .public)"
                )
                await state.reject(
                    handle,
                    failure: AeroLinkFailure.classify(error, for: handle),
                    cycle: cycle
                )
            }
        }
    }

    private func prepareDiscovery(
        existing: [AeroLinkConnectionHandle]
    ) -> AeroLinkDiscoveryAttempt {
        let scan = discovery.openConnections()
        var attempt = AeroLinkDiscoveryAttempt()
        attempt.hadOpenFailures = scan.hadOpenFailures
        attempt.scanFailure = scan.failure
        let known = Set(existing.map(\.key))
        for handle in scan.connections {
            if Task.isCancelled || known.contains(handle.key)
                || handle.band == nil || handle.transport == nil {
                attempt.discarded.append(handle)
                continue
            }
            do {
                let status = try prepare(handle)
                if let failure = AeroLinkStatusValue.reconnectFailure(
                    for: status.driverState
                ) {
                    attempt.receiverFailures.append(AeroLinkFailure(
                        availability: failure,
                        band: handle.band
                    ))
                    attempt.discarded.append(handle)
                } else {
                    attempt.prepared.append(PreparedAeroLinkConnection(
                        handle: handle,
                        status: AeroLinkStatusValue(
                            status,
                            current: RadioDiagnostics()
                        )
                    ))
                }
            } catch {
                attempt.receiverFailures.append(
                    AeroLinkFailure.classify(error, for: handle)
                )
                attempt.discarded.append(handle)
            }
        }
        return attempt
    }

    private func prepare(_ handle: AeroLinkConnectionHandle) throws -> ALReceiverStatus {
        var status = try handle.value.status()
        if status.driverState == .ready {
            try handle.value.start()
            status = try handle.value.status()
        }
        return status
    }

    private func drain(_ handle: AeroLinkConnectionHandle) throws -> AeroLinkDrainResult {
        var result = AeroLinkDrainResult()
        var consumed = 0
        while consumed < Self.maximumTransfersPerCycle {
            let batch = try handle.value.poll()
            guard batch.transferConsumed else { return result }
            result.hasConsumedTransfer = true
            result.eventLines.append(contentsOf: batch.jsonLines
                .split(whereSeparator: \.isNewline)
                .map(String.init))
            result.accepted &+= batch.accepted
            result.rejected &+= batch.rejected
            result.adsb1090GapSamples = batch.adsb1090GapSamples
            result.uat978GapCount = batch.uat978GapCount
            result.discardedUatBytes = batch.discardedUatBytes
            consumed &+= 1
        }
        result.limitExhausted = true
        return result
    }

    private func stop(_ connections: [AeroLinkConnectionHandle], reason: String) {
        for handle in connections {
            do {
                try handle.value.stop()
            } catch {
                logger.error(
                    "receiver \(reason, privacy: .public) failed: \(error.localizedDescription, privacy: .public)"
                )
            }
        }
    }

    private static func timestamps() -> (utcMillis: Int64, monotonicMicros: UInt64) {
        (
            Int64(Date().timeIntervalSince1970 * 1_000),
            DispatchTime.now().uptimeNanoseconds / 1_000
        )
    }
}
