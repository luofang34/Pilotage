#include "framing.hpp"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

#include <cerrno>
#include <cstring>
#include <string>
#include <utility>

namespace pilotage::bridge {

namespace {

// Max envelope size we will accept from the peer. A raw 320x240 RGB frame is
// ~230 KB; this cap leaves generous headroom while rejecting a corrupt length
// prefix that would otherwise demand an unbounded allocation.
constexpr std::size_t kMaxEnvelopeBytes = 16 * 1024 * 1024;

// Bounded outbound queue: at most this many droppable (camera-frame) payloads
// may sit pending. Odometry and other control-critical payloads bypass this cap
// so a slow peer's frame backlog can never starve them; frames beyond the cap
// are dropped rather than growing memory or back-pressuring the gz thread. A
// small depth keeps latency low while absorbing a brief send stall.
constexpr std::size_t kMaxPendingDroppable = 2;

// Blocking send timeout. A wedged peer that never drains its receive buffer
// would otherwise park the writer thread forever; on timeout the send fails and
// the connection is treated as dead.
constexpr time_t kSendTimeoutSeconds = 5;

// Appends a base-128 varint encoding of value to out (protobuf length prefix).
void AppendVarint(std::vector<std::uint8_t> &out, std::uint64_t value) {
  while (value >= 0x80) {
    out.push_back(static_cast<std::uint8_t>((value & 0x7F) | 0x80));
    value >>= 7;
  }
  out.push_back(static_cast<std::uint8_t>(value));
}

}  // namespace

BridgeConnection::BridgeConnection(int fd)
    : state_(std::make_unique<WriterState>()) {
  state_->fd = fd;
  WriterState *state = state_.get();
  writer_thread_ = std::thread([state] { state->WriterLoop(); });
}

std::optional<BridgeConnection> BridgeConnection::Connect(std::uint16_t port,
                                                          std::string &error_out) {
  int fd = ::socket(AF_INET, SOCK_STREAM, 0);
  if (fd < 0) {
    error_out = std::string("socket() failed: ") + std::strerror(errno);
    return std::nullopt;
  }

  sockaddr_in addr{};
  addr.sin_family = AF_INET;
  addr.sin_port = htons(port);
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);

  if (::connect(fd, reinterpret_cast<sockaddr *>(&addr), sizeof(addr)) != 0) {
    error_out = std::string("connect() to 127.0.0.1:") + std::to_string(port) +
                " failed: " + std::strerror(errno);
    ::close(fd);
    return std::nullopt;
  }

  timeval send_timeout{};
  send_timeout.tv_sec = kSendTimeoutSeconds;
  send_timeout.tv_usec = 0;
  if (::setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &send_timeout,
                   sizeof(send_timeout)) != 0) {
    error_out = std::string("setsockopt(SO_SNDTIMEO) failed: ") +
                std::strerror(errno);
    ::close(fd);
    return std::nullopt;
  }

  return BridgeConnection(fd);
}

BridgeConnection::BridgeConnection(BridgeConnection &&other) noexcept
    : state_(std::move(other.state_)),
      writer_thread_(std::move(other.writer_thread_)) {}

BridgeConnection &BridgeConnection::operator=(BridgeConnection &&other) noexcept {
  if (this != &other) {
    Shutdown();
    if (writer_thread_.joinable()) {
      writer_thread_.join();
    }
    state_ = std::move(other.state_);
    writer_thread_ = std::move(other.writer_thread_);
  }
  return *this;
}

BridgeConnection::~BridgeConnection() {
  Shutdown();
  if (writer_thread_.joinable()) {
    writer_thread_.join();
  }
  if (state_ && state_->fd >= 0) {
    ::close(state_->fd);
    state_->fd = -1;
  }
}

void BridgeConnection::Shutdown() {
  if (!state_) {
    return;
  }
  {
    std::lock_guard<std::mutex> guard(state_->queue_mutex);
    state_->stop = true;
  }
  state_->queue_cv.notify_all();
  int fd = state_->fd;
  if (fd >= 0) {
    // SHUT_RDWR unblocks any thread parked in recv/send; the fd is closed by
    // the destructor so the parent's poll detects our exit.
    ::shutdown(fd, SHUT_RDWR);
  }
}

