#include "link.h"

#include "XPLMProcessing.h"

#include <arpa/inet.h>
#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <unistd.h>

namespace pilotage_camera {
namespace {

/// Default port the host adapter listens on for this producer.
constexpr unsigned kDefaultPort = 45990;
/// Seconds between connection attempts while the host is absent.
constexpr double kRetryInterval = 2.0;
/// Outbound buffer ceiling: one frame plus headroom. A frame that does
/// not fit is dropped, never queued behind an older one.
constexpr std::size_t kMaxOutBytes = 8u * 1024u * 1024u;

void put_varint(std::uint64_t value, std::vector<std::uint8_t>* out) {
    while (value >= 0x80) {
        out->push_back(static_cast<std::uint8_t>((value & 0x7F) | 0x80));
        value >>= 7;
    }
    out->push_back(static_cast<std::uint8_t>(value));
}

void put_tag(std::uint32_t field, std::uint32_t wire,
             std::vector<std::uint8_t>* out) {
    put_varint((static_cast<std::uint64_t>(field) << 3) | wire, out);
}

/// Reads one base-128 varint; returns false when the buffer holds no
/// complete value yet.
bool read_varint(const std::uint8_t* bytes, std::size_t len,
                 std::size_t* offset, std::uint64_t* value) {
    std::uint64_t result = 0;
    std::uint32_t shift = 0;
    std::size_t index = *offset;
    while (index < len) {
        std::uint8_t byte = bytes[index++];
        result |= static_cast<std::uint64_t>(byte & 0x7F) << shift;
        if ((byte & 0x80) == 0) {
            *offset = index;
            *value = result;
            return true;
        }
        shift += 7;
        if (shift >= 64) {
            return false;
        }
    }
    return false;
}

float decode_float(const std::uint8_t* bytes) {
    float value = 0.0F;
    std::memcpy(&value, bytes, sizeof(value));
    return value;
}

/// Decodes a `BridgeCameraCommand` body into `out`.
void decode_camera_command(const std::uint8_t* body, std::size_t len,
                           CameraCommand* out) {
    std::size_t offset = 0;
    while (offset < len) {
        std::uint64_t tag = 0;
        if (!read_varint(body, len, &offset, &tag)) {
            return;
        }
        const std::uint32_t field = static_cast<std::uint32_t>(tag >> 3);
        const std::uint32_t wire = static_cast<std::uint32_t>(tag & 0x7);
        if (wire == 0) {
            std::uint64_t value = 0;
            if (!read_varint(body, len, &offset, &value)) {
                return;
            }
            if (field == 1) {
                out->mode = static_cast<std::uint32_t>(value);
            } else if (field == 4) {
                out->zoom_detent = static_cast<std::uint32_t>(value);
            }
        } else if (wire == 5) {
            if (offset + 4 > len) {
                return;
            }
            const float value = decode_float(body + offset);
            offset += 4;
            if (field == 2) {
                out->pan_rad = value;
            } else if (field == 3) {
                out->tilt_rad = value;
            }
        } else if (wire == 2) {
            std::uint64_t sub_len = 0;
            if (!read_varint(body, len, &offset, &sub_len)) {
                return;
            }
            offset += static_cast<std::size_t>(sub_len);
        } else {
            return;
        }
    }
}

}  // namespace

void HostLink::pump() {
    if (fd_ < 0) {
        const double now = XPLMGetElapsedTime();
        if (now < next_attempt_s_) {
            return;
        }
        next_attempt_s_ = now + kRetryInterval;
        const unsigned port =
            env_unsigned("PILOTAGE_XPLANE_CAMERA_PORT", kDefaultPort);
        int fd = ::socket(AF_INET, SOCK_STREAM, 0);
        if (fd < 0) {
            return;
        }
        sockaddr_in address{};
        address.sin_family = AF_INET;
        address.sin_port = htons(static_cast<std::uint16_t>(port));
        address.sin_addr.s_addr = inet_addr("127.0.0.1");
        if (::connect(fd, reinterpret_cast<sockaddr*>(&address),
                      sizeof(address)) != 0) {
            ::close(fd);
            return;
        }
        int flag = 1;
        ::setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &flag, sizeof(flag));
#ifdef SO_NOSIGPIPE
        ::setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &flag, sizeof(flag));
#endif
        ::fcntl(fd, F_SETFL, ::fcntl(fd, F_GETFL, 0) | O_NONBLOCK);
        fd_ = fd;
        out_.clear();
        out_sent_ = 0;
        in_.clear();
        log_line("host link up on 127.0.0.1:" + std::to_string(port));
        return;
    }

    std::uint8_t buffer[2048];
    for (;;) {
        ssize_t got = ::recv(fd_, buffer, sizeof(buffer), 0);
        if (got > 0) {
            in_.insert(in_.end(), buffer, buffer + got);
            continue;
        }
        if (got == 0) {
            log_line("host closed the link");
            close();
            return;
        }
        if (errno != EAGAIN && errno != EWOULDBLOCK) {
            log_line("host link read failed; reconnecting");
            close();
            return;
        }
        break;
    }

    // Length-delimited envelopes; only camera commands are expected.
    for (;;) {
        std::size_t offset = 0;
        std::uint64_t envelope_len = 0;
        if (!read_varint(in_.data(), in_.size(), &offset, &envelope_len)) {
            break;
        }
        if (in_.size() - offset < envelope_len) {
            break;
        }
        const std::uint8_t* envelope = in_.data() + offset;
        std::size_t inner = 0;
        std::uint64_t tag = 0;
        if (read_varint(envelope, static_cast<std::size_t>(envelope_len),
                        &inner, &tag) &&
            (tag >> 3) == 4 && (tag & 0x7) == 2) {
            std::uint64_t body_len = 0;
            if (read_varint(envelope, static_cast<std::size_t>(envelope_len),
                            &inner, &body_len) &&
                inner + body_len <= envelope_len) {
                command_ = CameraCommand{};
                command_.mode = static_cast<std::uint32_t>(state().mode());
                command_.zoom_detent =
                    static_cast<std::uint32_t>(state().zoom_detent());
                command_.pan_rad = state().pan_rad();
                command_.tilt_rad = state().tilt_rad();
                decode_camera_command(envelope + inner,
                                      static_cast<std::size_t>(body_len),
                                      &command_);
                command_pending_ = true;
            }
        }
        in_.erase(in_.begin(),
                  in_.begin() + static_cast<std::ptrdiff_t>(offset) +
                      static_cast<std::ptrdiff_t>(envelope_len));
    }
    flush();
}

