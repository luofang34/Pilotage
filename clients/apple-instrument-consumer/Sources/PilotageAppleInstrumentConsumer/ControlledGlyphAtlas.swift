import CryptoKit
import Foundation
import IndicateAppleDisplay

/// The canonical glyph bytes and their recorded content hash.
public struct InstrumentGlyphAsset: Equatable, Sendable {
    public let canonical: [UInt8]
    public let recordedHash: [UInt8]

    public init(canonical: [UInt8], recordedHash: [UInt8]) {
        self.canonical = canonical
        self.recordedHash = recordedHash
    }
}

/// A typed refusal of an unverified glyph asset.
public enum InstrumentGlyphAssetError: Error, Equatable, Sendable {
    case recordedHashMismatch(actual: String, expected: String)
    case canonicalHashMismatch(actual: String, expected: String)
    case malformed
}

public extension InstrumentGlyphAssetError {
    var displayReason: DisplayReason { .glyphAsset }
}

/// The controlled glyph atlas used by the Apple scene backend.
public struct PilotageGlyphAtlas: GlyphAtlas {
    public let cellWidth: Int
    public let cellHeight: Int
    public let advance: Int
    private let glyphs: [UInt32: [UInt8]]

    public init(asset: InstrumentGlyphAsset) throws {
        try self.init(
            asset: asset,
            expectedHash: AppleInstrumentCompatibilityGate.glyphRecordedHash
        )
    }

    init(asset: InstrumentGlyphAsset, expectedHash: String) throws {
        let recorded = hex(asset.recordedHash)
        guard recorded == expectedHash else {
            throw InstrumentGlyphAssetError.recordedHashMismatch(
                actual: recorded,
                expected: expectedHash
            )
        }
        let computed = hex(Array(SHA256.hash(data: Data(asset.canonical))))
        guard computed == expectedHash else {
            throw InstrumentGlyphAssetError.canonicalHashMismatch(
                actual: computed,
                expected: expectedHash
            )
        }
        let parsed = try parseManifest(asset.canonical)
        cellWidth = parsed.cellWidth
        cellHeight = parsed.cellHeight
        advance = parsed.advance
        glyphs = parsed.glyphs
    }

    public func rows(for scalar: UInt32) -> [UInt8]? {
        glyphs[scalar]
    }
}

private struct ParsedManifest {
    let cellWidth: Int
    let cellHeight: Int
    let advance: Int
    let glyphs: [UInt32: [UInt8]]
}

private func parseManifest(_ bytes: [UInt8]) throws -> ParsedManifest {
    let headerLength = 8
    guard bytes.count >= headerLength,
          readUInt16(bytes, at: 0) == 1 else {
        throw InstrumentGlyphAssetError.malformed
    }
    let cellWidth = Int(bytes[2])
    let cellHeight = Int(bytes[3])
    let advance = Int(bytes[4])
    let baseline = Int(bytes[5])
    let count = Int(readUInt16(bytes, at: 6))
    guard (1 ... 8).contains(cellWidth),
          (1 ... 8).contains(cellHeight),
          advance > 0,
          baseline <= cellHeight else {
        throw InstrumentGlyphAssetError.malformed
    }
    let recordLength = 5 + cellHeight
    guard count <= (Int.max - headerLength) / recordLength,
          bytes.count == headerLength + count * recordLength else {
        throw InstrumentGlyphAssetError.malformed
    }
    let usedMask = UInt8((1 << cellWidth) - 1)
    var glyphs: [UInt32: [UInt8]] = [:]
    glyphs.reserveCapacity(count)
    for index in 0 ..< count {
        let offset = headerLength + index * recordLength
        let scalar = readUInt32(bytes, at: offset)
        let glyphAdvance = Int(bytes[offset + 4])
        let rows = Array(bytes[offset + 5 ..< offset + recordLength])
        guard Unicode.Scalar(scalar) != nil,
              glyphAdvance == advance,
              rows.allSatisfy({ $0 & ~usedMask == 0 }),
              glyphs.updateValue(rows, forKey: scalar) == nil else {
            throw InstrumentGlyphAssetError.malformed
        }
    }
    return ParsedManifest(
        cellWidth: cellWidth,
        cellHeight: cellHeight,
        advance: advance,
        glyphs: glyphs
    )
}

private func readUInt16(_ bytes: [UInt8], at offset: Int) -> UInt16 {
    UInt16(bytes[offset]) | UInt16(bytes[offset + 1]) << 8
}

private func readUInt32(_ bytes: [UInt8], at offset: Int) -> UInt32 {
    UInt32(bytes[offset])
        | UInt32(bytes[offset + 1]) << 8
        | UInt32(bytes[offset + 2]) << 16
        | UInt32(bytes[offset + 3]) << 24
}

func hex(_ bytes: [UInt8]) -> String {
    bytes.map { String(format: "%02x", $0) }.joined()
}
