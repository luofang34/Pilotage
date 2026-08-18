// pilotage-xplane-capture - X-Plane window capture video sidecar.
//
// Captures the X-Plane window with ScreenCaptureKit and streams RGB
// frames to the Pilotage session host as length-delimited
// pilotage.bridge.v1 BridgeEnvelope{ BridgeFrame } messages over the
// localhost TCP dial-back connection every Pilotage video sidecar uses.
//
// The first run asks macOS for the Screen Recording permission; the
// operator must grant it once in System Settings > Privacy & Security.
//
// Arguments:
//   --port <n>           host listener port to dial (required)
//   --vehicle <name>     accepted for sidecar-protocol compatibility; unused
//   --window-owner <s>   application name substring to capture (default "X-Plane")
//   --fps <n>            capture rate (default 24)
//   --max-width <n>      downscale bound, pixels (default 1280)
//
// Capture stamps are host-monotonic nanoseconds: a window capture has no
// access to the simulator's internal clock, and the consumer declares
// that domain (MeasurementClock::HostMonotonic) when stamping.

import CoreMedia
import Foundation
import ScreenCaptureKit

// MARK: - Arguments

struct Arguments {
    var port: UInt16 = 0
    var windowOwner = "X-Plane"
    var fps = 24
    var maxWidth = 1280

    static func parse() -> Arguments {
        var parsed = Arguments()
        var iterator = CommandLine.arguments.dropFirst().makeIterator()
        while let flag = iterator.next() {
            let value = iterator.next()
            switch flag {
            case "--port": parsed.port = UInt16(value ?? "") ?? 0
            case "--window-owner": parsed.windowOwner = value ?? parsed.windowOwner
            case "--fps": parsed.fps = max(1, Int(value ?? "") ?? parsed.fps)
            case "--max-width": parsed.maxWidth = max(64, Int(value ?? "") ?? parsed.maxWidth)
            case "--vehicle", "--camera-topic", "--gimbal-camera-topic":
                continue // sidecar-protocol compatibility; not applicable
            default:
                FileHandle.standardError.write(Data("unknown flag \(flag)\n".utf8))
                exit(2)
            }
        }
        if parsed.port == 0 {
            FileHandle.standardError.write(Data("usage: pilotage-xplane-capture --port <n>\n".utf8))
            exit(2)
        }
        return parsed
    }
}

// MARK: - Protobuf encoding (pilotage.bridge.v1, hand-encoded)

func putVarint(_ value: UInt64, into data: inout Data) {
    var remaining = value
    while remaining >= 0x80 {
        data.append(UInt8((remaining & 0x7F) | 0x80))
        remaining >>= 7
    }
    data.append(UInt8(remaining))
}

func putTag(field: UInt32, wire: UInt32, into data: inout Data) {
    putVarint(UInt64(field << 3 | wire), into: &data)
}

/// BridgeFrame{width=1,height=2,pixel_format=3,sim_time_ns=4,rgb=5,camera_id=6}
/// wrapped as BridgeEnvelope{frame=3}, length-delimited for the stream.
func encodeFrameEnvelope(width: UInt32, height: UInt32, simTimeNs: UInt64, rgb: Data) -> Data {
    var frame = Data(capacity: rgb.count + 64)
    putTag(field: 1, wire: 0, into: &frame)
    putVarint(UInt64(width), into: &frame)
    putTag(field: 2, wire: 0, into: &frame)
    putVarint(UInt64(height), into: &frame)
    putTag(field: 3, wire: 2, into: &frame)
    let format = Data("RGB_INT8".utf8)
    putVarint(UInt64(format.count), into: &frame)
    frame.append(format)
    putTag(field: 4, wire: 0, into: &frame)
    putVarint(simTimeNs, into: &frame)
    putTag(field: 5, wire: 2, into: &frame)
    putVarint(UInt64(rgb.count), into: &frame)
    frame.append(rgb)
    // camera_id 0 (FPV): proto3 default, still written for clarity.
    putTag(field: 6, wire: 0, into: &frame)
    putVarint(0, into: &frame)

    var envelope = Data(capacity: frame.count + 16)
    putTag(field: 3, wire: 2, into: &envelope)
    putVarint(UInt64(frame.count), into: &envelope)
    envelope.append(frame)

    var framed = Data(capacity: envelope.count + 8)
    putVarint(UInt64(envelope.count), into: &framed)
    framed.append(envelope)
    return framed
}

// MARK: - TCP dial-back

final class HostLink {
    private let handle: FileHandle
    private let queue = DispatchQueue(label: "host-link")
    private var writeInFlight = false

