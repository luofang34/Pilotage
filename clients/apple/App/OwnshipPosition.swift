import CoreLocation
import Foundation
import UIKit
import PilotageCore

/// Where a position for the aircraft came from.
///
/// The two are usually the same and are not always. An iPad in a bag on the back seat
/// still reports a position, and a panel or a receiver that hears the aircraft's own
/// transmission reports where the aircraft is. A display that swaps between them without
/// saying so is worse than one that offers only the tablet.
enum OwnshipSource: String, Equatable, Sendable {
    /// This device's own receiver.
    case device
    /// The aircraft, over the radio link.
    case aircraft

    /// How the source reads to someone deciding whether to trust the mark.
    var title: String {
        switch self {
        case .device: "This iPad"
        case .aircraft: "Aircraft"
        }
    }
}

/// One position for the aircraft.
struct OwnshipFix: Equatable, Sendable {
    /// WGS84 latitude in degrees.
    let latitudeDegrees: Double
    /// WGS84 longitude in degrees.
    let longitudeDegrees: Double
    /// Direction of travel in degrees from true north, when the source reports one.
    let courseDegrees: Double?
    /// Where the position came from.
    let source: OwnshipSource

    var coordinate: CLLocationCoordinate2D {
        CLLocationCoordinate2D(latitude: latitudeDegrees, longitude: longitudeDegrees)
    }
}

/// Whether the client may ask this device for a position, and what it answered.
enum DeviceLocationAuthorisation: Equatable, Sendable {
    /// The reader has not been asked.
    case undetermined
    /// The reader said yes.
    case granted
    /// The reader said no, or the device forbids it.
    case denied
}

/// Reads a position from the device.
///
/// Not every iPad has a satellite receiver: the cellular models do and the others do not.
/// This reports nothing on a tablet that cannot answer, which is the case the aircraft
/// source exists for, so the control is driven by whether a position exists rather than by
/// which hardware is present.
@MainActor
final class DeviceLocationProvider: NSObject, CLLocationManagerDelegate {
    /// Receives each position the device reports.
    var onFix: ((OwnshipFix) -> Void)?
    /// Receives the reader's answer to the permission request.
    var onAuthorisation: ((DeviceLocationAuthorisation) -> Void)?

    /// What the platform answers right now, without waiting to be told.
    ///
    /// The published copy of this is only filled once the provider has been started, so a
    /// decision about whether to start cannot be made from it without asking the question
    /// the answer depends on.
    var authorisation: DeviceLocationAuthorisation {
        Self.reading(manager.authorizationStatus)
    }

    private let manager = CLLocationManager()
    private var isRunning = false

    /// Whether the device's own location service is switched on.
    nonisolated static var servicesEnabled: Bool {
        CLLocationManager.locationServicesEnabled()
    }

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyBest
        // An aircraft moves far enough that a metre of filtering costs nothing and saves a
        // wake-up for every jitter of the fix.
        manager.distanceFilter = 5
    }

    /// Ask for a position, requesting permission the first time.
    func start() {
        guard !isRunning else { return }
        isRunning = true
        report(manager.authorizationStatus)
        switch manager.authorizationStatus {
        case .notDetermined:
            manager.requestWhenInUseAuthorization()
        case .authorizedWhenInUse, .authorizedAlways:
            manager.startUpdatingLocation()
        default:
            break
        }
    }

    /// Stop asking. A map nobody is looking at does not need a position.
    func stop() {
        isRunning = false
        manager.stopUpdatingLocation()
    }

    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        let status = manager.authorizationStatus
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.report(status)
            if status == .authorizedWhenInUse || status == .authorizedAlways, self.isRunning {
                self.manager.startUpdatingLocation()
            }
        }
    }

    nonisolated func locationManager(
        _ manager: CLLocationManager,
        didUpdateLocations locations: [CLLocation]
    ) {
        guard let location = locations.last else { return }
        let fix = OwnshipFix(
            latitudeDegrees: location.coordinate.latitude,
            longitudeDegrees: location.coordinate.longitude,
            courseDegrees: location.course >= 0 ? location.course : nil,
            source: .device
        )
        Task { @MainActor [weak self] in
            self?.onFix?(fix)
        }
    }

    nonisolated func locationManager(
        _ manager: CLLocationManager,
        didFailWithError error: Error
    ) {
        // A failure to fix is not a failure of the client. The aircraft source may still
        // answer, and the control follows whether a position exists.
    }

    private func report(_ status: CLAuthorizationStatus) {
        onAuthorisation?(Self.reading(status))
    }

    /// Read one platform answer, in one place, so the pushed and the pulled answer agree.
    private static func reading(_ status: CLAuthorizationStatus) -> DeviceLocationAuthorisation {
        switch status {
        case .notDetermined: .undetermined
        case .authorizedWhenInUse, .authorizedAlways: .granted
        default: .denied
        }
    }
}

