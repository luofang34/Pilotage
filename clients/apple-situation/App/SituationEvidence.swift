import Foundation
import PilotageRadioSource
import PilotageSituationCore

/// What the client is drawing, in a form a workstation can collect over the wire.
///
/// A photograph of the screen is the only other way to answer "does the map show traffic
/// at altitude and weather as areas". A photograph needs a person in front of the iPad,
/// it captures whatever else is on the display, and it cannot be compared between runs.
/// This file states the same facts as text.
struct SituationEvidence: Codable, Equatable {
    /// Whether the driver extension the application ships is enabled.
    var driverEnabled: Bool?
    /// Whether the terrain archive is in the bundle.
    var terrainArchiveAvailable: Bool
    /// Whether this build compiled against the terrain renderer.
    var terrainRendererBuild: Bool
    /// Whether the style asks the renderer for a terrain surface.
    var terrainStyleRequested: Bool
    /// Effective state of radio reception.
    var sourceAvailability: String
    /// Reception state for each attached receiver, by band.
    var receivers: [String: String]
    /// Bytes the driver completed, by band.
    var completedBytes: [String: UInt64]
    /// Events the portable decoders accepted, by band.
    var acceptedEvents: [String: UInt64]
    /// Inputs the portable decoders rejected, by band.
    var rejectedInputs: [String: UInt64]
    /// Why a band has no receiver, when the client knows.
    ///
    /// A band missing from `receivers` with no failure here was never attached. A band
    /// with a failure here is attached and unusable, which is the case a replug clears.
    var bandFailures: [String: String]
    /// Feature count for each application layer.
    var pointsByLayer: [String: Int]
    /// Shape count for each style.
    var shapesByStyle: [String: Int]
    /// Tracks that reported no position and cannot be placed.
    var positionlessTraffic: Int
    /// Tracks the client is holding, placed or not.
    var trackedAircraft: Int
    /// Shapes the renderer raises above the terrain surface.
    var extrudedShapes: Int
    /// Lowest floor among raised shapes, in metres.
    var lowestBaseM: Double?
    /// Highest ceiling among raised shapes, in metres.
    var highestTopM: Double?
    /// Layer control state, by layer identity.
    var layerSourceStates: [String: String]
    /// First error the client reported, if any.
    var errorMessage: String?
}

extension SituationEvidence {
    /// Read the evidence out of one display batch and its surrounding state.
    init(
        batch: DisplayBatch?,
        radioSource: RadioSourceSnapshot,
        driverEnabled: Bool?,
        terrainArchiveAvailable: Bool,
        errorMessage: String?
    ) {
        self.driverEnabled = driverEnabled
        self.terrainArchiveAvailable = terrainArchiveAvailable
#if PILOTAGE_MAPLIBRE_TERRAIN
        terrainRendererBuild = true
        terrainStyleRequested = terrainArchiveAvailable
#else
        terrainRendererBuild = false
        terrainStyleRequested = false
#endif
        receivers = Dictionary(
            radioSource.receivers.map { ("\($0.band)", "\($0.availability)") },
            uniquingKeysWith: { first, _ in first }
        )
        completedBytes = Self.byBand(radioSource) { $0.diagnostics.completedBytes }
        acceptedEvents = Self.byBand(radioSource) { $0.diagnostics.acceptedEvents }
        rejectedInputs = Self.byBand(radioSource) { $0.diagnostics.rejectedInputs }
        bandFailures = Dictionary(
            radioSource.bandFailures.map { ("\($0.id)", $0.detail) },
            uniquingKeysWith: { first, _ in first }
        )
        sourceAvailability = "\(radioSource.availability)"
        let extruded = Self.extrudedStyles(in: batch)
        pointsByLayer = Self.tally(batch?.points ?? [], by: \.layerId)
        shapesByStyle = Self.tally(batch?.shapes ?? [], by: \.styleId)
        positionlessTraffic = batch?.positionlessTraffic.count ?? 0
        trackedAircraft = batch?.trafficDetails.count ?? 0
        let raised = (batch?.shapes ?? []).filter { extruded.contains($0.styleId) }
        extrudedShapes = raised.count
        lowestBaseM = raised.compactMap(\.baseAboveTerrainM).min()
        highestTopM = raised.compactMap(\.topAboveTerrainM).max()
        layerSourceStates = Dictionary(
            (batch?.layers ?? []).map { ($0.id, $0.sourceStateLabel) },
            uniquingKeysWith: { first, _ in first }
        )
        self.errorMessage = errorMessage
    }

    /// Read one counter for each attached receiver.
    ///
    /// Bytes that arrive with no feature on the map separate "nothing is transmitting"
    /// from "the decode path dropped it", and those need different work.
    private static func byBand(
        _ source: RadioSourceSnapshot,
        _ counter: (RadioReceiver) -> UInt64
    ) -> [String: UInt64] {
        Dictionary(
            source.receivers.map { ("\($0.band)", counter($0)) },
            uniquingKeysWith: { first, _ in first }
        )
    }

    private static func extrudedStyles(in batch: DisplayBatch?) -> Set<String> {
        Set((batch?.shapeStyles ?? []).filter(\.extruded).map(\.id))
    }

    private static func tally<Element>(
        _ elements: [Element],
        by key: KeyPath<Element, String>
    ) -> [String: Int] {
        var counts: [String: Int] = [:]
        for element in elements {
            counts[element[keyPath: key], default: 0] += 1
        }
        return counts
    }
}

/// Writes the evidence file the collection script reads.
///
/// The write is skipped when nothing a reader cares about changed, because a display that
/// updates many times each second would otherwise rewrite the file at that rate.
@MainActor
final class SituationEvidenceWriter {
    static let fileName = "situation-evidence.json"

    private var last: SituationEvidence?
    private let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }()

    func record(_ evidence: SituationEvidence) {
        guard evidence != last else { return }
        last = evidence
        guard let url = Self.destination(), let data = try? encoder.encode(evidence) else {
            return
        }
        try? data.write(to: url, options: .atomic)
    }

    private static func destination() -> URL? {
        FileManager.default
            .urls(for: .documentDirectory, in: .userDomainMask)
            .first?
            .appendingPathComponent(fileName)
    }
}
