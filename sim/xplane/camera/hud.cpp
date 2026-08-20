#include "hud.h"

#include "XPLMDisplay.h"
#include "XPLMGraphics.h"

#include <OpenGL/gl.h>

#include <cstdio>

namespace pilotage_camera {
namespace {

constexpr float kReticleColor[3] = {0.35F, 1.0F, 0.45F};

void draw_line(float x0, float y0, float x1, float y1) {
    glBegin(GL_LINES);
    glVertex2f(x0, y0);
    glVertex2f(x1, y1);
    glEnd();
}

}  // namespace

void draw_hud(const CameraState& state) {
    int screen_w = 0;
    int screen_h = 0;
    XPLMGetScreenSize(&screen_w, &screen_h);
    const float center_x = static_cast<float>(screen_w) * 0.5F;
    const float center_y = static_cast<float>(screen_h) * 0.5F;
    const float arm = static_cast<float>(screen_h) * 0.03F;
    const float gap = arm * 0.35F;

    XPLMSetGraphicsState(0, 0, 0, 0, 1, 0, 0);
    glColor4f(kReticleColor[0], kReticleColor[1], kReticleColor[2], 0.85F);

    // Center reticle: four ticks around an open center, so the aim
    // point stays visible.
    draw_line(center_x - arm, center_y, center_x - gap, center_y);
    draw_line(center_x + gap, center_y, center_x + arm, center_y);
    draw_line(center_x, center_y - arm, center_x, center_y - gap);
    draw_line(center_x, center_y + gap, center_x, center_y + arm);

    // Corner brackets frame the fixed capture aspect the calibration
    // assumes, so an operator sees what the published frame contains.
    const float frame_h = static_cast<float>(screen_h) * 0.4F;
    const float frame_w = frame_h * 16.0F / 9.0F;
    const float bracket = arm * 0.8F;
    const float left = center_x - frame_w;
    const float right = center_x + frame_w;
    const float bottom = center_y - frame_h;
    const float top = center_y + frame_h;
    draw_line(left, bottom, left + bracket, bottom);
    draw_line(left, bottom, left, bottom + bracket);
    draw_line(right - bracket, bottom, right, bottom);
    draw_line(right, bottom, right, bottom + bracket);
    draw_line(left, top, left + bracket, top);
    draw_line(left, top - bracket, left, top);
    draw_line(right - bracket, top, right, top);
    draw_line(right, top - bracket, right, top);

    char legend[96];
    std::snprintf(legend, sizeof(legend), "%s  FOV %.0f  PAN %+.0f  TILT %+.0f",
                  state.mode() == CameraMode::Gimbal ? "GIMBAL" : "FPV",
                  static_cast<double>(state.detent().field_of_view_deg),
                  static_cast<double>(state.pan_rad() * 57.2957795F),
                  static_cast<double>(state.tilt_rad() * 57.2957795F));
    float color[3] = {kReticleColor[0], kReticleColor[1], kReticleColor[2]};
    XPLMDrawString(color, static_cast<int>(left),
                   static_cast<int>(top + arm * 0.5F), legend, nullptr,
                   xplmFont_Basic);
}

}  // namespace pilotage_camera
