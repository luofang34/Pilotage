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
    /// Display of the replay in progress, when a flight is open.
    @Published private(set) var replayDisplay: DisplayBatch?
    /// Recordings the container holds.
    @Published private(set) var flights: [Flight] = []
    /// The notice each map source asks to be shown.
    ///
    /// Taken from the style document, which is what says which sources draw and what each
    /// one asks for. The loaded map may report the same notices later and is welcome to,
    /// but a credit that waits for a map to finish loading is a credit a reader can open
    /// the panel and not find.
    @Published private(set) var mapAttributions: [String] = SituationStyleResource.attributions()
    /// Whether the client claims the radios.
    @Published var adsbEnabled: Bool {
        didSet {
            guard adsbEnabled != oldValue else { return }
            UserDefaults.standard.set(adsbEnabled, forKey: Self.adsbEnabledKey)
            Task { await adsbEnabled ? activateRadio() : suspendRadio() }
        }
    }

    /// Batch the map draws. A reader who opened a flight sees the flight.
    var mapDisplay: DisplayBatch? { replayDisplay ?? display }

    /// The flight being replayed, when one is open.
    var replayingFlight: Flight? { replayRun?.flight }

    /// Whether something the reader should see is wrong.
    ///
    /// Reception state moved into the drawer, so a band that failed leaves the map with
    /// nothing on it and no reason given. A map that looks clear because a receiver died
    /// is the failure the layer states exist to prevent.
    var hasAttention: Bool {
        errorMessage != nil || (adsbEnabled && !radioSource.bandFailures.isEmpty)
    }

    private let session: PresentationSession
    private let domain: RadioDomainSession?
    private let terrainArchivePath: String?
    private let terrainAvailable: Bool
    private let discovery = AeroLinkDiscoveryGate()
    private let evidenceWriter = SituationEvidenceWriter()
    /// Receives the aircraft's own position whenever a batch carries one.
    var onOwnship: ((DisplayOwnship?) -> Void)?
    /// Reads the position the map may centre on, for the evidence file.
    var currentOwnship: (() -> (OwnshipFix?, HeadingFix?, FollowMode, DeviceLocationAuthorisation, Bool))?
    private var replayTask: Task<Void, Never>?
    private var pendingDisplay: DisplayBatch?
    private var publishTask: Task<Void, Never>?
    private var lastPublishMicros: UInt64 = 0
    private var pendingReplayDisplay: DisplayBatch?
    private var replayPublishTask: Task<Void, Never>?
    private var lastReplayPublishMicros: UInt64 = 0
    private var replayRun: SituationReplayRun?
    private var runtime: AeroLinkRadioRuntime?
    private var maintenanceTask: Task<Void, Never>?
    private var drainTask: Task<Void, Never>?
    private var cleanupTask: (generation: UInt64, task: Task<Void, Never>)?
    private var cleanupGeneration: UInt64 = 0
    private var startupError: String?
    private var isActive = false

    /// Preference key for radio reception.
    static let adsbEnabledKey = "pilotageAdsbInEnabled"

    init() {
        adsbEnabled = UserDefaults.standard.bool(forKey: Self.adsbEnabledKey)
        let source = RadioSourceSnapshot(
            availability: .suspended,
            receivers: [],
            bandFailures: []
        )
        let session = PresentationSession()
        self.session = session
        radioSource = source
        let terrainArchivePath = Bundle.main.url(
            forResource: "SituationTerrain",
            withExtension: "mbtiles"
        )?.path
        self.terrainArchivePath = terrainArchivePath
        var loadedTerrain = false
        var initialError: String?
        if let terrainArchivePath {
            do {
                try session.loadTerrainArchiveBlocking(archivePath: terrainArchivePath)
                loadedTerrain = true
            } catch {
                initialError = Self.join(initialError, error.localizedDescription)
            }
        }
        var createdDomain: RadioDomainSession?
        var initialDisplay: DisplayBatch?
        do {
            initialDisplay = try session.observeSources(
                observation: PresentationSourceObservation(
                    source: source,
                    terrainAvailable: loadedTerrain
                ),
                nowMicros: Self.monotonicMicros
            )
            createdDomain = try RadioDomainSession()
        } catch {
            initialError = Self.join(initialError, error.localizedDescription)
        }
        terrainAvailable = loadedTerrain
        domain = createdDomain
        startupError = initialError
        display = initialDisplay
        errorMessage = initialError
    }

    func activate() async {
        recordEvidence()
        reloadFlights()
        // Reception is optional. A reader who has not turned it on gets no driver
        // discovery, no receiver claim, and no radio state on the map.
        guard adsbEnabled else { return }
        await activateRadio()
    }

    private func activateRadio() async {
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
        await suspendRadio()
    }

    private func suspendRadio() async {
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

    /// Hold one batch for the map, at a rate a screen can show.
    ///
    /// A batch arrives for every reception, and a busy band produces them far faster than
    /// a display refreshes. Publishing each one drives a full SwiftUI update and a source
    /// replacement that nobody ever sees. The newest batch always wins, and the trailing
    /// one is never dropped, so the map settles on the current picture rather than on
    /// whichever batch happened to arrive on a boundary.
    private func applyDisplay(_ next: DisplayBatch) {
        pendingDisplay = next
        let now = Self.monotonicMicros
        if now &- lastPublishMicros >= Self.publishIntervalMicros {
            publishPendingDisplay()
            return
        }
        guard publishTask == nil else { return }
        publishTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.publishIntervalMicros * 1_000)
            guard let self else { return }
            self.publishTask = nil
            self.publishPendingDisplay()
        }
    }

    private func publishPendingDisplay() {
        guard let next = pendingDisplay else { return }
        pendingDisplay = nil
        lastPublishMicros = Self.monotonicMicros
        display = next
        // The aircraft's own return is a position the client can be sure belongs to this
        // aircraft, so it reaches the ownship model rather than stopping at the map.
        onOwnship?(next.ownship)
        if let selected = selectedTraffic {
            selectedTraffic = next.trafficDetails.first { $0.id == selected.id }
        }
        recordEvidence()
    }

    /// Write the evidence again after the view has supplied its readers.
    ///
    /// The first write happens as the client activates, before the view is on screen, so
    /// everything the view reports would otherwise stay absent until something else
    /// changed. A file that says nothing because it was written too early is worse than
    /// one that says nothing because there is nothing to say.
    func refreshEvidence() {
        recordEvidence()
    }

    /// Take the notices the loaded style asks to be shown.
    func observeMapAttributions(_ notices: [String]) {
        // A map that reports nothing has not withdrawn the notices its style declares.
        guard !notices.isEmpty, notices != mapAttributions else { return }
        mapAttributions = notices
    }

    /// Recordings the container holds.
    func reloadFlights() {
        flights = FlightsLibrary.flights()
    }

    /// Open one recorded flight.
    ///
    /// A replay never touches the live map. It owns its decoders and its display, and the
    /// map shows whichever of the two the reader asked for, because a map fed from a file
    /// looks exactly like a map fed from a radio.
    func startReplay(_ flight: Flight) {
        stopReplay()
        guard let run = SituationReplayRun(
            flight: flight,
            terrainArchivePath: terrainArchivePath
        ) else {
            errorMessage = Self.join(startupError, "\(flight.receptionFileName) cannot be read.")
            return
        }
        replayRun = run
        replayTask = Task { [weak self] in
            await run.run(deviceMonotonicMicros: Self.monotonicMicros) { batch in
                guard let self else { return }
                self.applyReplayDisplay(batch)
            }
            self?.flushReplayDisplay()
            self?.recordEvidence()
        }
    }

    /// Close the replay and put the live map back.
    func stopReplay() {
        replayTask?.cancel()
        replayTask = nil
        replayRun = nil
        pendingReplayDisplay = nil
        replayDisplay = nil
        selectedTraffic = nil
        recordEvidence()
    }

    private func applyReplayDisplay(_ next: DisplayBatch) {
        pendingReplayDisplay = next
        let now = Self.monotonicMicros
        if now &- lastReplayPublishMicros >= Self.publishIntervalMicros {
            flushReplayDisplay()
            return
        }
        guard replayPublishTask == nil else { return }
        replayPublishTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: Self.publishIntervalMicros * 1_000)
            guard let self else { return }
            self.replayPublishTask = nil
            self.flushReplayDisplay()
        }
    }

    private func flushReplayDisplay() {
        guard let next = pendingReplayDisplay else { return }
        pendingReplayDisplay = nil
        lastReplayPublishMicros = Self.monotonicMicros
        replayDisplay = next
        recordEvidence()
    }

    private func recordEvidence() {
        evidenceWriter.record(
            SituationEvidence(
                batch: display,
                radioSource: radioSource,
                ownship: currentOwnship?().0,
                heading: currentOwnship?().1,
                follow: currentOwnship?().2 ?? .idle,
                deviceLocationAuthorisation: currentOwnship?().3 ?? .undetermined,
                deviceLocationEnabled: currentOwnship?().4 ?? false,
                replay: replayRun,
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

    /// Shortest gap between two map updates, in microseconds.
    private static let publishIntervalMicros: UInt64 = 100_000

    private static var monotonicMicros: UInt64 {
        DispatchTime.now().uptimeNanoseconds / 1_000
    }
}
