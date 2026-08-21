import Foundation
import Testing

@testable import PilotageCore

private let device = AppleHIDCharacterizationCapture.Device(
    vendorID: 0x1209,
    productID: 0x4f54,
    product: "RadioMaster Pocket"
)
private let sourceAxes = [
    AppleHIDCharacterizationCapture.SourceAxis(
        sourceIndex: 0,
        minimum: 0,
        maximum: 2047,
        neutralPosition: "centered"
    ),
    AppleHIDCharacterizationCapture.SourceAxis(
        sourceIndex: 1,
        minimum: 0,
        maximum: 2047,
        neutralPosition: "centered"
    ),
]
private let instanceID = "synthetic-fixture-1"
private let contractDigest = String(repeating: "a", count: 64)

private func sampler() throws -> AppleHIDCharacterizationFixtureSampler {
    try .init(
        device: device,
        deviceInstanceID: instanceID,
        sourceContractDigest: contractDigest,
        sourceAxes: sourceAxes
    )
}

@Test("Apple HID sampling uses the portable evidence schema")
func appleHIDSamplingUsesPortableSchema() throws {
    var sampler = try sampler()
    try sampler.beginIdle()
    try sampler.record(
        deviceInstanceID: instanceID,
        isConnected: true,
        axes: [1024, 1024],
        observedAtUs: 1_000,
        sourceAtUs: 900,
        reportHex: nil
    )
    try sampler.record(
        deviceInstanceID: instanceID,
        isConnected: true,
        axes: [1025, 1024],
        observedAtUs: 5_000,
        sourceAtUs: 4_900,
        reportHex: nil
    )
    try sampler.endSegment()
    try sampler.beginMovement(logical: "roll")
    try sampler.record(
        deviceInstanceID: instanceID,
        isConnected: true,
        axes: [2047, 1024],
        observedAtUs: 9_000,
        sourceAtUs: 8_900,
        reportHex: nil
    )
    try sampler.endSegment()
    let capture = try sampler.finish()

    #expect(capture.source == "synthetic")
    #expect(capture.timestampSource == "source")
    #expect(capture.timingObservation == "injected_samples")
    #expect(capture.deviceInstanceID == instanceID)
    #expect(capture.sourceContractDigest == contractDigest)
    #expect(capture.deadzoneEvidence.status == "unknown")
    #expect(capture.deadzoneEvidence.method == "unmeasured")
    #expect(capture.deadzoneEvidence.sampleCount == 0)
    #expect(capture.segments[1].action.logical == "roll")

    let json = try JSONSerialization.jsonObject(with: JSONEncoder().encode(capture))
    let object = try #require(json as? [String: Any])
    #expect(object["schema_version"] as? Int == 1)
    #expect(object["timing_observation"] as? String == "injected_samples")
    #expect(object["source_contract_digest"] as? String == contractDigest)
}

@Test("Apple HID sampling rejects changed devices, axes, and timestamps")
func appleHIDSamplingRejectsChangedInputs() throws {
    var sampler = try sampler()
    #expect(throws: AppleHIDCharacterizationFixtureSampler.SamplingError.noOpenSegment) {
        try sampler.record(
            deviceInstanceID: instanceID,
            isConnected: true,
            axes: [0, 0],
            observedAtUs: 1,
            sourceAtUs: nil,
            reportHex: nil
        )
    }
    try sampler.beginIdle()
    #expect(throws: AppleHIDCharacterizationFixtureSampler.SamplingError.deviceDisconnected) {
        try sampler.record(
            deviceInstanceID: instanceID,
            isConnected: false,
            axes: [0, 0],
            observedAtUs: 1,
            sourceAtUs: nil,
            reportHex: nil
        )
    }
    #expect(throws: AppleHIDCharacterizationFixtureSampler.SamplingError.deviceInstanceChanged) {
        try sampler.record(
            deviceInstanceID: "reconnected-handle",
            isConnected: true,
            axes: [0, 0],
            observedAtUs: 1,
            sourceAtUs: nil,
            reportHex: nil
        )
    }
    #expect(throws: AppleHIDCharacterizationFixtureSampler.SamplingError.invalidAxes) {
        try sampler.record(
            deviceInstanceID: instanceID,
            isConnected: true,
            axes: [0],
            observedAtUs: 1,
            sourceAtUs: nil,
            reportHex: nil
        )
    }
    try sampler.record(
        deviceInstanceID: instanceID,
        isConnected: true,
        axes: [0, 0],
        observedAtUs: 2,
        sourceAtUs: nil,
        reportHex: nil
    )
    #expect(throws: AppleHIDCharacterizationFixtureSampler.SamplingError.nonMonotonicTimestamp) {
        try sampler.record(
            deviceInstanceID: instanceID,
            isConnected: true,
            axes: [0, 0],
            observedAtUs: 2,
            sourceAtUs: nil,
            reportHex: nil
        )
    }
}

