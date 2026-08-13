import Foundation

/// Writes what the radios received, so a flight can be flown again on the ground.
///
/// The lines are the reception events exactly as they arrived, which is what a replay
/// reads back. Anything further down the chain is a decision this build made, and a
/// recording that stored decisions would replay this build rather than that flight.
/// Written from the radio's context and read from the screen's, so every field is taken
/// under a lock rather than assumed to be reached from one place.
final class FlightRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var recording = false
    private var written: UInt64 = 0
    private var target: URL?
    private var handle: FileHandle?

    /// Whether a recording is being written now.
    var isRecording: Bool { lock.withLock { recording } }
    /// How many reception events this recording holds.
    var events: UInt64 { lock.withLock { written } }
    /// The recording being written, so it can be named and closed.
    var url: URL? { lock.withLock { target } }

    /// Start a recording named for the instant it began.
    ///
    /// The name carries the start because a reception event has no wall clock in it, and
    /// a replay needs one to place a weather product that states its own validity.
    @discardableResult
    func start(now: Date = Date()) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard !recording else { return true }
        guard let documents = FileManager.default
            .urls(for: .documentDirectory, in: .userDomainMask)
            .first else { return false }
        let name = "\(FlightsLibrary.filePrefix)\(Self.stamp.string(from: now)).ndjson"
        let file = documents.appendingPathComponent(name)
        guard FileManager.default.createFile(atPath: file.path, contents: nil),
              let opened = try? FileHandle(forWritingTo: file) else { return false }
        handle = opened
        target = file
        written = 0
        recording = true
        return true
    }

    /// Take the reception events of one drain.
    ///
    /// Failures are counted by the absence of growth rather than thrown: a recording is a
    /// convenience, and a full disc must not take the map down with it.
    func append(_ lines: [String]) {
        guard !lines.isEmpty else { return }
        lock.lock()
        defer { lock.unlock() }
        guard recording, let handle else { return }
        let payload = lines.map { $0 + "\n" }.joined()
        guard let data = payload.data(using: .utf8) else { return }
        try? handle.write(contentsOf: data)
        written &+= UInt64(lines.count)
    }

    /// Close the recording and report what it holds.
    @discardableResult
    func stop() -> URL? {
        lock.lock()
        defer { lock.unlock() }
        guard recording else { return nil }
        try? handle?.close()
        handle = nil
        recording = false
        let finished = target
        target = nil
        return finished
    }

    /// The instant in a recording's name, in the form the library reads back.
    private static let stamp: DateFormatter = {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "yyyy-MM-dd'T'HH-mm-ss'Z'"
        return formatter
    }()
}
