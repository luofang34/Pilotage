import Foundation
import GameController
import IndicateAppleDisplay
import PilotageAppleInstrumentConsumer
import PilotageCore
import SwiftUI

/// One session-host link and the instrument pipeline it feeds.
///
/// Every session decision is the shared Rust core's; this model relays
/// typed events onto the main actor, holds display state, and forwards
/// game-controller demands. It decodes nothing and derives nothing.
@MainActor
final class HostLinkModel: ObservableObject {
    /// Where the link stands, for interface state.
    enum Phase: Equatable {
        case idle
        case connecting(host: String)
        case observing(host: String)
        case controlling(scope: String)
        case reconnecting
        case stopped(reason: String)
    }

    /// Where the link stands now.
    @Published private(set) var phase: Phase = .idle
    /// What the link screen shows about the session.
    @Published private(set) var status = "not connected"
    /// Offered vehicles once admitted, in the host's order.
    @Published private(set) var catalog: LinkCatalog?
    /// Whether control is held now.
    @Published private(set) var leaseHeld = false
    /// Why the instruments cannot paint, when they cannot.
    @Published private(set) var instrumentFault: String?
    /// The panel the operator selected, by registry index; persisted so
    /// the choice survives relaunch. Selection is among registry
    /// descriptors only — the shell owns no look of its own (ADR-0029).
    @Published var selectedPanel: UInt32 {
        didSet { UserDefaults.standard.set(Int(selectedPanel), forKey: Self.panelKey) }
    }
    /// Whether a game controller is attached.
    @Published private(set) var controllerAttached = false

    /// One registry panel with the index the runtime addresses it by.
    struct PanelChoice: Identifiable {
        let index: UInt32
        let descriptor: BridgePanelDescriptor
        var id: UInt32 { index }
    }

    /// Every panel the registry publishes, in registry order.
    let panels: [PanelChoice]

    private var link: LinkSession?
    private var composition: PilotageInstrumentComposition?
    private var displays: [UInt32: PanelDisplay] = [:]
    private var verified: VerifiedInstrumentRuntime?
    private var controlTimer: Timer?
    private var watchdog: Timer?
    /// The link clock of the last accepted frame, advanced locally while
    /// frames are absent so ages keep growing into flags.
    private var lastFrameClockMs: UInt64 = 0
    private var lastFrameWallMs: UInt64 = 0
    private static let panelKey = "pilotageInstrumentPanel"

    init() {
        selectedPanel = UInt32(max(0, UserDefaults.standard.integer(forKey: Self.panelKey)))
        var choices: [PanelChoice] = []
        for index in 0..<panelCount() {
            if let descriptor = panelDescriptor(index: index) {
                choices.append(PanelChoice(index: index, descriptor: descriptor))
            }
        }
        panels = choices
        watchControllers()
    }

    /// Verifies and builds the paint pipeline on first use, so a map-only
    /// launch never pays for the glyph atlas or the gate. A refusal shows
    /// its reason; an unverified instrument never paints (ADR-0032).
    func prepareInstruments() {
        guard composition == nil, verified == nil else { return }
        do {
            let verified = try LinkedInstrumentRuntime.verifiedRuntime()
            self.verified = verified
            composition = PilotageInstrumentComposition(verifiedRuntime: verified)
            instrumentFault = nil
        } catch {
            instrumentFault = String(describing: error)
        }
    }

    /// The kept display pipeline for one panel: failure latch, recovery
    /// streak, and glyph cache survive selection changes.
    func display(for panel: UInt32) -> PanelDisplay? {
        if let existing = displays[panel] { return existing }
        guard let composition,
              let verified,
              let choice = panels.first(where: { $0.index == panel }),
              let layerMask = UInt8(exactly: choice.descriptor.requiredLayers)
        else { return nil }
        let descriptor = choice.descriptor
        let size = CGSize(
            width: CGFloat(descriptor.designWidth),
            height: CGFloat(descriptor.designHeight)
        )
        guard let requirements = PanelRequirements(
            id: descriptor.id,
            title: descriptor.title,
            criticalLayerMask: layerMask,
            frameMin: size,
            frameMax: size,
            canonicalFrame: size
        ) else { return nil }
        let display = PanelDisplay(
            requirements: requirements,
            producer: composition.producer(panel: panel),
            atlas: verified.glyphAtlas
        )
        displays[panel] = display
        return display
    }

