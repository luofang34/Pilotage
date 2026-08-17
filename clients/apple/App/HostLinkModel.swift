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

    /// Where the link stands now. A live link pins the idle timer: a
    /// locked screen suspends the app, the demand loop with it, and the
    /// host's silence watchdog then hands the vehicle to the link-loss
    /// failsafe mid-flight.
    @Published private(set) var phase: Phase = .idle {
        didSet {
            let live = switch phase {
            case .idle, .stopped: false
            case .connecting, .observing, .controlling, .reconnecting: true
            }
            UIApplication.shared.isIdleTimerDisabled = live
        }
    }
    /// What the link screen shows about the session.
    @Published private(set) var status = "not connected"
    /// Offered vehicles once admitted, in the host's order.
    @Published private(set) var catalog: LinkCatalog?
    /// Whether control is held now.
    @Published private(set) var leaseHeld = false
    /// The resolved pad profile and its arm/disarm control names.
    @Published private(set) var padHints = ""
    /// Whether the gimbal quasimode holds the right stick now.
    @Published private(set) var gimbalCaptured = false
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
    /// One second of link accounting, verbatim from the driver.
    @Published private(set) var linkStats = ""
    /// A pending ask from another operator, while this client holds.
    @Published var takeoverAsk: (fromPrincipal: UInt64, scope: String)?
    /// Whether the last denial named a standing holder, so the screen can
    /// offer the ask instead of a dead button.
    @Published private(set) var holderPresent = false
    /// The latest decoded picture per video source id.
    @Published private(set) var videoImages: [UInt8: UIImage] = [:]
    /// When each source last produced a picture, on the wall clock.
    @Published private(set) var videoSeenAtMs: [UInt8: UInt64] = [:]

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

    /// Arms the vehicle under the held lease. A disarmed vehicle ignores
    /// motion setpoints, so this is the gate between holding control and
    /// the sticks doing anything.
    func arm() {
        link?.sendAction(code: 1)
    }

    /// Disarms the vehicle under the held lease.
    func disarm() {
        link?.sendAction(code: 2)
    }

    /// Asks the present holder to hand control over; the handover
    /// finishes without another press here if they confirm.
    func requestTakeover() {
        guard let vehicle = catalog?.vehicles.first,
              let scope = vehicle.scopes.first
        else { return }
        status = "asked the holder for control"
        link?.requestTakeover(vehicleId: vehicle.vehicleId, scope: scope.scope)
    }

    /// Hands control to the principal who asked.
    func confirmHandover() {
        guard let ask = takeoverAsk else { return }
        link?.offerTransfer(toPrincipal: ask.fromPrincipal, scope: ask.scope)
        takeoverAsk = nil
    }

    /// Keeps control; the ask expires host-side.
    func declineHandover() {
        takeoverAsk = nil
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
        #if DEBUG
        if LaunchRequest.autoControl { print("harness event: \(event)") }
        #endif
        switch event {
        case .admitted(let catalog):
            self.catalog = catalog
            phase = .observing(host: catalog.hostVersion)
            status = "admitted by \(catalog.hostVersion)"
            if controllerAttached {
                selectPad(vendorName: lastPadVendor)
            }
            if LaunchRequest.autoControl {
                requestLease()
            }
        case .leaseChanged(let held, let scope, let detail):
            // The gimbal lane is the runtime's own business: its grant
            // must not flip the shell into "controlling", nor its
            // release stop the demand loop that keeps motion alive.
            guard scope == "vehicle.motion" else {
                if !held && !detail.isEmpty { status = "gimbal: \(detail)" }
                return
            }
            leaseHeld = held
            holderPresent = !held && detail.contains("another operator")
            if holderPresent {
                // The engine already escalated the denial into the ask;
                // the operator's one press keeps working on its own.
                status = "asked the holder of \(scope) to hand over"
                held ? startControlLoop() : stopControlLoop()
                return
            }
            if held {
                phase = .controlling(scope: scope)
            } else if let catalog {
                phase = .observing(host: catalog.hostVersion)
            }
            status = held ? "controlling \(scope)" : "observing (\(detail))"
            held ? startControlLoop() : stopControlLoop()
            if held && LaunchRequest.autoArm {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) { [weak self] in
                    self?.arm()
                }
            }
        case .padSelected(let label, let armHint, let disarmHint):
            padHints = "\(label): arm \(armHint) · disarm \(disarmHint)"
        case .pressSuppressed(let action):
            status = action == 1
                ? "arm press ignored — request control first"
                : "disarm press ignored — request control first"
        case .gimbalCapture(let active):
            gimbalCaptured = active
        case .notice(let text):
            status = text
        case .controlRejected(let sequence, let reason):
            status = "control frame \(sequence) rejected (reason \(reason))"
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
        case .takeoverAsked(let fromPrincipal, let scope):
            takeoverAsk = (fromPrincipal, scope)
        case .actionResult(let action, let accepted, let detail):
            let name = action == 1 ? "arm"
                : action == 2 ? "disarm"
                : action == 4 ? "gimbal recenter"
                : "action \(action)"
            status = accepted ? "\(name) accepted" : "\(name) rejected: \(detail)"
            if action == 1, accepted, LaunchRequest.autoClimb {
                climbUntil = Date().addingTimeInterval(15)
            }
        case .stats(
            let telemetry,
            let stateFrames,
            let controlFrames,
            let rejected,
            let actionResults,
            let streamPendingBytes
        ):
            linkStats = "tlm \(telemetry)/s · state \(stateFrames)/s · ctl "
                + "\(controlFrames)/s · rej \(rejected) · act \(actionResults)"
            if streamPendingBytes > 1_048_576 {
                linkStats += " · buf \(streamPendingBytes / 1_048_576) MB"
            }
        }
    }

    fileprivate func accept(videoImage: UIImage, sourceId: UInt8) {
        if LaunchRequest.publishNoStore {
            // The image crossed the hop and dies here: the bisect run
            // that separates the hop from the store-and-render side.
            _ = videoImage.size
            return
        }
        if LaunchRequest.storeClockOnly {
            videoSeenAtMs[sourceId] = Self.wallMs()
            return
        }
        videoImages[sourceId] = videoImage
        videoSeenAtMs[sourceId] = Self.wallMs()
    }

    fileprivate func accept(stateFrame: [UInt8], acceptedAtMs: UInt64) {
        guard let composition else { return }
        lastFrameClockMs = acceptedAtMs
        lastFrameWallMs = Self.wallMs()
        do {
            // One lock scope for the write and the recompose: the panel
            // render worker is free-running, and a render landing between
            // the two would fail, latch, and flap the whole panel.
            try composition.ingest(stateFrame, acceptedAtMs: acceptedAtMs, pathHealthy: true)
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
        if let pad = GCController.controllers().first(where: { $0.extendedGamepad != nil }) {
            selectPad(vendorName: pad.vendorName)
        }
        NotificationCenter.default.addObserver(
            forName: .GCControllerDidConnect, object: nil, queue: .main
        ) { [weak self] note in
            // Only the name crosses the isolation hop; the controller
            // object itself stays on the posting side.
            let vendorName = (note.object as? GCController)?.vendorName
            Task { @MainActor in
                self?.controllerAttached = true
                self?.selectPad(vendorName: vendorName)
            }
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

    /// Resolves the pad against the runtime's layered profile registry;
    /// the hints that come back name the arm control in the operator's
    /// terms, proving the button packing reads as intended.
    private func selectPad(vendorName: String?) {
        lastPadVendor = vendorName ?? lastPadVendor
        link?.selectPad(id: lastPadVendor ?? "gamepad")
    }

    /// The most recent pad's name, replayed into a link that connects
    /// after the pad did.
    private var lastPadVendor: String?

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

    /// Harness climb window; the demand loop climbs until it passes.
    private var climbUntil = Date.distantPast
    /// Harness demand-tick counter, printed to prove the loop runs.
    private var demandTicks = 0
    /// The pressed-button set last printed by the harness.
    private var lastPressedPrint: [Int] = []

    #if DEBUG
    /// Prints the process physical footprint once per interval, so a
    /// console-attached run shows a leak as a slope, not a surprise.
    static func startFootprintProbe() {
        guard LaunchRequest.openInstruments else { return }
        Timer.scheduledTimer(withTimeInterval: 10, repeats: true) { _ in
            var info = task_vm_info_data_t()
            var count = mach_msg_type_number_t(
                MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
            let result = withUnsafeMutablePointer(to: &info) {
                $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                    task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
                }
            }
            if result == KERN_SUCCESS {
                let mb = Double(info.phys_footprint) / 1_048_576
                print("harness memory: footprint \(Int(mb)) MB")
            }
        }
    }
    #endif

    private func sendDemand() {
        guard leaseHeld else { return }
        // The host's silence watchdog revokes a holder that stops
        // sending: one second of quiet cost the lease while the screen
        // still said "controlling". Frames flow at rate for as long as
        // the lease is held — a neutral demand when no stick is attached
        // is the holder's liveness, exactly as the browser streams it.
        let climb: Float = climbUntil > Date() ? 0.6 : 0
        #if DEBUG
        demandTicks += 1
        if LaunchRequest.autoControl && demandTicks % 20 == 0 {
            print("harness demand: tick \(demandTicks) held=\(leaseHeld) climb=\(climb)")
        }
        #endif
        guard let pad = GCController.controllers()
            .compactMap(\.extendedGamepad)
            .first
        else {
            link?.sendMotion(roll: 0, pitch: 0, throttle: climb, yaw: 0)
            return
        }
        // The raw sample in Standard Gamepad terms; every mapping,
        // curve, edge, and the gimbal quasimode live in the shared
        // runtime the browser also runs — nothing is bound here.
        // W3C sticks read down as positive; GameController reads up as
        // positive, so the Y axes flip.
        let axes: [Float] = [
            pad.leftThumbstick.xAxis.value,
            -pad.leftThumbstick.yAxis.value,
            pad.rightThumbstick.xAxis.value,
            -pad.rightThumbstick.yAxis.value,
        ]
        let buttons: [(Float, Bool)] = [
            (pad.buttonA.value, pad.buttonA.isPressed),
            (pad.buttonB.value, pad.buttonB.isPressed),
            (pad.buttonX.value, pad.buttonX.isPressed),
            (pad.buttonY.value, pad.buttonY.isPressed),
            (pad.leftShoulder.value, pad.leftShoulder.isPressed),
            (pad.rightShoulder.value, pad.rightShoulder.isPressed),
            (pad.leftTrigger.value, pad.leftTrigger.isPressed),
            (pad.rightTrigger.value, pad.rightTrigger.isPressed),
            (pad.buttonOptions?.value ?? 0, pad.buttonOptions?.isPressed ?? false),
            (pad.buttonMenu.value, pad.buttonMenu.isPressed),
            (pad.leftThumbstickButton?.value ?? 0, pad.leftThumbstickButton?.isPressed ?? false),
            (pad.rightThumbstickButton?.value ?? 0, pad.rightThumbstickButton?.isPressed ?? false),
            (pad.dpad.up.value, pad.dpad.up.isPressed),
            (pad.dpad.down.value, pad.dpad.down.isPressed),
            (pad.dpad.left.value, pad.dpad.left.isPressed),
            (pad.dpad.right.value, pad.dpad.right.isPressed),
        ]
        #if DEBUG
        let pressedNow = buttons.enumerated().filter(\.element.1).map(\.offset)
        if LaunchRequest.autoControl && pressedNow != lastPressedPrint {
            lastPressedPrint = pressedNow
            print("harness pad: pressed \(pressedNow)")
        }
        #endif
        link?.sendPadSample(
            axes: axes,
            values: buttons.map(\.0),
            pressed: buttons.map(\.1)
        )
    }
}

/// Hops link callbacks from the driver's thread onto the main actor.
/// Video decodes here, off the interface thread; only pictures cross.
private final class LinkRelay: LinkObserver, @unchecked Sendable {
    private weak var model: HostLinkModel?
    private let decoders = NSMutableDictionary()
    private let decoderLock = NSLock()
    /// Sources with a decode-and-publish in flight. Video is droppable
    /// by design: decoding every frame while the interface lags queued
    /// unbounded bitmaps until the process hit its memory limit — the
    /// operator saw the application vanish, and the jetsam record saw a
    /// five-gigabyte footprint. One frame in flight per source, the
    /// rest dropped; the next kept frame is always the newest.
    private var inFlight: Set<UInt8> = []

    init(model: HostLinkModel) {
        self.model = model
    }

    func onVideoFrame(sourceId: UInt8, codec: String, payload: Data) {
        if LaunchRequest.noVideoDecode { return }
        decoderLock.lock()
        if inFlight.contains(sourceId) {
            decoderLock.unlock()
            return
        }
        inFlight.insert(sourceId)
        let key = NSNumber(value: sourceId)
        let decoder: VideoTileDecoder
        if let existing = decoders[key] as? VideoTileDecoder {
            decoder = existing
        } else {
            decoder = VideoTileDecoder()
            decoders[key] = decoder
        }
        decoderLock.unlock()
        let image = decoder.decode(codec: codec, payload: Array(payload))
            .flatMap(Self.plausible)
        if LaunchRequest.decodeNoPublish {
            clearInFlight(sourceId)
            return
        }
        Task { @MainActor [model] in
            if let image {
                model?.accept(videoImage: image, sourceId: sourceId)
            }
            self.clearInFlight(sourceId)
        }
    }

    /// Refuses a decoded frame whose dimensions no camera produces: a
    /// desynchronized stream can decode into garbage geometry, and the
    /// renderer must never be handed a gigapixel to rasterize.
    private nonisolated static func plausible(_ image: UIImage) -> UIImage? {
        let size = image.size
        guard size.width >= 1, size.height >= 1, size.width <= 8192, size.height <= 8192
        else { return nil }
        return image
    }

    private nonisolated func clearInFlight(_ sourceId: UInt8) {
        decoderLock.lock()
        inFlight.remove(sourceId)
        decoderLock.unlock()
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
