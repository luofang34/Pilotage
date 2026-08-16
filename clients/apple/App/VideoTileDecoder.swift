import CoreGraphics
import Foundation
import UIKit
import VideoToolbox

/// Decodes one video source's frames into images the tile can show.
///
/// The platform owns the decoder (ADR-0037): MJPG decodes through
/// ImageIO, H264 through VideoToolbox. Everything runs off the main
/// actor; the published image is the only thing that crosses back.
final class VideoTileDecoder: @unchecked Sendable {
    private let lock = NSLock()
    private var session: VTDecompressionSession?
    private var format: CMVideoFormatDescription?
    private var parameterSets: [[UInt8]] = []

    /// Decodes one encoded payload, or nothing when the codec cannot be
    /// decoded yet (an H264 stream before its parameter sets arrive).
    func decode(codec: String, payload: [UInt8]) -> UIImage? {
        switch codec {
        case "MJPG":
            return UIImage(data: Data(payload))
        case "H264":
            return decodeH264(payload)
        default:
            return nil
        }
    }

    private func decodeH264(_ payload: [UInt8]) -> UIImage? {
        lock.lock()
        defer { lock.unlock() }
        var pixel: CVImageBuffer?
        for nalu in Self.annexBUnits(payload) {
            guard let first = nalu.first else { continue }
            switch first & 0x1F {
            case 7, 8:
                collectParameterSet(nalu)
            case 5, 1:
                guard prepareSession() else { continue }
                if let decoded = decodeUnit(nalu) {
                    pixel = decoded
                }
            default:
                continue
            }
        }
        guard let pixel else { return nil }
        var image: CGImage?
        VTCreateCGImageFromCVPixelBuffer(pixel as! CVPixelBuffer, options: nil, imageOut: &image)
        return image.map { UIImage(cgImage: $0) }
    }

    private func collectParameterSet(_ nalu: [UInt8]) {
        if !parameterSets.contains(nalu) {
            parameterSets.append(nalu)
            // New parameter sets describe a new stream shape; the old
            // session must not decode against them.
            session = nil
            format = nil
        }
    }

    private func prepareSession() -> Bool {
        if session != nil { return true }
        let sps = parameterSets.first { $0.first.map { $0 & 0x1F } == 7 }
        let pps = parameterSets.first { $0.first.map { $0 & 0x1F } == 8 }
        guard let sps, let pps else { return false }
        var created: CMVideoFormatDescription?
        let status = sps.withUnsafeBufferPointer { spsPtr in
            pps.withUnsafeBufferPointer { ppsPtr in
                CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    allocator: kCFAllocatorDefault,
                    parameterSetCount: 2,
                    parameterSetPointers: [spsPtr.baseAddress!, ppsPtr.baseAddress!],
                    parameterSetSizes: [sps.count, pps.count],
                    nalUnitHeaderLength: 4,
                    formatDescriptionOut: &created
                )
            }
        }
        guard status == noErr, let created else { return false }
        format = created
        var newSession: VTDecompressionSession?
        guard VTDecompressionSessionCreate(
            allocator: kCFAllocatorDefault,
            formatDescription: created,
            decoderSpecification: nil,
            imageBufferAttributes: nil,
            outputCallback: nil,
            decompressionSessionOut: &newSession
        ) == noErr, let newSession else { return false }
        session = newSession
        return true
    }

    private func decodeUnit(_ nalu: [UInt8]) -> CVImageBuffer? {
        guard let session, let format else { return nil }
        var avcc = withUnsafeBytes(of: UInt32(nalu.count).bigEndian) { Array($0) }
        avcc.append(contentsOf: nalu)
        var block: CMBlockBuffer?
        guard CMBlockBufferCreateWithMemoryBlock(
            allocator: kCFAllocatorDefault,
            memoryBlock: nil,
            blockLength: avcc.count,
            blockAllocator: nil,
            customBlockSource: nil,
            offsetToData: 0,
            dataLength: avcc.count,
            flags: 0,
            blockBufferOut: &block
        ) == noErr, let block else { return nil }
        guard CMBlockBufferReplaceDataBytes(
            with: avcc, blockBuffer: block, offsetIntoDestination: 0, dataLength: avcc.count
        ) == noErr else { return nil }
        var sample: CMSampleBuffer?
        var length = avcc.count
        guard CMSampleBufferCreateReady(
            allocator: kCFAllocatorDefault,
            dataBuffer: block,
            formatDescription: format,
            sampleCount: 1,
            sampleTimingEntryCount: 0,
            sampleTimingArray: nil,
            sampleSizeEntryCount: 1,
            sampleSizeArray: &length,
            sampleBufferOut: &sample
        ) == noErr, let sample else { return nil }
        var decoded: CVImageBuffer?
        VTDecompressionSessionDecodeFrame(
            session, sampleBuffer: sample, flags: [], infoFlagsOut: nil
        ) { _, _, buffer, _, _ in
            decoded = buffer
        }
        VTDecompressionSessionWaitForAsynchronousFrames(session)
        return decoded
    }

    /// Splits an Annex B stream into NAL units, tolerating 3- and 4-byte
    /// start codes.
    static func annexBUnits(_ bytes: [UInt8]) -> [[UInt8]] {
        var starts: [Int] = []
        var index = 0
        while index + 3 <= bytes.count {
            if bytes[index] == 0, bytes[index + 1] == 0 {
                if bytes[index + 2] == 1 {
                    starts.append(index + 3)
                    index += 3
                    continue
                }
                if index + 4 <= bytes.count, bytes[index + 2] == 0, bytes[index + 3] == 1 {
                    starts.append(index + 4)
                    index += 4
                    continue
                }
            }
            index += 1
        }
        guard !starts.isEmpty else { return bytes.isEmpty ? [] : [bytes] }
        var units: [[UInt8]] = []
        for (position, start) in starts.enumerated() {
            let end = position + 1 < starts.count
                ? max(starts[position + 1] - 3, start)
                : bytes.count
            var unitEnd = end
            // A 4-byte start code leaves one extra zero before the next
            // unit; trim trailing zeros that belong to the next start code.
            while unitEnd > start, position + 1 < starts.count, bytes[unitEnd - 1] == 0 {
                unitEnd -= 1
            }
            if start < unitEnd {
                units.append(Array(bytes[start..<unitEnd]))
            }
        }
        return units
    }
}