bool BridgeConnection::WriterState::WriteAll(const std::uint8_t *data,
                                             std::size_t len) {
  std::size_t sent = 0;
  while (sent < len) {
    ssize_t n = ::send(fd, data + sent, len - sent, 0);
    if (n > 0) {
      sent += static_cast<std::size_t>(n);
      continue;
    }
    if (n < 0 && errno == EINTR) {
      continue;
    }
    // EAGAIN/EWOULDBLOCK here is the SO_SNDTIMEO firing on a wedged peer: treat
    // it as a fatal disconnect rather than spinning.
    return false;
  }
  return true;
}

void BridgeConnection::WriterState::WriterLoop() {
  for (;;) {
    QueuedFrame frame;
    {
      std::unique_lock<std::mutex> lock(queue_mutex);
      queue_cv.wait(lock, [this] { return stop || !queue.empty(); });
      if (stop) {
        // Shutdown requested: abandon any pending frames; the socket is being
        // torn down and further sends would fail anyway.
        return;
      }
      frame = std::move(queue.front());
      queue.pop_front();
      if (frame.droppable && pending_droppable > 0) {
        --pending_droppable;
      }
    }
    if (!WriteAll(frame.bytes.data(), frame.bytes.size())) {
      // Send failed (peer dead or wedged): stop draining and shut the read side
      // so a reader loop parked in recv observes the same disconnect promptly.
      {
        std::lock_guard<std::mutex> lock(queue_mutex);
        stop = true;
        queue.clear();
        pending_droppable = 0;
      }
      if (fd >= 0) {
        ::shutdown(fd, SHUT_RD);
      }
      return;
    }
  }
}

bool BridgeConnection::WriteEnvelope(
    const pilotage::bridge::v1::BridgeEnvelope &envelope, bool droppable) {
  if (!state_) {
    return false;
  }

  const std::size_t body_size = envelope.ByteSizeLong();
  if (body_size > kMaxEnvelopeBytes) {
    return false;
  }

  std::string body;
  if (!envelope.SerializeToString(&body)) {
    return false;
  }

  std::vector<std::uint8_t> frame;
  frame.reserve(body.size() + 10);
  AppendVarint(frame, static_cast<std::uint64_t>(body.size()));
  frame.insert(frame.end(), body.begin(), body.end());

  {
    std::lock_guard<std::mutex> guard(state_->queue_mutex);
    if (state_->stop) {
      return false;
    }
    if (droppable && state_->pending_droppable >= kMaxPendingDroppable) {
      // Queue congested: drop this best-effort frame so the producer thread
      // returns immediately instead of parking behind a slow peer.
      return true;
    }
    if (droppable) {
      ++state_->pending_droppable;
    }
    state_->queue.push_back(QueuedFrame{std::move(frame), droppable});
  }
  state_->queue_cv.notify_one();
  return true;
}

bool BridgeConnection::ReadAll(std::uint8_t *data, std::size_t len) {
  const int fd = state_ ? state_->fd : -1;
  std::size_t got = 0;
  while (got < len) {
    ssize_t n = ::recv(fd, data + got, len - got, 0);
    if (n > 0) {
      got += static_cast<std::size_t>(n);
      continue;
    }
    if (n == 0) {
      return false;  // orderly EOF
    }
    if (errno == EINTR) {
      continue;
    }
    return false;
  }
  return true;
}

bool BridgeConnection::ReadEnvelope(
    pilotage::bridge::v1::BridgeEnvelope &envelope_out) {
  if (!state_) {
    return false;
  }
  // Decode a varint length prefix one byte at a time so we never over-read
  // into the next frame's body.
  std::uint64_t body_size = 0;
  int shift = 0;
  while (true) {
    std::uint8_t byte = 0;
    if (!ReadAll(&byte, 1)) {
      return false;
    }
    body_size |= static_cast<std::uint64_t>(byte & 0x7F) << shift;
    if ((byte & 0x80) == 0) {
      break;
    }
    shift += 7;
    if (shift >= 64) {
      return false;  // malformed varint
    }
  }

  if (body_size > kMaxEnvelopeBytes) {
    return false;
  }

  auto &read_buffer = state_->read_buffer;
  read_buffer.resize(static_cast<std::size_t>(body_size));
  if (body_size > 0 && !ReadAll(read_buffer.data(), read_buffer.size())) {
    return false;
  }

  return envelope_out.ParseFromArray(read_buffer.data(),
                                     static_cast<int>(read_buffer.size()));
}

}  // namespace pilotage::bridge
