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
    private static let maximumArtifactBytes = 64 * 1024 * 1024
    /// The schema version.
    public let schemaVersion: UInt32
    /// The USB device identity.
    public let device: Device
    /// A token for one source connection or one synthetic fixture instance.
    public let deviceInstanceID: String
    /// The sampling source name.
    public let source: String
    /// The selected timing clock.
    public let timestampSource: String
    /// The event represented by each timing sample.
    public let timingObservation: String
    /// Platform dead-zone evidence.
    public let deadzoneEvidence: DeadzoneEvidence
    /// SHA-256 of the exact source-axis contract.
    public let sourceContractDigest: String
    /// Trusted source-unit ranges for decoded axes.
    public let sourceAxes: [SourceAxis]
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

    /// Evidence about platform dead-zone shaping.
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

    /// One trusted source-axis range and neutral position.
    public struct SourceAxis: Codable, Equatable, Sendable {
        /// The index in each decoded report.
        public let sourceIndex: Int
        /// The smallest source value.
        public let minimum: Float
        /// The largest source value.
        public let maximum: Float
        /// `centered`, `minimum`, or `maximum`.
        public let neutralPosition: String

        enum CodingKeys: String, CodingKey {
            case sourceIndex = "source_index"
            case minimum
            case maximum
            case neutralPosition = "neutral_position"
        }

        /// Creates one trusted source-axis contract entry.
        public init(sourceIndex: Int, minimum: Float, maximum: Float, neutralPosition: String) {
            self.sourceIndex = sourceIndex
            self.minimum = minimum
            self.maximum = maximum
            self.neutralPosition = neutralPosition
        }
    }

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case device
        case deviceInstanceID = "device_instance_id"
        case source
        case timestampSource = "timestamp_source"
        case timingObservation = "timing_observation"
        case deadzoneEvidence = "deadzone_evidence"
        case sourceContractDigest = "source_contract_digest"
        case sourceAxes = "source_axes"
        case samples
        case segments
    }

    /// Encodes one deterministic JSON artifact for digest and promotion input.
    public func encodedJSON() throws -> Data {
        try encodedJSON(maximumBytes: Self.maximumArtifactBytes)
    }

    func encodedJSON(maximumBytes: Int) throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        var data = try encoder.encode(self)
        data.append(0x0A)
        guard data.count <= maximumBytes else { throw ArtifactError.encodedSizeLimit }
        return data
    }

    /// Errors from portable artifact encoding.
    public enum ArtifactError: Error, Equatable {
        /// The encoded capture exceeds the shared artifact limit.
        case encodedSizeLimit
    }
}

/// Collects caller-supplied samples for Apple schema interoperability tests.
///
/// The result uses synthetic provenance. Promotion rejects this provenance.
public struct AppleHIDCharacterizationFixtureSampler: Sendable {
    private static let maximumSamples = 1_000_000
    private static let maximumSegments = 65
    private static let maximumProductNameBytes = 256
    private static let maximumLogicalNameBytes = 64
    private let device: AppleHIDCharacterizationCapture.Device
    private let deviceInstanceID: String
    private let sourceContractDigest: String
    private let sourceAxes: [AppleHIDCharacterizationCapture.SourceAxis]
    private var samples: [HIDCharacterizationSample] = []
    private var segments: [HIDCharacterizationSegment] = []
    private var openAction: HIDCharacterizationSegment.Action?
    private var openStart: UInt64 = 0

    /// Creates a sampler bound to one synthetic device instance and source contract.
    public init(
        device: AppleHIDCharacterizationCapture.Device,
        deviceInstanceID: String,
        sourceContractDigest: String,
        sourceAxes: [AppleHIDCharacterizationCapture.SourceAxis]
    ) throws {
        guard device.product.map({ !$0.isEmpty && $0.utf8.count <= Self.maximumProductNameBytes })
            ?? true
        else { throw SamplingError.invalidProductName }
        guard !deviceInstanceID.isEmpty, deviceInstanceID.utf8.count <= 256 else {
            throw SamplingError.invalidDeviceInstance
        }
        guard Self.isDigest(sourceContractDigest) else {
            throw SamplingError.invalidSourceContract
        }
        guard !sourceAxes.isEmpty, sourceAxes.count <= 64 else {
            throw SamplingError.invalidSourceContract
        }
        for (index, axis) in sourceAxes.enumerated() {
            guard axis.sourceIndex == index, axis.minimum.isFinite, axis.maximum.isFinite,
                  axis.minimum < axis.maximum,
                  ["centered", "minimum", "maximum"].contains(axis.neutralPosition)
            else { throw SamplingError.invalidSourceContract }
        }
        self.device = device
        self.deviceInstanceID = deviceInstanceID
        self.sourceContractDigest = sourceContractDigest
        self.sourceAxes = sourceAxes
    }