    /// Connects to a host. `certificateSha256Hex` empty accepts any
    /// certificate and exists for loopback development only.
    func connect(url: String, certificateSha256Hex: String) {
        disconnect()
        phase = .connecting(host: Self.hostName(of: url))
        status = "connecting to \(url)"
        do {
            link = try LinkSession.connect(
                config: LinkConfig(
                    url: url,
                    certificateSha256Hex: certificateSha256Hex,
                    clientName: "pilotage-ipad"
                ),
                observer: LinkRelay(model: self)
            )
        } catch {
            status = "connect failed: \(error.localizedDescription)"
        }
    }

    /// Stops the link and stands the control loop down.
    func disconnect() {
        controlTimer?.invalidate()
        controlTimer = nil
        watchdog?.invalidate()
        watchdog = nil
        if let link {
            link.shutdown()
            // The drop joins the driver runtime briefly; that wait belongs
            // off the interface thread.
            Task.detached { _ = link }
        }
        link = nil
        catalog = nil
        leaseHeld = false
        phase = .idle
        status = "not connected"
    }

    private static func hostName(of url: String) -> String {
        URL(string: url)?.host ?? url
    }

    /// The registry panel matching one profile tile id, by descriptor id
    /// first and title second, case-insensitively. The registry is the
    /// only source of what exists; a miss is the caller's typed reason.
    func panelChoice(forTileId id: String) -> PanelChoice? {
        let wanted = id.lowercased()
        return panels.first { $0.descriptor.id.lowercased() == wanted }
            ?? panels.first { $0.descriptor.title.lowercased() == wanted }
    }

    /// Asks for control of the first advertised scope.
    func requestLease() {
        guard let vehicle = catalog?.vehicles.first,
              let scope = vehicle.scopes.first
        else { return }
        link?.requestLease(vehicleId: vehicle.vehicleId, scope: scope.scope)
    }

    /// Stands down from control.
    func releaseLease() {
        link?.releaseLease()
    }

    /// Fetches the session manifest xtask serves and connects with it:
    /// the same three facts the browser reads, taken from the same file,
    /// so a hand-typed hash is never the price of pinning.
    func connectFromManifest(_ manifestUrl: String) {
        guard let url = URL(string: manifestUrl) else {
            status = "manifest url is invalid"
            return
        }
        status = "fetching \(manifestUrl)"
        Task { [weak self] in
            do {
                let (data, _) = try await URLSession.shared.data(from: url)
                let manifest = try JSONDecoder().decode(SessionManifest.self, from: data)
                await MainActor.run {
                    self?.connect(
                        url: "https://\(manifest.host):\(manifest.port)/pilotage",
                        certificateSha256Hex: manifest.certHash
                    )
                }
            } catch {
                await MainActor.run {
                    self?.status = "manifest fetch failed: \(error.localizedDescription)"
                }
            }
        }
    }

    /// The connect facts `cargo xtask sim` serves at `/session.json`.
    private struct SessionManifest: Decodable {
        let host: String
        let port: UInt16
        let certHash: String
    }

    // MARK: - Link relay targets (main actor)

    fileprivate func accept(_ event: LinkEvent) {
        switch event {
        case .admitted(let catalog):
            self.catalog = catalog
            phase = .observing(host: catalog.hostVersion)
            status = "admitted by \(catalog.hostVersion)"
        case .leaseChanged(let held, let scope, let detail):
            leaseHeld = held
            if held {
                phase = .controlling(scope: scope)
            } else if let catalog {
                phase = .observing(host: catalog.hostVersion)
            }
            status = held ? "controlling \(scope)" : "observing (\(detail))"
            held ? startControlLoop() : stopControlLoop()
        case .controlRejected(let sequence):
            status = "control frame \(sequence) rejected"
        case .down(let retryAtMs):
            leaseHeld = false
            stopControlLoop()
            phase = retryAtMs == nil ? .idle : .reconnecting
            status = retryAtMs == nil ? "disconnected" : "reconnecting…"
        case .stopped(let reason):
            leaseHeld = false
            stopControlLoop()
            phase = .stopped(reason: reason)
            status = "stopped: \(reason)"
        }
    }

