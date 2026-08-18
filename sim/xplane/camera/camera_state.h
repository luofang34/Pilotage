// Shared vocabulary for the camera producer: modes, zoom detents, the
// commanded state, and small logging/config helpers.

#ifndef PILOTAGE_CAMERA_STATE_H
#define PILOTAGE_CAMERA_STATE_H

#include <cstdint>
#include <string>

namespace pilotage_camera {

/// Which vehicle camera the single rendered view embodies.
enum class CameraMode : std::uint32_t {
    /// Body-fixed forward view at the nose station (video source 0).
    Fpv = 0,
    /// Steerable payload view (video source 2).
    Gimbal = 1,
    /// The view belongs to the operator; the producer streams nothing.
    Free = 2,
};

/// One zoom detent: its horizontal field of view, and the calibration
/// identity published for frames captured at that detent. Intrinsics
/// follow from the field of view and the FIXED capture geometry, so a
/// detent is a complete camera model for a consumer that reprojects
/// (a head-mounted display, a conformal overlay).
struct ZoomDetent {
    /// Horizontal field of view, degrees.
    float field_of_view_deg;
    /// Published calibration id for this detent. Mirrors the host
    /// adapter's detent table; the two MUST agree, or a frame would
    /// carry a camera model it was not captured with.
    std::uint32_t calibration_id;
};

/// The detent table. Entry 0 is the wide FPV framing a head-mounted
/// display needs for head-look reprojection room.
constexpr ZoomDetent kZoomDetents[] = {
    {100.0F, 0x5850'0001U},
    {60.0F, 0x5850'0002U},
    {30.0F, 0x5850'0003U},
    {12.0F, 0x5850'0004U},
};
constexpr int kZoomDetentCount =
    static_cast<int>(sizeof(kZoomDetents) / sizeof(kZoomDetents[0]));

/// Video source ids, matching the host's routing vocabulary.
constexpr std::uint32_t kSourceFpv = 0;
constexpr std::uint32_t kSourceGimbal = 2;

/// Gimbal travel limits, radians.
constexpr float kPanLimitRad = 3.1415926F;
constexpr float kTiltMinRad = -1.5707963F;
constexpr float kTiltMaxRad = 0.5235988F;

/// One decoded host camera command.
struct CameraCommand {
    std::uint32_t mode;
    float pan_rad;
    float tilt_rad;
    std::uint32_t zoom_detent;
};

/// The commanded camera state, clamped to what the producer can enact.
class CameraState {
   public:
    void apply(const CameraCommand* command);

    CameraMode mode() const { return mode_; }
    float pan_rad() const { return pan_rad_; }
    float tilt_rad() const { return tilt_rad_; }
    int zoom_detent() const { return zoom_detent_; }
    const ZoomDetent& detent() const { return kZoomDetents[zoom_detent_]; }
    /// The video source id frames carry in the current mode.
    std::uint32_t source_id() const {
        return mode_ == CameraMode::Gimbal ? kSourceGimbal : kSourceFpv;
    }

   private:
    CameraMode mode_ = CameraMode::Fpv;
    float pan_rad_ = 0.0F;
    float tilt_rad_ = 0.0F;
    int zoom_detent_ = 0;
};

CameraState& state();

/// True when the producer draws its own HUD overlay into the captured
/// picture (`PILOTAGE_XPLANE_CAMERA_HUD=1`).
bool hud_enabled();

/// Reads an unsigned environment setting, or `fallback` when absent or
/// malformed.
unsigned env_unsigned(const char* name, unsigned fallback);

void log_line(const std::string& text);

}  // namespace pilotage_camera

#endif  // PILOTAGE_CAMERA_STATE_H
