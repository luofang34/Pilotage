#include "capture.h"

#include "XPLMDataAccess.h"
#include "XPLMProcessing.h"

#include <OpenGL/gl.h>
#include <OpenGL/glext.h>

#include <chrono>

#include "camera_state.h"

namespace pilotage_camera {
namespace {

constexpr int kRgbaBytes = kCaptureWidth * kCaptureHeight * 4;
constexpr int kRgbBytes = kCaptureWidth * kCaptureHeight * 3;
/// Consecutive blank readbacks that mean the capture path stopped
/// seeing the scene (a simulator change broke the mechanism).
constexpr int kBlankFramesFailClosed = 90;

XPLMDataRef current_fbo_ref = nullptr;

std::uint64_t monotonic_ns() {
    const auto now = std::chrono::steady_clock::now().time_since_epoch();
    return static_cast<std::uint64_t>(
        std::chrono::duration_cast<std::chrono::nanoseconds>(now).count());
}

}  // namespace

bool Capture::ensure_objects() {
    if (objects_ready_) {
        return true;
    }
    if (current_fbo_ref == nullptr) {
        current_fbo_ref = XPLMFindDataRef("sim/graphics/view/current_gl_fbo");
    }
    glGenTextures(1, &target_tex_);
    glBindTexture(GL_TEXTURE_2D, target_tex_);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, kCaptureWidth, kCaptureHeight, 0,
                 GL_RGBA, GL_UNSIGNED_BYTE, nullptr);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glGenFramebuffersEXT(1, &target_fbo_);
    glBindFramebufferEXT(GL_FRAMEBUFFER_EXT, target_fbo_);
    glFramebufferTexture2DEXT(GL_FRAMEBUFFER_EXT, GL_COLOR_ATTACHMENT0_EXT,
                              GL_TEXTURE_2D, target_tex_, 0);
    const GLenum status = glCheckFramebufferStatusEXT(GL_FRAMEBUFFER_EXT);
    if (status != GL_FRAMEBUFFER_COMPLETE_EXT) {
        log_line("capture framebuffer incomplete; video disabled");
        disabled_ = true;
        return false;
    }
    glGenBuffers(2, pbo_);
    for (int index = 0; index < 2; ++index) {
        glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo_[index]);
        glBufferData(GL_PIXEL_PACK_BUFFER, kRgbaBytes, nullptr,
                     GL_STREAM_READ);
    }
    glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
    if (glGetError() != GL_NO_ERROR) {
        log_line("capture buffer setup failed; video disabled");
        disabled_ = true;
        return false;
    }
    rgb_.assign(kRgbBytes, 0);
    objects_ready_ = true;
    return true;
}

bool Capture::scene_is_live(const std::uint8_t* rgba) {
    // A live scene varies across the frame; an all-constant readback
    // means the capture path no longer sees the rendered world.
    const std::uint8_t first = rgba[0];
    for (int offset = 0; offset < kRgbaBytes; offset += 4093) {
        if (rgba[offset] != first) {
            blank_frames_ = 0;
            return true;
        }
    }
    blank_frames_ += 1;
    if (blank_frames_ >= kBlankFramesFailClosed) {
        log_line(
            "readback stopped yielding the rendered scene; video disabled "
            "(the simulator's plugin framebuffer contract changed)");
        disabled_ = true;
    }
    return false;
}

void Capture::on_frame(HostLink& host_link) {
    if (disabled_ || state().mode() == CameraMode::Free ||
        !host_link.connected()) {
        return;
    }
    const unsigned fps = env_unsigned("PILOTAGE_XPLANE_CAMERA_FPS", 24);
    const double now = XPLMGetElapsedTime();
    if (now < next_capture_s_) {
        return;
    }
    next_capture_s_ = now + 1.0 / static_cast<double>(fps);
    if (!ensure_objects()) {
        return;
    }

    GLint scene_fbo = 0;
    glGetIntegerv(GL_FRAMEBUFFER_BINDING, &scene_fbo);
    GLint viewport[4] = {0, 0, 0, 0};
    glGetIntegerv(GL_VIEWPORT, viewport);

    // Fixed-aspect centered crop: the published calibration assumes this
    // geometry, so a resized window must not change the intrinsics.
    int crop_w = viewport[2];
    int crop_h = (viewport[2] * kCaptureHeight) / kCaptureWidth;
    if (crop_h > viewport[3]) {
        crop_h = viewport[3];
        crop_w = (viewport[3] * kCaptureWidth) / kCaptureHeight;
    }
    const int crop_x = viewport[0] + (viewport[2] - crop_w) / 2;
    const int crop_y = viewport[1] + (viewport[3] - crop_h) / 2;

    glBindFramebufferEXT(GL_READ_FRAMEBUFFER_EXT,
                         static_cast<GLuint>(scene_fbo));
    glBindFramebufferEXT(GL_DRAW_FRAMEBUFFER_EXT, target_fbo_);
    glBlitFramebufferEXT(crop_x, crop_y, crop_x + crop_w, crop_y + crop_h, 0,
                         0, kCaptureWidth, kCaptureHeight,
                         GL_COLOR_BUFFER_BIT, GL_LINEAR);

    glBindFramebufferEXT(GL_READ_FRAMEBUFFER_EXT, target_fbo_);
    glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo_[pbo_index_]);
    glReadPixels(0, 0, kCaptureWidth, kCaptureHeight, GL_RGBA,
                 GL_UNSIGNED_BYTE, nullptr);

    if (primed_ >= 2) {
        const int previous = pbo_index_ ^ 1;
        glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo_[previous]);
        const void* mapped = glMapBuffer(GL_PIXEL_PACK_BUFFER, GL_READ_ONLY);
        if (mapped != nullptr) {
            const std::uint8_t* rgba = static_cast<const std::uint8_t*>(mapped);
            if (scene_is_live(rgba)) {
                // GL rows run bottom-up; the wire frame is top-down RGB.
                for (int row = 0; row < kCaptureHeight; ++row) {
                    const std::uint8_t* source =
                        rgba + (kCaptureHeight - 1 - row) * kCaptureWidth * 4;
                    std::uint8_t* destination =
                        rgb_.data() + row * kCaptureWidth * 3;
                    for (int column = 0; column < kCaptureWidth; ++column) {
                        destination[column * 3] = source[column * 4];
                        destination[column * 3 + 1] = source[column * 4 + 1];
                        destination[column * 3 + 2] = source[column * 4 + 2];
                    }
                }
                host_link.send_frame(kCaptureWidth, kCaptureHeight,
                                     monotonic_ns(), state().source_id(),
                                     rgb_.data(), rgb_.size());
            }
            glUnmapBuffer(GL_PIXEL_PACK_BUFFER);
        }
    } else {
        primed_ += 1;
    }
    pbo_index_ ^= 1;
    glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);

    const GLint restore =
        current_fbo_ref != nullptr ? XPLMGetDatai(current_fbo_ref) : scene_fbo;
    glBindFramebufferEXT(GL_FRAMEBUFFER_EXT, static_cast<GLuint>(restore));
}

void Capture::reset_epoch() {
    primed_ = 0;
    blank_frames_ = 0;
}

void Capture::shutdown() {
    if (!objects_ready_) {
        return;
    }
    glDeleteBuffers(2, pbo_);
    glDeleteFramebuffersEXT(1, &target_fbo_);
    glDeleteTextures(1, &target_tex_);
    objects_ready_ = false;
}

Capture& capture() {
    static Capture instance;
    return instance;
}

}  // namespace pilotage_camera