    fileprivate func accept(stateFrame: [UInt8], acceptedAtMs: UInt64) {
        guard let composition else { return }
        lastFrameClockMs = acceptedAtMs
        lastFrameWallMs = Self.wallMs()
        do {
            try composition.writeState(stateFrame, acceptedAtMs: acceptedAtMs)
            try composition.compose(nowMs: acceptedAtMs, pathHealthy: true)
            instrumentFault = nil
        } catch {
            // The panel's own health latch covers the screen; this is the
            // reason the drawer can show for it. The fault clears on the
            // next accepted frame rather than latching a transient forever.
            instrumentFault = String(describing: error)
        }
        startWatchdog()
    }

    /// Recomposes on a local clock while frames are absent, so a silent
    /// link ages the committed scene into stale and failed flags instead
    /// of repainting the last good reading as if it were live.
    private func startWatchdog() {
        guard watchdog == nil else { return }
        let timer = Timer(timeInterval: 0.5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.watchdogTick() }
        }
        RunLoop.main.add(timer, forMode: .common)
        watchdog = timer
    }

    private func watchdogTick() {
        guard let composition, lastFrameWallMs > 0 else { return }
        let silentMs = Self.wallMs() - lastFrameWallMs
        // Fresh frames drive composition themselves; the watchdog composes
        // only across silence, with the link clock advanced by it.
        guard silentMs > 500 else { return }
        try? composition.compose(
            nowMs: lastFrameClockMs + silentMs,
            pathHealthy: false
        )
    }

    private static func wallMs() -> UInt64 {
        UInt64(DispatchTime.now().uptimeNanoseconds / 1_000_000)
    }

    // MARK: - Game controller

    private func watchControllers() {
        controllerAttached = GCController.controllers().contains { $0.extendedGamepad != nil }
        NotificationCenter.default.addObserver(
            forName: .GCControllerDidConnect, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in self?.controllerAttached = true }
        }
        NotificationCenter.default.addObserver(
            forName: .GCControllerDidDisconnect, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor in
                self?.controllerAttached =
                    GCController.controllers().contains { $0.extendedGamepad != nil }
            }
        }
    }

    /// Sends the current stick demand at a fixed cadence while control is
    /// held. The demand is normalized here and scaled by the ADVERTISED
    /// envelope in the shared core; an unfenced demand dies there, not
    /// here.
    private func startControlLoop() {
        controlTimer?.invalidate()
        let timer = Timer(timeInterval: 0.05, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.sendDemand() }
        }
        RunLoop.main.add(timer, forMode: .common)
        controlTimer = timer
    }

    private func stopControlLoop() {
        controlTimer?.invalidate()
        controlTimer = nil
    }

    private func sendDemand() {
        guard leaseHeld,
              let pad = GCController.controllers()
                  .compactMap(\.extendedGamepad)
                  .first
        else { return }
        // Left stick: throttle up / yaw right. Right stick: pitch forward /
        // roll right. The GameController framework already normalizes and
        // deadzones the axes.
        link?.sendMotion(
            roll: pad.rightThumbstick.xAxis.value,
            pitch: pad.rightThumbstick.yAxis.value,
            throttle: pad.leftThumbstick.yAxis.value,
            yaw: pad.leftThumbstick.xAxis.value
        )
    }
}

/// Hops link callbacks from the driver's thread onto the main actor.
private final class LinkRelay: LinkObserver, @unchecked Sendable {
    private weak var model: HostLinkModel?

    init(model: HostLinkModel) {
        self.model = model
    }

    func onEvent(event: LinkEvent) {
        Task { @MainActor [model] in model?.accept(event) }
    }

    func onStateFrame(frame: Data, acceptedAtMs: UInt64) {
        Task { @MainActor [model] in
            model?.accept(stateFrame: Array(frame), acceptedAtMs: acceptedAtMs)
        }
    }
}
