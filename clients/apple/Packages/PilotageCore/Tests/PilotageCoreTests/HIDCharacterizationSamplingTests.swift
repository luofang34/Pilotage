import Foundation
import Testing

@testable import PilotageCore

@Test("Apple HID sampling uses the portable evidence schema")
func appleHIDSamplingUsesPortableSchema() throws {
    var sampler = AppleHIDCharacterizationSampler()
    try sampler.beginIdle()
    try sampler.record(axes: [1024, 1024], observedAtUs: 1_000, sourceAtUs: 900, reportHex: "00")
    try sampler.record(axes: [1025, 1024], observedAtUs: 5_000, sourceAtUs: 4_900, reportHex: "01")
    try sampler.endSegment()
    try sampler.beginMovement(logical: "roll")
    try sampler.record(axes: [2047, 1024], observedAtUs: 9_000, sourceAtUs: 8_900, reportHex: "02")
    try sampler.endSegment()
    let capture = try sampler.finish(
        device: .init(vendorID: 0x1209, productID: 0x4f54, product: "RadioMaster Pocket")
    )

    #expect(capture.source == "apple_hid")
    #expect(capture.timestampSource == "source")
    #expect(capture.deadzoneEvidence.status == "not_observed")
    #expect(capture.deadzoneEvidence.sampleCount == 3)
    #expect(capture.segments[1].action.logical == "roll")

    let json = try JSONSerialization.jsonObject(with: JSONEncoder().encode(capture))
    let object = try #require(json as? [String: Any])
    #expect(object["schema_version"] as? Int == 1)
    #expect(object["timestamp_source"] as? String == "source")
    #expect(object["deadzone_evidence"] != nil)
}

@Test("Apple HID sampling rejects invalid segment and timestamp order")
func appleHIDSamplingRejectsInvalidOrder() throws {
    var sampler = AppleHIDCharacterizationSampler()
    #expect(throws: AppleHIDCharacterizationSampler.SamplingError.noOpenSegment) {
        try sampler.record(axes: [0], observedAtUs: 1, sourceAtUs: nil, reportHex: nil)
    }
    try sampler.beginIdle()
    try sampler.record(axes: [0], observedAtUs: 2, sourceAtUs: nil, reportHex: nil)
    #expect(throws: AppleHIDCharacterizationSampler.SamplingError.nonMonotonicTimestamp) {
        try sampler.record(axes: [0], observedAtUs: 2, sourceAtUs: nil, reportHex: nil)
    }
}
