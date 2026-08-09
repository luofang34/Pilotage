import CryptoKit
import Foundation
import IndicateAppleDisplay
import Testing
@testable import PilotageAppleInstrumentConsumer

private let oneGlyphManifest: [UInt8] = [
    1, 0, 1, 1, 1, 1, 1, 0,
    65, 0, 0, 0, 1, 1,
]

private func digest(_ bytes: [UInt8]) -> [UInt8] {
    Array(SHA256.hash(data: Data(bytes)))
}

@Test("A verified manifest supplies its complete glyph record")
func verifiedManifestSuppliesGlyph() throws {
    let hash = digest(oneGlyphManifest)
    let atlas = try PilotageGlyphAtlas(
        asset: InstrumentGlyphAsset(canonical: oneGlyphManifest, recordedHash: hash),
        expectedHash: hex(hash)
    )

    #expect(atlas.cellWidth == 1)
    #expect(atlas.cellHeight == 1)
    #expect(atlas.advance == 1)
    #expect(atlas.rows(for: 65) == [1])
    #expect(atlas.rows(for: 66) == nil)
}

@Test("A missing glyph maps to the glyph failure reason")
func missingGlyphFailsTyped() {
    guard let scalar = Unicode.Scalar(66) else {
        Issue.record("The test scalar is invalid")
        return
    }
    #expect(SceneRenderError.missingGlyph(scalar).displayReason == .glyphAsset)
}

@Test("A recorded glyph hash mismatch maps to the glyph failure reason")
func recordedHashMismatchFailsTyped() {
    do {
        _ = try PilotageGlyphAtlas(
            asset: InstrumentGlyphAsset(
                canonical: oneGlyphManifest,
                recordedHash: digest(oneGlyphManifest)
            )
        )
        Issue.record("The atlas accepted an unpinned recorded hash")
    } catch let error as InstrumentGlyphAssetError {
        guard case .recordedHashMismatch = error else {
            Issue.record("The atlas returned the wrong glyph error")
            return
        }
        #expect(error.displayReason == .glyphAsset)
    } catch {
        Issue.record("The atlas returned an untyped error: \(error)")
    }
}

@Test("A canonical glyph hash mismatch maps to the glyph failure reason")
func canonicalHashMismatchFailsTyped() {
    let pinnedHash: [UInt8] = [
        0x28, 0x1e, 0xef, 0x62, 0x29, 0xfe, 0xee, 0x41,
        0x7c, 0x70, 0x90, 0xd8, 0xc8, 0xea, 0x79, 0x48,
        0x9c, 0x01, 0x7c, 0xd1, 0xc0, 0x2f, 0xc7, 0x23,
        0x48, 0x76, 0xb2, 0xa6, 0x4a, 0x53, 0x21, 0x58,
    ]
    do {
        _ = try PilotageGlyphAtlas(
            asset: InstrumentGlyphAsset(
                canonical: oneGlyphManifest,
                recordedHash: pinnedHash
            )
        )
        Issue.record("The atlas accepted canonical bytes with the wrong hash")
    } catch let error as InstrumentGlyphAssetError {
        guard case .canonicalHashMismatch = error else {
            Issue.record("The atlas returned the wrong glyph error")
            return
        }
        #expect(error.displayReason == .glyphAsset)
    } catch {
        Issue.record("The atlas returned an untyped error: \(error)")
    }
}

@Test("A malformed verified glyph manifest fails typed")
func malformedManifestFailsTyped() {
    var malformed = oneGlyphManifest
    malformed[malformed.count - 1] = 2
    let hash = digest(malformed)
    do {
        _ = try PilotageGlyphAtlas(
            asset: InstrumentGlyphAsset(canonical: malformed, recordedHash: hash),
            expectedHash: hex(hash)
        )
        Issue.record("The atlas accepted a row outside the cell width")
    } catch let error as InstrumentGlyphAssetError {
        #expect(error == .malformed)
        #expect(error.displayReason == .glyphAsset)
    } catch {
        Issue.record("The atlas returned an untyped error: \(error)")
    }
}