    /// Starts the idle segment.
    public mutating func beginIdle() throws {
        try begin(.init(kind: "idle", logical: nil, positiveFirst: nil))
    }

    /// Starts one named positive-first movement segment.
    public mutating func beginMovement(logical: String) throws {
        guard !logical.isEmpty, logical.utf8.count <= Self.maximumLogicalNameBytes else {
            throw SamplingError.invalidLogicalName
        }
        try begin(.init(kind: "movement", logical: logical, positiveFirst: true))
    }

    /// Records one caller-supplied sample.
    public mutating func record(
        deviceInstanceID: String,
        isConnected: Bool,
        axes: [Float],
        observedAtUs: UInt64,
        sourceAtUs: UInt64?,
        reportHex: String?
    ) throws {
        guard openAction != nil else { throw SamplingError.noOpenSegment }
        guard isConnected else { throw SamplingError.deviceDisconnected }
        guard deviceInstanceID == self.deviceInstanceID else {
            throw SamplingError.deviceInstanceChanged
        }
        guard samples.count < Self.maximumSamples else { throw SamplingError.sampleLimit }
        guard reportHex == nil else {
            throw SamplingError.invalidReport
        }
        guard axes.count == sourceAxes.count,
              axes.enumerated().allSatisfy({ index, value in
                  value.isFinite && value >= sourceAxes[index].minimum
                      && value <= sourceAxes[index].maximum
              })
        else {
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
        guard segments.count < Self.maximumSegments else { throw SamplingError.segmentLimit }
        segments.append(
            .init(action: action, startSequence: openStart, endSequence: UInt64(samples.count - 1))
        )
        openAction = nil
    }

    /// Creates a portable capture for the bound device.
    public func finish() throws -> AppleHIDCharacterizationCapture {
        guard openAction == nil else { throw SamplingError.segmentStillOpen }
        guard !samples.isEmpty, !segments.isEmpty else { throw SamplingError.emptyCapture }
        let sourceClock = samples.allSatisfy { $0.sourceAtUs != nil }
        let capture = AppleHIDCharacterizationCapture(
            schemaVersion: 1,
            device: device,
            deviceInstanceID: deviceInstanceID,
            source: "synthetic",
            timestampSource: sourceClock ? "source" : "arrival",
            timingObservation: "injected_samples",
            deadzoneEvidence: .init(
                status: "unknown",
                method: "unmeasured",
                sampleCount: 0
            ),
            sourceContractDigest: sourceContractDigest,
            sourceAxes: sourceAxes,
            samples: samples,
            segments: segments
        )
        _ = try capture.encodedJSON()
        return capture
    }

    private mutating func begin(_ action: HIDCharacterizationSegment.Action) throws {
        guard openAction == nil else { throw SamplingError.segmentStillOpen }
        openAction = action
        openStart = UInt64(samples.count)
    }

    private static func isDigest(_ value: String) -> Bool {
        value.utf8.count == 64 && value.utf8.allSatisfy { byte in
            (48 ... 57).contains(byte) || (97 ... 102).contains(byte)
        }
    }

    /// Errors from capture sequencing and sample validation.
    public enum SamplingError: Error, Equatable {
        /// A segment is already open.
        case segmentStillOpen
        /// No segment is open.
        case noOpenSegment
        /// A segment has no report.
        case emptySegment
        /// A movement name is empty or too large.
        case invalidLogicalName
        /// The synthetic sampler received a raw report.
        case invalidReport
        /// The device product name is empty or too large.
        case invalidProductName
        /// Axis values are empty or non-finite.
        case invalidAxes
        /// The caller declared that the synthetic device is disconnected.
        case deviceDisconnected
        /// A sample came from a different synthetic device instance.
        case deviceInstanceChanged
        /// The device instance token is invalid.
        case invalidDeviceInstance
        /// The source-axis contract is invalid.
        case invalidSourceContract
        /// The capture reached its report limit.
        case sampleLimit
        /// The capture reached its segment limit.
        case segmentLimit
        /// Arrival timestamps do not increase.
        case nonMonotonicTimestamp
        /// The capture has no report or segment.
        case emptyCapture
    }
}
