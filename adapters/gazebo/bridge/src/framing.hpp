// Length-delimited protobuf framing over a blocking TCP socket. A single
// dedicated writer thread owns the send path; gz-transport callback threads
// hand off already-framed bytes through a bounded queue and never block on the
// socket. This keeps a slow or wedged host peer from stalling a control-
// critical odometry callback behind a fat camera frame.
//
// The wire format is a protobuf varint byte length prefix followed by the
// encoded `BridgeEnvelope` payload, matching prost's
// `encode_length_delimited` / `decode_length_delimited` on the Rust side.
#ifndef PILOTAGE_GZ_BRIDGE_FRAMING_HPP
#define PILOTAGE_GZ_BRIDGE_FRAMING_HPP

#include <condition_variable>
#include <cstdint>
#include <deque>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <vector>

#include "pilotage/bridge/v1/bridge.pb.h"

namespace pilotage::bridge {

// Owns the connected TCP socket file descriptor for the sidecar<->host link.
// A dedicated writer thread drains a bounded outbound queue and performs every
// blocking send; producers only briefly hold the queue mutex to hand off bytes.
// Reads are single-threaded (only the inbound control loop calls ReadEnvelope).
class BridgeConnection {
 public:
  // Dials 127.0.0.1:port and returns a connected instance, or an error string
  // describing why the connect failed. On success the writer thread is running.
  static std::optional<BridgeConnection> Connect(std::uint16_t port,
                                                  std::string &error_out);

  BridgeConnection(BridgeConnection &&other) noexcept;
  BridgeConnection &operator=(BridgeConnection &&other) noexcept;
  BridgeConnection(const BridgeConnection &) = delete;
  BridgeConnection &operator=(const BridgeConnection &) = delete;
  ~BridgeConnection();

  // Encodes an envelope and hands the framed bytes to the writer thread's
  // bounded queue. Never blocks on the socket, so it is safe to call from a
  // gz-transport callback thread. `droppable` marks best-effort payloads
  // (camera frames): when the queue is congested they are dropped rather than
  // enqueued, so a slow peer cannot back-pressure the producer. Non-droppable
  // payloads (odometry) are always enqueued and are never starved behind a
  // pending frame. Returns false only once the connection is torn down; a
  // congestion drop of a droppable payload still returns true.
  bool WriteEnvelope(const pilotage::bridge::v1::BridgeEnvelope &envelope,
                     bool droppable);

  // Blocks reading exactly one length-delimited envelope from the socket.
  // Returns false on EOF or a read error (peer closed / broken pipe). Not
  // thread-safe; a single reader thread owns it.
  bool ReadEnvelope(pilotage::bridge::v1::BridgeEnvelope &envelope_out);

  // Marks the connection closed, shuts the socket down so a blocked
  // ReadEnvelope returns promptly, and wakes the writer thread to exit. Safe to
  // call from a signal-driven thread.
  void Shutdown();

 private:
  // Serialized outbound frame plus its priority. Frames are pre-encoded on the
  // producer thread so the writer thread only touches the socket.
  struct QueuedFrame {
    std::vector<std::uint8_t> bytes;
    bool droppable;
  };

  // Shared socket + outbound queue, heap-allocated so its address is stable
  // across a BridgeConnection move: the writer thread captures a raw pointer to
  // this block, never to the (movable) BridgeConnection itself.
  struct WriterState {
    int fd = -1;
    std::vector<std::uint8_t> read_buffer;

    // Bounded outbound queue drained by the writer thread. queue_mutex is held
    // only to push/pop; the blocking send happens with the lock released so a
    // stalled peer never parks a producer.
    std::mutex queue_mutex;
    std::condition_variable queue_cv;
    std::deque<QueuedFrame> queue;
    std::size_t pending_droppable = 0;
    bool stop = false;

    // Pops frames and sends them until stop or a send error, then closes the
    // send path so the peer sees EOF.
    void WriterLoop();
    // Writes exactly len bytes, retrying short writes; false on hard error or a
    // send timeout (a wedged peer). Runs only on the writer thread.
    bool WriteAll(const std::uint8_t *data, std::size_t len);
  };

  explicit BridgeConnection(int fd);

  // Reads exactly len bytes; false on EOF or hard error.
  bool ReadAll(std::uint8_t *data, std::size_t len);

  std::unique_ptr<WriterState> state_;
  std::thread writer_thread_;
};

}  // namespace pilotage::bridge

#endif  // PILOTAGE_GZ_BRIDGE_FRAMING_HPP
