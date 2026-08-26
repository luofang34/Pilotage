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
    /// The vehicle's own telemetry. Ranked above the surveillance shadow: a
    /// vehicle under this operator's control reports itself directly, and the
    /// shadow is a second-hand return of the same aircraft.
    private var vehicleFix: OwnshipFix?
    private var vehicleHeading: HeadingFix?
    /// When the vehicle last reported. A fix is only the vehicle's CURRENT
    /// position for as long as reports keep arriving.
    private var vehicleFixAt: Date?
    private var staleness: Timer?
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
        // Elapsed time is the only thing that can retire a report from a link
        // that has gone quiet, so it has to be driven by a clock rather than
        // by the next sample — there is no next sample.
        staleness = Timer.scheduledTimer(withTimeInterval: 1.0, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.expireStaleVehicleFix() }
        }
    }

    /// Take the vehicle's own report of where it is and which way it points.
    ///
    /// Position and heading arrive together because they come from one lane. Splitting
    /// them across two calls would let a position from the simulator's oracle sit under a
    /// heading from the estimator, and nothing on the mark could say so.
    func observeVehicle(_ vehicle: VehicleFix?) {
        guard let vehicle else {
            vehicleFix = nil
            vehicleHeading = nil
            vehicleFixAt = nil
            resolve()
            return
        }
        vehicleFixAt = Date()
        vehicleFix = OwnshipFix(
            latitudeDegrees: vehicle.latitudeDegrees,
            longitudeDegrees: vehicle.longitudeDegrees,
            courseDegrees: vehicle.courseDegrees,
            source: .aircraft
        )
        // A heading the lane did not state is not replaced by the course: they are
        // different quantities, and a crabbing aircraft would be drawn pointing the way
        // it travels rather than the way it faces.
        vehicleHeading = vehicle.headingDegrees.map {
            HeadingFix(trueDegrees: $0, source: .aircraft)
        }
        resolve()
    }

    /// Take a heading reported by a panel or receiver in the aircraft.
    ///
    /// A GDL 90 source carries one, and it beats a tablet that may be lying on a seat.
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
        let next = vehicleHeading ?? aircraftHeading ?? deviceHeading ?? course
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

    /// How long a vehicle report stands before the mark stops believing it.
    ///
    /// The same three seconds the browser withholds after, and for the same
    /// reason: a link that goes silent delivers no sample to decide it with,
    /// so nothing but elapsed time can retire the last one. Without this the
    /// vehicle's last position outranks this tablet's live receiver forever,
    /// and a reader who has walked away from a disconnected vehicle is drawn
    /// standing on it.
    static let vehicleFixStaleAfter: TimeInterval = 3.0

    /// Retires a vehicle report that has stopped arriving.
    private func expireStaleVehicleFix() {
        guard let reportedAt = vehicleFixAt else { return }
        guard Date().timeIntervalSince(reportedAt) > Self.vehicleFixStaleAfter else { return }
        observeVehicle(nil)
    }

    private func resolve() {
        // The vehicle's own report, then its surveillance shadow, then this
        // tablet. A reader without any of the three still gets a map.
        let next = vehicleFix ?? aircraftFix ?? deviceFix
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
