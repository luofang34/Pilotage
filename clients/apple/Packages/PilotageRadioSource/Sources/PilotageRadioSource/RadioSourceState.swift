/// A radio band that the situation client can receive.
public enum RadioBand: String, CaseIterable, Identifiable, Sendable {
    /// The 1090 MHz ADS-B band.
    case adsb1090
    /// The 978 MHz UAT band.
    case uat978

    /// Stable identity for list views.
    public var id: String { rawValue }
}

/// The current state of radio reception.
public enum RadioAvailability: Equatable, Sendable {
    /// The client checks the driver and attached receivers.
    case checking
    /// The host does not have permission to use the driver.
    case permissionDenied(String)
    /// The user disabled the driver.
    case driverDisabled
    /// No receiver is attached.
    case unplugged
    /// At least one receiver is ready.
    case ready
    /// At least one receiver streams data.
    case streaming
    /// The scene stopped radio reception.
    case suspended
    /// The USB path cannot supply sufficient power.
    case underpowered
    /// USB enumeration failed.
    case enumerationFailure(String)
    /// A USB endpoint failed.
    case endpointFailure(String)
    /// The system removed a receiver.
    case deviceRemoved
}

/// A receiver transport that AeroLink detected.
public enum RadioTransport: String, Identifiable, Sendable {
    /// A 1090 MHz receiver.
    case adsb1090
    /// A 978 MHz receiver with an FTDI interface.
    case uat978Ftdi
    /// A 978 MHz receiver with a USB CDC interface.
    case uat978Cdc

    /// Stable identity for list views.
    public var id: String { rawValue }
}

/// Counters for one receiver connection.
public struct RadioDiagnostics: Equatable, Sendable {
    /// Transfers that wait for the host.
    public var queueDepth: UInt32
    /// Maximum transfers that can wait for the host.
    public var queueCapacity: UInt32
    /// Transfers that the driver completed.
    public var completedTransfers: UInt64
    /// Bytes that the driver completed.
    public var completedBytes: UInt64
    /// Transfers that the driver dropped.
    public var droppedTransfers: UInt64
    /// Bytes that the driver dropped.
    public var droppedBytes: UInt64
    /// Driver input or output errors.
    public var ioErrors: UInt64
    /// Successful discovery generation.
    public var reconnectGeneration: UInt64
    /// Events that the portable decoders accepted.
    public var acceptedEvents: UInt64
    /// Inputs that the portable decoders rejected.
    public var rejectedInputs: UInt64
    /// Samples lost from the 1090 MHz stream.
    public var adsb1090GapSamples: UInt64
    /// Gaps found in the 978 MHz stream.
    public var uat978GapCount: UInt64
    /// Bytes discarded to resynchronize the 978 MHz stream.
    public var discardedUatBytes: UInt64
    /// Cycles that reached the host drain limit.
    public var drainLimitExhaustions: UInt64

    /// Make zero-valued counters.
    public init() {
        queueDepth = 0
        queueCapacity = 0
        completedTransfers = 0
        completedBytes = 0
        droppedTransfers = 0
        droppedBytes = 0
        ioErrors = 0
        reconnectGeneration = 0
        acceptedEvents = 0
        rejectedInputs = 0
        adsb1090GapSamples = 0
        uat978GapCount = 0
        discardedUatBytes = 0
        drainLimitExhaustions = 0
    }
}

/// The state and counters for one receiver connection.
public struct RadioReceiver: Equatable, Identifiable, Sendable {
    /// Stable transport identity.
    public let id: RadioTransport
    /// Radio band for this transport.
    public let band: RadioBand
    /// Current connection state.
    public let availability: RadioAvailability
    /// Current driver and decoder counters.
    public let diagnostics: RadioDiagnostics

    /// Make a receiver value.
    public init(
        id: RadioTransport,
        band: RadioBand,
        availability: RadioAvailability,
        diagnostics: RadioDiagnostics
    ) {
        self.id = id
        self.band = band
        self.availability = availability
        self.diagnostics = diagnostics
    }
}

