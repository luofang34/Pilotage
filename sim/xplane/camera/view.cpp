#include "view.h"

#include "XPLMCamera.h"
#include "XPLMDataAccess.h"
#include "XPLMGraphics.h"
#include "XPLMProcessing.h"
#include "XPLMUtilities.h"

#include <cmath>

#include "camera_state.h"

namespace pilotage_camera {
namespace {

/// Camera station offsets in the vehicle body frame, meters: forward,
/// right, up, measured from the vehicle reference point the position
/// datarefs report (the center of gravity). The FPV station sits ahead
/// of the airframe so the picture is the vehicle's forward view, not
/// the inside of a cockpit; the gimbal station sits under the belly, so
/// its downward travel is unobstructed.
constexpr float kFpvForwardM = 2.8F;
constexpr float kFpvUpM = 0.4F;
// The payload eye must sit CLEAR of the airframe model: an eye inside
// the hull renders the cabin's interior surfaces, which reads as "the
// gimbal shows the inside of the plane". A three-tonne airframe's
// belly is well over a metre below its reference point, so the gimbal
// hangs chin-style ahead of and beneath it.
constexpr float kGimbalForwardM = 1.6F;
constexpr float kGimbalUpM = -2.0F;

XPLMDataRef local_x_ref = nullptr;
XPLMDataRef local_y_ref = nullptr;
XPLMDataRef local_z_ref = nullptr;
XPLMDataRef psi_ref = nullptr;
XPLMDataRef theta_ref = nullptr;
XPLMDataRef phi_ref = nullptr;
XPLMDataRef fov_ref = nullptr;

void bind_datarefs() {
    if (local_x_ref != nullptr) {
        return;
    }
    local_x_ref = XPLMFindDataRef("sim/flightmodel/position/local_x");
    local_y_ref = XPLMFindDataRef("sim/flightmodel/position/local_y");
    local_z_ref = XPLMFindDataRef("sim/flightmodel/position/local_z");
    psi_ref = XPLMFindDataRef("sim/flightmodel/position/psi");
    theta_ref = XPLMFindDataRef("sim/flightmodel/position/theta");
    phi_ref = XPLMFindDataRef("sim/flightmodel/position/phi");
    fov_ref = XPLMFindDataRef("sim/graphics/view/field_of_view_deg");
}

constexpr float kDegToRad = 3.14159265F / 180.0F;
constexpr float kRadToDeg = 180.0F / 3.14159265F;

/// Places the camera at the commanded station and orientation. The
/// station offset is rotated by vehicle heading and pitch into
/// X-Plane's local OpenGL frame (x east, y up, z south).
int CameraFunc(XPLMCameraPosition_t* out_position, int is_losing, void*) {
    if (is_losing != 0) {
        view().note_camera_lost();
        return 0;
    }
    view().note_camera_served();
    if (out_position == nullptr) {
        return 1;
    }
    bind_datarefs();
    const float heading_deg = psi_ref ? XPLMGetDataf(psi_ref) : 0.0F;
    const float pitch_deg = theta_ref ? XPLMGetDataf(theta_ref) : 0.0F;
    const float roll_deg = phi_ref ? XPLMGetDataf(phi_ref) : 0.0F;

    const bool gimbal = state().mode() == CameraMode::Gimbal;
    const float forward_m = gimbal ? kGimbalForwardM : kFpvForwardM;
    const float up_m = gimbal ? kGimbalUpM : kFpvUpM;

    // The station rides the AIRFRAME, so its offset rotates by the
    // full attitude, not the heading alone: a heading-only offset
    // leaves the eye at its level-flight height while the hull pitches
    // through it, which renders as the airframe's interior sweeping
    // across the picture on every pitch oscillation.
    const float heading_rad = heading_deg * kDegToRad;
    const float pitch_rad = pitch_deg * kDegToRad;
    const float roll_rad = roll_deg * kDegToRad;
    const float sin_h = std::sin(heading_rad);
    const float cos_h = std::cos(heading_rad);
    const float sin_p = std::sin(pitch_rad);
    const float cos_p = std::cos(pitch_rad);
    const float sin_r = std::sin(roll_rad);
    const float cos_r = std::cos(roll_rad);
    // Local frame: +x east, +y up, -z north. Body axes at heading psi,
    // pitch theta, roll phi:
    const float fwd_x = sin_h * cos_p;
    const float fwd_y = sin_p;
    const float fwd_z = -cos_h * cos_p;
    const float right_x = cos_h;
    const float right_y = 0.0F;
    const float right_z = sin_h;
    // up = right x forward, then rolled about the forward axis.
    const float up0_x = -sin_h * sin_p;
    const float up0_y = cos_p;
    const float up0_z = cos_h * sin_p;
    // X-Plane's phi is positive-RIGHT (the FPV branch passes it
    // straight into the camera's own roll): rolling right tips the
    // body-up vector TOWARD the right wing, so the right axis enters
    // with a plus. The wrong sign swings a station to the wrong side
    // of the hull by |up_m|*sin(phi) — two meters of gimbal mast at a
    // thirty-degree bank.
    const float up_x = up0_x * cos_r + right_x * sin_r;
    const float up_y = up0_y * cos_r + right_y * sin_r;
    const float up_z = up0_z * cos_r + right_z * sin_r;
    out_position->x = (local_x_ref ? XPLMGetDataf(local_x_ref) : 0.0F)
        + forward_m * fwd_x + up_m * up_x;
    out_position->y = (local_y_ref ? XPLMGetDataf(local_y_ref) : 0.0F)
        + forward_m * fwd_y + up_m * up_y;
    out_position->z = (local_z_ref ? XPLMGetDataf(local_z_ref) : 0.0F)
        + forward_m * fwd_z + up_m * up_z;

    if (gimbal) {
        // Pan follows the vehicle heading; tilt is world-stabilized, so
        // the payload keeps its aim while the airframe manoeuvres.
        out_position->heading = heading_deg + state().pan_rad() * kRadToDeg;
        out_position->pitch = state().tilt_rad() * kRadToDeg;
        out_position->roll = 0.0F;
    } else {
        out_position->heading = heading_deg;
        out_position->pitch = pitch_deg;
        out_position->roll = roll_deg;
    }
    out_position->zoom = 1.0F;

    // The zoom detent IS the field of view: intrinsics for the
    // published calibration follow from it.
    if (fov_ref != nullptr) {
        XPLMSetDataf(fov_ref, state().detent().field_of_view_deg);
    }
    return 1;
}

}  // namespace

void View::start() {
    bind_datarefs();
    if (!field_of_view_saved_ && fov_ref != nullptr) {
        saved_field_of_view_deg_ = XPLMGetDataf(fov_ref);
        field_of_view_saved_ = true;
    }
    // Camera control places the eye, but the VIEW MODE decides what the
    // simulator draws around it: a cockpit view keeps drawing the
    // cockpit geometry, which a vehicle camera must not see. Select the
    // external view with no cockpit before taking the camera.
    if (XPLMCommandRef external =
            XPLMFindCommand("sim/view/forward_with_nothing");
        external != nullptr) {
        XPLMCommandOnce(external);
    }
    XPLMControlCamera(xplm_ControlCameraForever, CameraFunc, nullptr);
}

void View::stop() {
    if (serving_) {
        XPLMDontControlCamera();
        serving_ = false;
    }
    if (field_of_view_saved_ && fov_ref != nullptr) {
        XPLMSetDataf(fov_ref, saved_field_of_view_deg_);
    }
}

void View::note_camera_served() {
    last_served_s_ = XPLMGetElapsedTime();
    if (!serving_) {
        serving_ = true;
        log_line("vehicle view active");
    }
}

void View::reassert_if_lost() {
    if (state().mode() == CameraMode::Free) {
        return;
    }
    // Ownership is judged by the camera callback actually running, not
    // by the request returning: a request placed before the simulator
    // is ready is accepted and then never served, which no loss
    // notification ever reports.
    const float now = XPLMGetElapsedTime();
    if (now - last_served_s_ < kServeTimeoutS) {
        return;
    }
    serving_ = false;
    last_served_s_ = now;
    log_line("vehicle view is not being served; taking camera control");
    start();
}

View& view() {
    static View instance;
    return instance;
}

}  // namespace pilotage_camera
