import Foundation

/// A decoded Apple HID report for automatic device characterization.
public struct HIDCharacterizationSample: Codable, Equatable, Sendable {
    /// A capture-local sequence.
    public let sequence: UInt64
    /// Microseconds from capture start at bridge receipt.
    public let observedAtUs: UInt64
    /// Microseconds from the HID source clock, when available.
    public let sourceAtUs: UInt64?
    /// Axis values in raw HID units.
    public let axes: [Float]
    /// Raw report bytes in hexadecimal form, when available.
    public let reportHex: String?

    enum CodingKeys: String, CodingKey {
        case sequence
        case observedAtUs = "observed_at_us"
        case sourceAtUs = "source_at_us"
        case axes
        case reportHex = "report_hex"
    }
}

/// One guided capture segment.
public struct HIDCharacterizationSegment: Codable, Equatable, Sendable {
    /// The guided action.
    public let action: Action
    /// The first included sequence.
    public let startSequence: UInt64
    /// The last included sequence.
    public let endSequence: UInt64

    /// A guided operator action.
    public struct Action: Codable, Equatable, Sendable {
        /// `idle` or `movement`.
        public let kind: String
        /// The named control for a movement.
        public let logical: String?
        /// True when positive movement occurs first.
        public let positiveFirst: Bool?

        enum CodingKeys: String, CodingKey {
            case kind
            case logical
            case positiveFirst = "positive_first"
        }
    }

    enum CodingKeys: String, CodingKey {
        case action
        case startSequence = "start_sequence"
        case endSequence = "end_sequence"
    }
}

/// An Apple HID capture that uses the shared JSON schema.
public struct AppleHIDCharacterizationCapture: Codable, Equatable, Sendable {
    /// The schema version.
    public let schemaVersion: UInt32
    /// The USB device identity.
    public let device: Device
    /// The sampling source name.
    public let source: String
    /// The selected timing clock.
    public let timestampSource: String
    /// Platform dead-zone evidence.
    public let deadzoneEvidence: DeadzoneEvidence
    /// Decoded reports.
    public let samples: [HIDCharacterizationSample]
    /// Guided segments.
    public let segments: [HIDCharacterizationSegment]

    /// A USB device identity.
    public struct Device: Codable, Equatable, Sendable {
        /// USB vendor ID.
        public let vendorID: UInt16
        /// USB product ID.
        public let productID: UInt16
        /// Product name, when available.
        public let product: String?

        enum CodingKeys: String, CodingKey {
            case vendorID = "vendor_id"
            case productID = "product_id"
            case product
        }

        /// Creates a USB device identity.
        public init(vendorID: UInt16, productID: UInt16, product: String?) {
            self.vendorID = vendorID
            self.productID = productID
            self.product = product
        }
    }

    /// Evidence that raw HID reports have no platform dead zone.
    public struct DeadzoneEvidence: Codable, Equatable, Sendable {
        /// The measured status.
        public let status: String
        /// The measurement method.
        public let method: String
        /// The report count.
        public let sampleCount: UInt64

        enum CodingKeys: String, CodingKey {
            case status
            case method
            case sampleCount = "sample_count"
        }
    }

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case device
        case source
        case timestampSource = "timestamp_source"
        case deadzoneEvidence = "deadzone_evidence"
        case samples
        case segments
    }
}

/// Collects decoded Apple HID reports and guided segment evidence.
public struct AppleHIDCharacterizationSampler: Sendable {
    private var samples: [HIDCharacterizationSample] = []
    private var segments: [HIDCharacterizationSegment] = []
    private var openAction: HIDCharacterizationSegment.Action?
    private var openStart: UInt64 = 0

    /// Creates an empty sampler.
    public init() {}

    /// Starts the idle segment.
    public mutating func beginIdle() throws {
        try begin(.init(kind: "idle", logical: nil, positiveFirst: nil))
    }

    /// Starts one named positive-first movement segment.
    public mutating func beginMovement(logical: String) throws {
        guard !logical.isEmpty else { throw SamplingError.emptyLogicalName }
        try begin(.init(kind: "movement", logical: logical, positiveFirst: true))
    }

    /// Records one decoded raw HID report.
    public mutating func record(
        axes: [Float],
        observedAtUs: UInt64,
        sourceAtUs: UInt64?,
        reportHex: String?
    ) throws {
        guard openAction != nil else { throw SamplingError.noOpenSegment }
        guard !axes.isEmpty, axes.allSatisfy(\.isFinite) else {
            throw SamplingError.invalidAxes
        }
        guard samples.last.map({ observedAtUs > $0.observedAtUs }) ?? true else {
            throw SamplingError.nonMonotonicTimestamp
        }
        if let sourceAtUs, let previous = samples.last?.sourceAtUs, sourceAtUs <= previous {
            throw SamplingError.nonMonotonicTimestamp
        }
        samples.append(
            .init(
                sequence: UInt64(samples.count),
                observedAtUs: observedAtUs,
                sourceAtUs: sourceAtUs,
                axes: axes,
                reportHex: reportHex
            )
        )
    }

    /// Closes the current segment.
    public mutating func endSegment() throws {
        guard let action = openAction else { throw SamplingError.noOpenSegment }
        guard UInt64(samples.count) > openStart else { throw SamplingError.emptySegment }
        segments.append(
            .init(action: action, startSequence: openStart, endSequence: UInt64(samples.count - 1))
        )
        openAction = nil
    }

    /// Creates a portable capture for the device.
    public func finish(device: AppleHIDCharacterizationCapture.Device) throws
        -> AppleHIDCharacterizationCapture
    {
        guard openAction == nil else { throw SamplingError.segmentStillOpen }
        guard !samples.isEmpty, !segments.isEmpty else { throw SamplingError.emptyCapture }
        let sourceClock = samples.allSatisfy { $0.sourceAtUs != nil }
        return .init(
            schemaVersion: 1,
            device: device,
            source: "apple_hid",
            timestampSource: sourceClock ? "source" : "arrival",
            deadzoneEvidence: .init(
                status: "not_observed",
                method: "raw_hid_reports",
                sampleCount: UInt64(samples.count)
            ),
            samples: samples,
            segments: segments
        )
    }

    private mutating func begin(_ action: HIDCharacterizationSegment.Action) throws {
        guard openAction == nil else { throw SamplingError.segmentStillOpen }
        openAction = action
        openStart = UInt64(samples.count)
    }

    /// Errors from capture sequencing and sample validation.
    public enum SamplingError: Error, Equatable {
        /// A segment is already open.
        case segmentStillOpen
        /// No segment is open.
        case noOpenSegment
        /// A segment has no report.
        case emptySegment
        /// A movement has no name.
        case emptyLogicalName
        /// Axis values are empty or non-finite.
        case invalidAxes
        /// Arrival timestamps do not increase.
        case nonMonotonicTimestamp
        /// The capture has no report or segment.
        case emptyCapture
    }
}