/// A persistent failure for one radio band.
public struct RadioBandFailure: Equatable, Identifiable, Sendable {
    /// Failed band.
    public let id: RadioBand
    /// Failure detail.
    public let detail: String

    /// Make a band failure.
    public init(id: RadioBand, detail: String) {
        self.id = id
        self.detail = detail
    }
}

/// State that the application can present.
public struct RadioSourceSnapshot: Equatable, Sendable {
    /// Effective state of radio reception.
    public let availability: RadioAvailability
    /// Connected receivers.
    public let receivers: [RadioReceiver]
    /// Failures that do not stop a healthy sibling band.
    public let bandFailures: [RadioBandFailure]

    /// Make an application snapshot.
    public init(
        availability: RadioAvailability,
        receivers: [RadioReceiver],
        bandFailures: [RadioBandFailure]
    ) {
        self.availability = availability
        self.receivers = receivers
        self.bandFailures = bandFailures
    }
}

/// Persistent degraded state for independent radio bands.
public struct RadioDegradedState: Sendable {
    private var byBand: [RadioBand: RadioAvailability] = [:]
    private var unscoped: RadioAvailability?

    /// Make an empty degraded state.
    public init() {}

    /// Record a failure for one band.
    public mutating func record(_ availability: RadioAvailability, for band: RadioBand) {
        byBand[band] = availability
    }

    /// Record a failure that applies to the host process.
    public mutating func recordUnscoped(_ availability: RadioAvailability) {
        unscoped = availability
    }

    /// Clear the failure for one reconnected band.
    public mutating func clear(_ band: RadioBand) {
        byBand.removeValue(forKey: band)
    }

    /// Clear the host process failure after a clean scan.
    public mutating func clearUnscoped() {
        unscoped = nil
    }

    /// Clear all failures when reception stops.
    public mutating func clearAll() {
        byBand.removeAll()
        unscoped = nil
    }

    /// Select the state that the application must show.
    public func effectiveAvailability(
        active: RadioAvailability?,
        idle: RadioAvailability? = nil
    ) -> RadioAvailability? {
        if let unscoped, case .permissionDenied = unscoped {
            return unscoped
        }
        return active ?? unscoped ?? Self.highestPriority(Array(byBand.values)) ?? idle
    }

    /// Get persistent failures for receiver rows.
    public var bandFailures: [RadioBandFailure] {
        byBand.map { band, availability in
            RadioBandFailure(id: band, detail: Self.detail(for: availability))
        }.sorted { $0.id.rawValue < $1.id.rawValue }
    }

    private static func highestPriority(
        _ failures: [RadioAvailability]
    ) -> RadioAvailability? {
        for failure in failures {
            if case .permissionDenied = failure { return failure }
        }
        if failures.contains(.underpowered) { return .underpowered }
        for failure in failures {
            if case .enumerationFailure = failure { return failure }
        }
        for failure in failures {
            if case .endpointFailure = failure { return failure }
        }
        return failures.contains(.deviceRemoved) ? .deviceRemoved : nil
    }

    private static func detail(for availability: RadioAvailability) -> String {
        switch availability {
        case .permissionDenied(let detail): "Permission denied: \(detail)"
        case .underpowered: "USB path is underpowered"
        case .enumerationFailure(let detail): detail
        case .endpointFailure(let detail): detail
        case .deviceRemoved: "Receiver removed"
        default: "Receiver unavailable"
        }
    }
}

/// Test whether a scan can retire a host process failure.
public func scanRetiresProcessFailure(
    hadOpenFailures: Bool,
    hasScanError: Bool,
    hasReceiverFailures: Bool
) -> Bool {
    !hadOpenFailures && !hasScanError && !hasReceiverFailures
}

/// Keep a pending reconnect request or record a scan failure.
public func reconnectRequiredAfterScan(
    pending: Bool,
    hadOpenFailures: Bool,
    hasScanError: Bool,
    hasReceiverFailures: Bool
) -> Bool {
    pending || hadOpenFailures || hasScanError || hasReceiverFailures
}