/// The position the map may centre on, from whichever source has one.
///
/// The aircraft is preferred when it reports, because that is where the aircraft is; the
/// device answers otherwise. A tablet with no satellite receiver still gets the control as
/// soon as a panel or a receiver supplies a position, so the control follows whether a
/// position exists rather than which hardware is in the bag.
@MainActor
final class OwnshipModel: ObservableObject {
    /// The position to centre on, or nothing when no source has one.
    @Published private(set) var fix: OwnshipFix?
    /// What this device answered when asked for a position.
    @Published private(set) var deviceAuthorisation: DeviceLocationAuthorisation = .undetermined
    /// Whether the control that centres the map is worth offering.
    ///
    /// Offered unless the answer is already no. A fix comes and goes indoors, and a reader
    /// who has not been asked for permission yet has not refused: pressing the control is
    /// how they are asked. Hiding it until a fix exists means the reader never gets to ask
    /// for one, which is the state this replaced.
    var canLocate: Bool {
        fix != nil || deviceAuthorisation != .denied
    }

    /// Whether the device is willing to locate at all.
    ///
    /// An application can hold permission on a device whose location service is switched
    /// off entirely. Without this the two states look the same from outside: permission
    /// granted and no position ever arriving.
    var deviceLocationEnabled: Bool { DeviceLocationProvider.servicesEnabled }

    /// Which way the aircraft is pointing, from the best source that has an answer.
    @Published private(set) var heading: HeadingFix?

    /// How closely the map is tied to the aircraft.
    ///
    /// Held here rather than in the view because a closure that reads it outlives the
    /// view value that created it, and a captured view value holds the mode as it was
    /// when the closure was made rather than as it is when the closure runs.
    @Published var follow: FollowMode = .idle

    private let device = DeviceLocationProvider()
    private let compass = DeviceHeadingProvider()
    private var deviceFix: OwnshipFix?
    private var aircraftFix: OwnshipFix?
    private var deviceHeading: HeadingFix?
    private var aircraftHeading: HeadingFix?

    init() {
        device.onFix = { [weak self] fix in
            self?.deviceFix = fix
            self?.resolve()
        }
        device.onAuthorisation = { [weak self] authorisation in
            self?.deviceAuthorisation = authorisation
            // Permission granted part way through a run is the same situation as
            // permission held at the start of one.
            self?.startIfPermitted()
        }
        compass.onHeading = { [weak self] heading in
            self?.deviceHeading = heading
            self?.resolveHeading()
        }
    }

    /// Take a heading reported by a panel or receiver in the aircraft.
    ///
    /// Nothing calls this yet. A GDL 90 source carries one, and it beats a tablet that may
    /// be lying on a seat.
    func observeAircraftHeading(_ heading: HeadingFix?) {
        aircraftHeading = heading
        resolveHeading()
    }

