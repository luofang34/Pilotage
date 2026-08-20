// Frame capture: scene framebuffer -> fixed-aspect crop -> downscale
// blit into an own framebuffer -> double-buffered asynchronous readback
// -> RGB rows in scan order -> the host link.
//
// The capture geometry is FIXED (never the window's own size): a
// published camera calibration is only meaningful when the intrinsics
// it implies stay constant, so the producer crops to a fixed aspect
// before it downscales.

#ifndef PILOTAGE_CAMERA_CAPTURE_H
#define PILOTAGE_CAMERA_CAPTURE_H

#include <cstdint>
#include <vector>

#include "link.h"

namespace pilotage_camera {

/// Fixed capture geometry. 16:9 at 960x540 keeps one frame near
/// 1.5 MB of RGB, which a localhost link carries comfortably at the
/// capped rate.
constexpr int kCaptureWidth = 960;
constexpr int kCaptureHeight = 540;

class Capture {
   public:
    /// Captures one frame if the rate cap allows, and hands it to
    /// `link`. Runs inside a 2-D drawing callback.
    void on_frame(HostLink& link);
    /// Marks a capture discontinuity (a flight reload).
    void reset_epoch();
    /// Releases the GL objects.
    void shutdown();

   private:
    bool ensure_objects();
    /// True while readback still yields a live scene. A producer that
    /// cannot see the scene stops streaming rather than sending a
    /// frozen or blank picture.
    bool scene_is_live(const std::uint8_t* rgba);

    unsigned int target_fbo_ = 0;
    unsigned int target_tex_ = 0;
    unsigned int pbo_[2] = {0, 0};
    int pbo_index_ = 0;
    int primed_ = 0;
    bool objects_ready_ = false;
    bool disabled_ = false;
    int blank_frames_ = 0;
    double next_capture_s_ = 0.0;
    std::vector<std::uint8_t> rgb_;
};

Capture& capture();

}  // namespace pilotage_camera

#endif  // PILOTAGE_CAMERA_CAPTURE_H