    init?(port: UInt16) {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { return nil }
        var address = sockaddr_in()
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian
        address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))
        let connected = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { raw in
                connect(fd, raw, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard connected == 0 else {
            close(fd)
            return nil
        }
        var noDelay: Int32 = 1
        setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &noDelay, socklen_t(4))
        handle = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
    }

    /// Latest-wins: a frame arriving while the previous write is still on
    /// the wire is dropped rather than queued, so a slow link never grows
    /// a frame backlog.
    func sendLatest(_ data: Data, onFatal: @escaping () -> Void) {
        queue.async {
            if self.writeInFlight { return }
            self.writeInFlight = true
            do {
                try self.handle.write(contentsOf: data)
            } catch {
                onFatal()
            }
            self.writeInFlight = false
        }
    }
}

// MARK: - Capture

final class FrameOutput: NSObject, SCStreamOutput {
    let link: HostLink
    init(link: HostLink) { self.link = link }

    func stream(_ stream: SCStream, didOutputSampleBuffer buffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, let pixels = buffer.imageBuffer else { return }
        guard CVPixelBufferLockBaseAddress(pixels, .readOnly) == kCVReturnSuccess else { return }
        defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(pixels) else { return }
        let width = CVPixelBufferGetWidth(pixels)
        let height = CVPixelBufferGetHeight(pixels)
        let stride = CVPixelBufferGetBytesPerRow(pixels)

        // BGRA -> tightly packed RGB.
        var rgb = Data(count: width * height * 3)
        rgb.withUnsafeMutableBytes { (out: UnsafeMutableRawBufferPointer) in
            let source = base.assumingMemoryBound(to: UInt8.self)
            let destination = out.baseAddress!.assumingMemoryBound(to: UInt8.self)
            for row in 0..<height {
                let sourceRow = source + row * stride
                let destinationRow = destination + row * width * 3
                for column in 0..<width {
                    destinationRow[column * 3] = sourceRow[column * 4 + 2]
                    destinationRow[column * 3 + 1] = sourceRow[column * 4 + 1]
                    destinationRow[column * 3 + 2] = sourceRow[column * 4]
                }
            }
        }

        let envelope = encodeFrameEnvelope(
            width: UInt32(width),
            height: UInt32(height),
            simTimeNs: DispatchTime.now().uptimeNanoseconds,
            rgb: rgb
        )
        link.sendLatest(envelope) {
            FileHandle.standardError.write(Data("host link closed; exiting\n".utf8))
            exit(0)
        }
    }
}

func findWindow(owner: String) async throws -> SCWindow? {
    let content = try await SCShareableContent.excludingDesktopWindows(
        false, onScreenWindowsOnly: true)
    return content.windows
        .filter { window in
            guard let app = window.owningApplication else { return false }
            return app.applicationName.contains(owner) && window.frame.width > 200
        }
        .max { lhs, rhs in
            lhs.frame.width * lhs.frame.height < rhs.frame.width * rhs.frame.height
        }
}

let arguments = Arguments.parse()

guard let link = HostLink(port: arguments.port) else {
    FileHandle.standardError.write(Data("cannot dial host on 127.0.0.1:\(arguments.port)\n".utf8))
    exit(1)
}

Task {
    do {
        guard let window = try await findWindow(owner: arguments.windowOwner) else {
            FileHandle.standardError.write(
                Data("no on-screen window owned by \(arguments.windowOwner)\n".utf8))
            exit(1)
        }
        let filter = SCContentFilter(desktopIndependentWindow: window)
        let configuration = SCStreamConfiguration()
        let scale = min(1.0, Double(arguments.maxWidth) / Double(window.frame.width))
        configuration.width = Int(Double(window.frame.width) * scale)
        configuration.height = Int(Double(window.frame.height) * scale)
        configuration.pixelFormat = kCVPixelFormatType_32BGRA
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(arguments.fps))
        configuration.showsCursor = false

        let stream = SCStream(filter: filter, configuration: configuration, delegate: nil)
        let output = FrameOutput(link: link)
        try stream.addStreamOutput(
            output, type: .screen, sampleHandlerQueue: DispatchQueue(label: "capture"))
        try await stream.startCapture()
        FileHandle.standardError.write(
            Data("capturing \(configuration.width)x\(configuration.height) @\(arguments.fps)fps\n".utf8))
    } catch {
        FileHandle.standardError.write(Data("capture failed: \(error)\n".utf8))
        exit(1)
    }
}

RunLoop.main.run()
