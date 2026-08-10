import Foundation
import PilotageSituationCore

/// Replays one recorded flight beside the live map.
///
/// A radio, a transmitter in range, and weather in range all have to agree before the map
/// can be seen carrying traffic and weather. A recording removes all three and exercises
/// the same decoders, the same display policy, and the same renderer on the device itself.
///
/// This is not a flight source. It runs only when a reader opens a flight, it owns its own
/// decoders and its own display so nothing it produces can reach the live map, and while it
/// runs the client says the map is a replay.
@MainActor
final class SituationReplayRun {
    /// The flight being replayed.
    let flight: Flight
    /// Receptions handed to the decoders so far.
    private(set) var events: UInt64 = 0
    /// Track records the decoders produced.
    private(set) var trackRecords: UInt64 = 0
    /// Weather records the decoders produced.
    private(set) var weatherRecords: UInt64 = 0
    /// Records the display refused.
    private(set) var refusedRecords: UInt64 = 0
    /// Whether every reception has been read.
    private(set) var finished = false

    /// Decoders and display of this replay alone.
    ///
    /// Sharing the live sessions was the source of the trouble it replaced: the live
    /// decoders count reconnect generations a recording cannot supply, and replayed
    /// features outlive the run inside the live display.
    private let domain: RadioDomainSession
    private let session: PresentationSession
    private let lines: [String]

    init?(flight: Flight) {
        guard let text = try? String(contentsOf: flight.receptionURL, encoding: .utf8),
              let domain = try? RadioDomainSession() else { return nil }
        self.flight = flight
        self.domain = domain
        session = PresentationSession()
        lines = text.split(separator: "\n").map(String.init)
    }

    /// Push every reception through the decoders, handing each display batch to the caller.
    ///
    /// Elapsed time starts from the device clock and advances with the recording. The
    /// display policy never moves time backwards, so a replay based at zero would leave
    /// every feature far enough in the past to be retired at once. Wall-clock time stays
    /// the recording's, because an advisory states its validity in real time.
    func run(deviceMonotonicMicros: UInt64, apply: (DisplayBatch) -> Void) async {
        var utcMillis = flight.startedAtUtcMillis
        var monotonicMicros = deviceMonotonicMicros
        for line in lines {
            if Task.isCancelled { return }
            utcMillis += 100
            monotonicMicros &+= 100_000
            events &+= 1
            if let records = try? domain.acceptReceptionEvent(
                eventJson: line,
                reconnectGeneration: 1,
                utcMillis: utcMillis,
                monotonicMicros: monotonicMicros
            ) {
                accept(records, nowMicros: monotonicMicros, apply: apply)
            }
            if events % 200 == 0 {
                await Task.yield()
            }
        }
        finished = true
    }

    private func accept(
        _ records: RadioRecordBatch,
        nowMicros: UInt64,
        apply: (DisplayBatch) -> Void
    ) {
        for record in records.trackRecords {
            trackRecords &+= 1
            // A refusal is counted rather than thrown. One unreadable record must not end a
            // replay, and a silent skip would read as a recording that carried nothing.
            if let batch = try? session.acceptTrackRecord(
                recordJson: record,
                nowMicros: nowMicros
            ) {
                apply(batch)
            } else {
                refusedRecords &+= 1
            }
        }
        for record in records.weatherRecords {
            weatherRecords &+= 1
            if let batch = try? session.acceptWeatherRecord(
                recordJson: record,
                nowMicros: nowMicros
            ) {
                apply(batch)
            } else {
                refusedRecords &+= 1
            }
        }
    }
}
