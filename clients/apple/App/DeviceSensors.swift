// What this tablet can answer about itself: where it is, and which way it
// points. Kept apart from the model that ranks sources, because reading a
// sensor and deciding which reading the map believes are different jobs.

import CoreLocation
import Foundation
import UIKit

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