    private func resolveHeading() {
        // The aircraft knows better than the tablet. Failing both, course over the ground
        // is the last answer, and it is marked as such because it is not heading.
        let course = fix?.courseDegrees.map {
            HeadingFix(trueDegrees: $0, source: .courseOverGround)
        }
        let next = aircraftHeading ?? deviceHeading ?? course
        guard next != heading else { return }
        heading = next
    }

    /// Begin asking this device for a position.
    func start() {
        device.start()
        compass.start()
    }

    /// Take the orientation again, because a tablet is turned while it is being read.
    func refreshOrientation() {
        compass.refreshOrientation()
    }

    /// Stop asking.
    func stop() {
        device.stop()
        compass.stop()
    }

    /// Start locating when the reader has already agreed to it.
    ///
    /// A reader who granted permission on an earlier run has not withdrawn it by closing
    /// the application. Waiting for a press before asking the sensors anything means the
    /// first press reports no position and no heading, and the control appears broken at
    /// exactly the moment it is first used.
    func startIfPermitted() {
        guard device.authorisation == .granted else { return }
        start()
    }

    /// Ask for permission if it has not been asked for, and start locating.
    ///
    /// A reader pressing the control is the request. The map follows as soon as there is
    /// a position, which may be after the reader answers a prompt.
    func requestPositionIfNeeded() {
        start()
    }

    /// Take a position reported by the aircraft over the radio link.
    func observeAircraft(_ fix: OwnshipFix?) {
        aircraftFix = fix
        resolve()
    }

    private func resolve() {
        let next = aircraftFix ?? deviceFix
        guard next != fix else { return }
        fix = next
        resolveHeading()
    }
}

extension OwnshipFix {
    /// Read the aircraft's own return as a position to centre on.
    init(_ ownship: DisplayOwnship) {
        self.init(
            latitudeDegrees: ownship.coordinate.latitudeDeg,
            longitudeDegrees: ownship.coordinate.longitudeDeg,
            courseDegrees: ownship.courseDeg,
            source: .aircraft
        )
    }
}

/// How closely the map is tied to the aircraft.
///
/// Three states rather than two, because "centred" and "turning with me" are different
/// questions. North-up keeps the chart oriented the way it is printed; heading-up puts
/// what is ahead of the aircraft at the top of the screen, which is how most flying is
/// actually done. A reader has to be able to see which one they are in without moving the
/// map to find out.
enum FollowMode: Equatable, Sendable {
    /// The map goes where the reader puts it.
    case idle
    /// The map stays on the aircraft, north up.
    case centred
    /// The map stays on the aircraft and turns with it.
    case heading

    /// The next state when the control is pressed.
    var next: FollowMode {
        switch self {
        case .idle: .centred
        case .centred: .heading
        case .heading: .centred
        }
    }

    /// The mark that says which state this is.
    var symbol: String {
        switch self {
        case .idle: "location"
        case .centred: "location.fill"
        case .heading: "location.north.line.fill"
        }
    }

    /// What the state means, said in full.
    var label: String {
        switch self {
        case .idle: "Centre the map on the aircraft"
        case .centred: "Following the aircraft, turn the map with it"
        case .heading: "Turning with the aircraft, stop turning"
        }
    }

    /// Whether the map should stay on the aircraft.
    var followsPosition: Bool { self != .idle }
}

/// Which direction the aircraft is pointing, and where that came from.
///
/// Three references, and they are not the same number:
///
/// - magnetic heading is what a magnetometer reads;
/// - true heading is that corrected for magnetic variation, which the platform computes
///   from position and its own declination model;
/// - course over the ground is where the aircraft is going, which in wind is not where it
///   is pointing.
///
/// The map draws in true north, so a heading-up map wants true heading. A panel or a
/// GDL 90 source supplies a better one than a tablet on a yoke mount, and takes priority
/// when it arrives.
struct HeadingFix: Equatable, Sendable {
    /// Degrees clockwise from true north.
    let trueDegrees: Double
    /// Where the value came from.
    let source: Source

