import Foundation

/// One recorded flight the client can replay.
///
/// A flight is a folder of what was observed during one run. Today it holds the reception
/// file the harness writes, which already carries nearby traffic and every FIS-B product
/// the receiver heard. An ownship track and the navigation-data cycle that was current
/// belong to the same flight and are not recorded yet, so the type names the recording
/// rather than the reception file: adding those later does not change what a flight is.
struct Flight: Identifiable, Equatable {
    /// Stable identity, the recording's file name.
    var id: String { receptionFileName }
    /// File the receptions were written to.
    let receptionFileName: String
    /// Where the recording sits.
    let receptionURL: URL
    /// Instant the recording started.
    let startedAtUtcMillis: Int64
    /// Size of the recording in bytes.
    let bytes: Int

    /// Text a list row shows.
    var title: String {
        let seconds = TimeInterval(startedAtUtcMillis) / 1_000
        let date = Date(timeIntervalSince1970: seconds)
        return Flight.rowFormatter.string(from: date)
    }

    /// Size, for a reader deciding which recording to open.
    var subtitle: String {
        let megabytes = Double(bytes) / (1_024 * 1_024)
        return String(format: "%.1f MB", megabytes)
    }

    private static let rowFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        formatter.timeZone = TimeZone(identifier: "UTC")
        return formatter
    }()
}

/// Finds the recordings the container holds.
enum FlightsLibrary {
    /// Prefix a recording carries, followed by the instant it started.
    static let filePrefix = "replay-receptions-"

    /// Every recording in the application container, newest first.
    static func flights() -> [Flight] {
        guard let documents = FileManager.default
            .urls(for: .documentDirectory, in: .userDomainMask)
            .first else { return [] }
        let manager = FileManager.default
        let names = (try? manager.contentsOfDirectory(atPath: documents.path)) ?? []
        return names
            .filter { $0.hasPrefix(filePrefix) && $0.hasSuffix(".ndjson") }
            .compactMap { name -> Flight? in
                guard let started = startedAtUtcMillis(fromFileName: name) else { return nil }
                let url = documents.appendingPathComponent(name)
                let size = (try? manager.attributesOfItem(atPath: url.path)[.size]) as? Int
                return Flight(
                    receptionFileName: name,
                    receptionURL: url,
                    startedAtUtcMillis: started,
                    bytes: size ?? 0
                )
            }
            .sorted { $0.startedAtUtcMillis > $1.startedAtUtcMillis }
    }

    /// Remove one recording from the container.
    ///
    /// The container is the only copy: a recording is written here and read from here, so
    /// this is a deletion and not an unlink from a list.
    @discardableResult
    static func delete(_ flight: Flight) -> Bool {
        (try? FileManager.default.removeItem(at: flight.receptionURL)) != nil
    }

    /// Read `replay-receptions-YYYY-MM-DDTHH-MM-SSZ.ndjson`.
    ///
    /// A recording carries no wall clock inside it, and an advisory states its validity in
    /// real time, so the instant in the name is what lets a replay place its products.
    static func startedAtUtcMillis(fromFileName name: String) -> Int64? {
        let stamp = name
            .replacingOccurrences(of: filePrefix, with: "")
            .replacingOccurrences(of: ".ndjson", with: "")
        guard stamp.hasSuffix("Z") else { return nil }
        let parts = String(stamp.dropLast()).split(separator: "T")
        guard parts.count == 2 else { return nil }
        let date = parts[0].split(separator: "-").compactMap { Int64($0) }
        let time = parts[1].split(separator: "-").compactMap { Int64($0) }
        guard date.count == 3, time.count == 3 else { return nil }
        let days = daysFromCivil(year: date[0], month: date[1], day: date[2])
        return (days * 86_400 + time[0] * 3_600 + time[1] * 60 + time[2]) * 1_000
    }

    /// Days between the Unix epoch and one proleptic Gregorian date.
    private static func daysFromCivil(year: Int64, month: Int64, day: Int64) -> Int64 {
        let year = month <= 2 ? year - 1 : year
        let era = (year >= 0 ? year : year - 399) / 400
        let yearOfEra = year - era * 400
        let monthTerm = month > 2 ? month - 3 : month + 9
        let dayOfYear = (153 * monthTerm + 2) / 5 + day - 1
        let dayOfEra = yearOfEra * 365 + yearOfEra / 4 - yearOfEra / 100 + dayOfYear
        return era * 146_097 + dayOfEra - 719_468
    }
}
