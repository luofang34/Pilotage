// The localhost TCP link to the Pilotage host: dials back into the
// adapter's listener, writes length-delimited `pilotage.bridge.v1`
// frame envelopes, and reads camera commands.
//
// Non-blocking throughout: a frame that cannot be written promptly is
// dropped rather than stalling the render thread, and the newest frame
// always wins.

#ifndef PILOTAGE_CAMERA_LINK_H
#define PILOTAGE_CAMERA_LINK_H

#include <cstddef>
#include <cstdint>
#include <vector>

#include "camera_state.h"

namespace pilotage_camera {

class HostLink {
   public:
    /// Retries the connection when down, and drains inbound commands.
    void pump();
    /// The most recent decoded command, or `nullptr` when none arrived
    /// since the last call.
    const CameraCommand* take_command();
    /// Queues one encoded frame; drops it when a write is still
    /// pending (latest-frame-wins, never a growing backlog).
    void send_frame(std::uint32_t width, std::uint32_t height,
                    std::uint64_t time_ns, std::uint32_t camera_id,
                    const std::uint8_t* rgb, std::size_t rgb_len);
    void close();
    bool connected() const { return fd_ >= 0; }

   private:
    void flush();

    int fd_ = -1;
    double next_attempt_s_ = 0.0;
    std::vector<std::uint8_t> out_;
    std::size_t out_sent_ = 0;
    std::vector<std::uint8_t> in_;
    CameraCommand command_{};
    bool command_pending_ = false;
};

HostLink& link();

}  // namespace pilotage_camera

#endif  // PILOTAGE_CAMERA_LINK_H