    enum Source: String, Equatable, Sendable {
        /// This device's magnetometer, corrected for variation by the platform.
        case deviceTrue
        /// This device's magnetometer, uncorrected because no correction was available.
        case deviceMagnetic
        /// Course over the ground, which is not heading.
        case courseOverGround
        /// A panel or receiver in the aircraft.
        case aircraft
    }
}

/// Reads which way the device is pointing.
///
/// Not every iPad has a magnetometer, so this reports nothing on a tablet that cannot
/// answer and the map falls back to course over the ground.
@MainActor
final class DeviceHeadingProvider: NSObject, CLLocationManagerDelegate {
    /// Receives each heading the device reports.
    var onHeading: ((HeadingFix) -> Void)?

    private let manager = CLLocationManager()
    private var isRunning = false


    /// Whether this device can report which way it is pointing.
    nonisolated static var available: Bool { CLLocationManager.headingAvailable() }

    override init() {
        super.init()
        manager.delegate = self
        // A degree of filtering keeps a map from shivering while the aircraft holds course.
        manager.headingFilter = 3
        // A tablet flown in landscape reads ninety degrees off if the platform is told it
        // is upright, and the reading is plausible enough that nobody notices it is wrong.
        applyOrientation()
    }

    /// Take the orientation again, because a tablet is turned while it is being read.
    func refreshOrientation() {
        applyOrientation()
    }

    /// Tell the platform which way the tablet is being held.
    ///
    /// The interface orientation rather than the device orientation, because the device
    /// answers `unknown` until it has been moved, and a tablet started flat on a table in
    /// landscape would be told it is upright and read ninety degrees off.
    private func applyOrientation() {
        // Landscape is named from opposite ends by the two enumerations: turning the
        // tablet left turns the content right, so the platform's interface orientation and
        // the location service's device orientation swap the two. Passing one straight
        // through as the other reads a hundred and eighty degrees off.
        manager.headingOrientation = switch Self.interfaceOrientation {
        case .landscapeLeft: .landscapeRight
        case .landscapeRight: .landscapeLeft
        case .portraitUpsideDown: .portraitUpsideDown
        default: .portrait
        }
    }

    /// Which way the window is drawn, which is which way the reader holds the tablet.
    private static var interfaceOrientation: UIInterfaceOrientation {
        UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first?
            .interfaceOrientation ?? .portrait
    }

    func start() {
        guard Self.available, !isRunning else { return }
        isRunning = true
        applyOrientation()
        manager.startUpdatingHeading()
    }

    func stop() {
        isRunning = false
        manager.stopUpdatingHeading()
    }

    /// Let the platform ask the reader to swing the tablet through a figure of eight.
    ///
    /// A magnetometer near a keyboard cover or a metal panel reports a negative accuracy,
    /// which is the platform saying it does not know which way it is pointing. Refusing
    /// the prompt leaves a compass that is present, started, and permanently silent, with
    /// nothing on screen to say why.
    nonisolated func locationManagerShouldDisplayHeadingCalibration(
        _ manager: CLLocationManager
    ) -> Bool {
        true
    }

    nonisolated func locationManager(
        _ manager: CLLocationManager,
        didUpdateHeading newHeading: CLHeading
    ) {
        // A negative accuracy means the reading is not trustworthy at all, and a negative
        // true heading means the platform had no variation to apply.
        guard newHeading.headingAccuracy >= 0 else { return }
        let fix: HeadingFix = newHeading.trueHeading >= 0
            ? HeadingFix(trueDegrees: newHeading.trueHeading, source: .deviceTrue)
            : HeadingFix(trueDegrees: newHeading.magneticHeading, source: .deviceMagnetic)
        Task { @MainActor [weak self] in
            self?.onHeading?(fix)
        }
    }
}