const CameraCommand* HostLink::take_command() {
    if (!command_pending_) {
        return nullptr;
    }
    command_pending_ = false;
    return &command_;
}

void HostLink::send_frame(std::uint32_t width, std::uint32_t height,
                          std::uint64_t time_ns, std::uint32_t camera_id,
                          const std::uint8_t* rgb, std::size_t rgb_len) {
    if (fd_ < 0) {
        return;
    }
    if (out_.size() - out_sent_ > 0) {
        // A previous frame is still on the wire: drop this one so the
        // producer never trails the simulator.
        flush();
        return;
    }
    out_.clear();
    out_sent_ = 0;

    std::vector<std::uint8_t> frame;
    frame.reserve(rgb_len + 64);
    put_tag(1, 0, &frame);
    put_varint(width, &frame);
    put_tag(2, 0, &frame);
    put_varint(height, &frame);
    put_tag(3, 2, &frame);
    static const char kFormat[] = "RGB_INT8";
    put_varint(sizeof(kFormat) - 1, &frame);
    frame.insert(frame.end(), kFormat, kFormat + sizeof(kFormat) - 1);
    put_tag(4, 0, &frame);
    put_varint(time_ns, &frame);
    put_tag(5, 2, &frame);
    put_varint(rgb_len, &frame);
    frame.insert(frame.end(), rgb, rgb + rgb_len);
    put_tag(6, 0, &frame);
    put_varint(camera_id, &frame);

    std::vector<std::uint8_t> envelope;
    envelope.reserve(frame.size() + 16);
    put_tag(3, 2, &envelope);
    put_varint(frame.size(), &envelope);
    envelope.insert(envelope.end(), frame.begin(), frame.end());

    put_varint(envelope.size(), &out_);
    if (out_.size() + envelope.size() > kMaxOutBytes) {
        out_.clear();
        return;
    }
    out_.insert(out_.end(), envelope.begin(), envelope.end());
    flush();
}

void HostLink::flush() {
    while (fd_ >= 0 && out_sent_ < out_.size()) {
        ssize_t wrote = ::send(fd_, out_.data() + out_sent_,
                               out_.size() - out_sent_, 0);
        if (wrote > 0) {
            out_sent_ += static_cast<std::size_t>(wrote);
            continue;
        }
        if (wrote < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
            return;
        }
        log_line("host link write failed; reconnecting");
        close();
        return;
    }
    if (out_sent_ >= out_.size()) {
        out_.clear();
        out_sent_ = 0;
    }
}

void HostLink::close() {
    if (fd_ >= 0) {
        ::close(fd_);
        fd_ = -1;
    }
    out_.clear();
    out_sent_ = 0;
    in_.clear();
    command_pending_ = false;
}

HostLink& link() {
    static HostLink instance;
    return instance;
}

}  // namespace pilotage_camera