@Test("Apple fixture sampling enforces shared string and artifact limits")
func appleHIDSamplingEnforcesSharedLimits() throws {
    let oversizedDevice = AppleHIDCharacterizationCapture.Device(
        vendorID: 0x1209,
        productID: 0x4f54,
        product: String(repeating: "é", count: 129)
    )
    #expect(throws: AppleHIDCharacterizationFixtureSampler.SamplingError.invalidProductName) {
        _ = try AppleHIDCharacterizationFixtureSampler(
            device: oversizedDevice,
            deviceInstanceID: instanceID,
            sourceContractDigest: contractDigest,
            sourceAxes: sourceAxes
        )
    }
    var sampler = try sampler()
    #expect(throws: AppleHIDCharacterizationFixtureSampler.SamplingError.invalidLogicalName) {
        try sampler.beginMovement(logical: String(repeating: "x", count: 65))
    }
    try sampler.beginIdle()
    #expect(throws: AppleHIDCharacterizationFixtureSampler.SamplingError.invalidReport) {
        try sampler.record(
            deviceInstanceID: instanceID,
            isConnected: true,
            axes: [0, 0],
            observedAtUs: 1,
            sourceAtUs: nil,
            reportHex: String(repeating: "a", count: 8_193)
        )
    }
    try sampler.record(
        deviceInstanceID: instanceID,
        isConnected: true,
        axes: [0, 0],
        observedAtUs: 1,
        sourceAtUs: nil,
        reportHex: nil
    )
    try sampler.endSegment()
    let capture = try sampler.finish()
    let encoded = try capture.encodedJSON()
    #expect(try capture.encodedJSON(maximumBytes: encoded.count) == encoded)
    #expect(throws: AppleHIDCharacterizationCapture.ArtifactError.encodedSizeLimit) {
        try capture.encodedJSON(maximumBytes: encoded.count - 1)
    }
}

@Test("Apple and Rust use the same synthetic capture bytes")
func appleHIDSamplingProducesGoldenCapture() throws {
    let physical = try captureFixture("synthetic-capture.json")
    let expected = try captureFixture("apple-capture.json")
    var sampler = try AppleHIDCharacterizationFixtureSampler(
        device: physical.device,
        deviceInstanceID: "apple-hid-synthetic-1",
        sourceContractDigest: physical.sourceContractDigest,
        sourceAxes: physical.sourceAxes
    )

    for segment in physical.segments {
        if segment.action.kind == "idle" {
            try sampler.beginIdle()
        } else {
            try sampler.beginMovement(logical: try #require(segment.action.logical))
        }
        for sequence in segment.startSequence ... segment.endSequence {
            let sample = physical.samples[Int(sequence)]
            try sampler.record(
                deviceInstanceID: "apple-hid-synthetic-1",
                isConnected: true,
                axes: sample.axes,
                observedAtUs: sample.observedAtUs,
                sourceAtUs: nil,
                reportHex: nil
            )
        }
        try sampler.endSegment()
    }

    let capture = try sampler.finish()
    #expect(capture == expected)
    #expect(try capture.encodedJSON() == Data(contentsOf: fixtureURL("apple-capture.json")))
}

private func captureFixture(_ name: String) throws -> AppleHIDCharacterizationCapture {
    try JSONDecoder().decode(
        AppleHIDCharacterizationCapture.self,
        from: Data(contentsOf: fixtureURL(name))
    )
}

private func fixtureURL(_ name: String) -> URL {
    var directory = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    for _ in 0 ..< 6 {
        directory.deleteLastPathComponent()
    }
    return directory.appending(path: "tools/hid-probe/fixtures/\(name)")
}
