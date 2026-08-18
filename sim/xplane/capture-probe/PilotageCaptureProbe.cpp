// PilotageCaptureProbe - production-shaped capture bench for X-Plane 12
// on Metal (Apple Silicon).
//
// Measures the full pipeline the camera plugin will run EVERY frame:
// scene framebuffer -> fixed-aspect crop -> glBlitFramebuffer downscale
// into an own FBO -> double-buffered PBO async glReadPixels -> mapped
// readback. Logs per-frame cost and callback cadence, and dumps frames
// for visual/gamma inspection.
//
// HARD CONSTRAINT: never touch GL_FRAMEBUFFER_SRGB. Toggling it under
// the bridge driver makes all subsequent rendering disappear (known
// Mesa/Zink defect class); treat the sRGB state as owned by X-Plane.

#include "XPLMDataAccess.h"
#include "XPLMDefs.h"
#include "XPLMDisplay.h"
#include "XPLMGraphics.h"
#include "XPLMUtilities.h"

#include <OpenGL/gl.h>
#include <OpenGL/glext.h>

#include <chrono>
#include <cstdio>
#include <cstring>
#include <string>

namespace {

constexpr int kOutW = 960;
constexpr int kOutH = 540;
constexpr int kBytes = kOutW * kOutH * 4;

XPLMDataRef msaa_ref = nullptr;
XPLMDataRef reverse_y_ref = nullptr;
XPLMDataRef current_fbo_ref = nullptr;
XPLMDataRef viewport_ref = nullptr;

bool gl_ready = false;
bool gl_failed = false;
GLuint target_fbo = 0;
GLuint target_tex = 0;
GLuint pbo[2] = {0, 0};
int pbo_index = 0;
int warmup = 0;

long frames = 0;
double capture_ms_sum = 0;
double capture_ms_max = 0;
std::chrono::steady_clock::time_point last_cb;
double cb_dt_sum = 0;
double cb_dt_max = 0;
bool dumped = false;
bool env_logged = false;

void log_line(const std::string& text) {
    XPLMDebugString(("PilotageCaptureProbe: " + text + "\n").c_str());
}

bool init_gl_objects() {
    glGenTextures(1, &target_tex);
    glBindTexture(GL_TEXTURE_2D, target_tex);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, kOutW, kOutH, 0, GL_RGBA,
                 GL_UNSIGNED_BYTE, nullptr);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glGenFramebuffersEXT(1, &target_fbo);
    glBindFramebufferEXT(GL_FRAMEBUFFER_EXT, target_fbo);
    glFramebufferTexture2DEXT(GL_FRAMEBUFFER_EXT, GL_COLOR_ATTACHMENT0_EXT,
                              GL_TEXTURE_2D, target_tex, 0);
    GLenum status = glCheckFramebufferStatusEXT(GL_FRAMEBUFFER_EXT);
    if (status != GL_FRAMEBUFFER_COMPLETE_EXT) {
        char line[96];
        std::snprintf(line, sizeof(line), "target FBO incomplete: 0x%x",
                      status);
        log_line(line);
        return false;
    }
    glGenBuffers(2, pbo);
    for (int i = 0; i < 2; ++i) {
        glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo[i]);
        glBufferData(GL_PIXEL_PACK_BUFFER, kBytes, nullptr, GL_STREAM_READ);
    }
    glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
    return glGetError() == GL_NO_ERROR;
}

void log_environment(GLint scene_fbo) {
    int msaa = msaa_ref ? XPLMGetDatai(msaa_ref) : -1;
    int reverse_y = reverse_y_ref ? XPLMGetDatai(reverse_y_ref) : -1;
    int dataref_fbo = current_fbo_ref ? XPLMGetDatai(current_fbo_ref) : -1;
    const GLubyte* version = glGetString(GL_VERSION);
    char line[256];
    std::snprintf(line, sizeof(line),
                  "env: msaa=%d reverse_y=%d dataref_fbo=%d bound_fbo=%d "
                  "gl=%s",
                  msaa, reverse_y, dataref_fbo, (int)scene_fbo,
                  version ? (const char*)version : "?");
    log_line(line);
}

int OnWindowPhase(XPLMDrawingPhase, int, void*) {
    if (gl_failed) {
        return 1;
    }
    auto t0 = std::chrono::steady_clock::now();
    if (frames > 0) {
        double dt =
            std::chrono::duration<double, std::milli>(t0 - last_cb).count();
        cb_dt_sum += dt;
        if (dt > cb_dt_max) cb_dt_max = dt;
    }
    last_cb = t0;

    GLint scene_fbo = 0;
    glGetIntegerv(GL_FRAMEBUFFER_BINDING, &scene_fbo);
    if (!env_logged) {
        env_logged = true;
        log_environment(scene_fbo);
    }
    if (!gl_ready) {
        gl_ready = init_gl_objects();
        gl_failed = !gl_ready;
        if (gl_failed) {
            log_line("GL init failed; bench disabled");
            return 1;
        }
        log_line("GL objects ready");
    }

    GLint vp[4] = {0, 0, 0, 0};
    glGetIntegerv(GL_VIEWPORT, vp);
    // Fixed 16:9 crop centered in the source so intrinsics stay stable
    // regardless of window shape.
    int src_w = vp[2];
    int src_h = vp[3];
    int crop_w = src_w;
    int crop_h = (src_w * 9) / 16;
    if (crop_h > src_h) {
        crop_h = src_h;
        crop_w = (src_h * 16) / 9;
    }
    int crop_x = (src_w - crop_w) / 2;
    int crop_y = (src_h - crop_h) / 2;

    glBindFramebufferEXT(GL_READ_FRAMEBUFFER_EXT, (GLuint)scene_fbo);
    glBindFramebufferEXT(GL_DRAW_FRAMEBUFFER_EXT, target_fbo);
    glBlitFramebufferEXT(crop_x, crop_y, crop_x + crop_w, crop_y + crop_h, 0,
                         0, kOutW, kOutH, GL_COLOR_BUFFER_BIT, GL_LINEAR);
    GLenum blit_err = glGetError();

    glBindFramebufferEXT(GL_READ_FRAMEBUFFER_EXT, target_fbo);
    glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo[pbo_index]);
    glReadPixels(0, 0, kOutW, kOutH, GL_RGBA, GL_UNSIGNED_BYTE, nullptr);
    GLenum read_err = glGetError();

    // Map LAST frame's PBO: the async copy has had a frame to finish.
    unsigned long checksum = 0;
    if (warmup >= 2) {
        int previous = pbo_index ^ 1;
        glBindBuffer(GL_PIXEL_PACK_BUFFER, pbo[previous]);
        void* mapped = glMapBuffer(GL_PIXEL_PACK_BUFFER, GL_READ_ONLY);
        if (mapped != nullptr) {
            const unsigned char* bytes = (const unsigned char*)mapped;
            for (int i = 0; i < kBytes; i += 4096) {
                checksum += bytes[i];
            }
            if (!dumped && frames > 400) {
                if (FILE* f = std::fopen("/tmp/pilotage-bench-frame.raw",
                                         "wb")) {
                    std::fwrite(mapped, 1, kBytes, f);
                    std::fclose(f);
                    dumped = true;
                    log_line("dumped /tmp/pilotage-bench-frame.raw");
                }
            }
            glUnmapBuffer(GL_PIXEL_PACK_BUFFER);
        }
    } else {
        warmup += 1;
    }
    pbo_index ^= 1;
    glBindBuffer(GL_PIXEL_PACK_BUFFER, 0);
    // Restore the framebuffer X-Plane expects (guidance: restore by
    // dataref when available).
    GLint restore = current_fbo_ref ? XPLMGetDatai(current_fbo_ref)
                                    : scene_fbo;
    glBindFramebufferEXT(GL_FRAMEBUFFER_EXT, (GLuint)restore);

    auto t1 = std::chrono::steady_clock::now();
    double ms = std::chrono::duration<double, std::milli>(t1 - t0).count();
    capture_ms_sum += ms;
    if (ms > capture_ms_max) capture_ms_max = ms;
    frames += 1;
    if (frames % 300 == 0) {
        char line[256];
        std::snprintf(
            line, sizeof(line),
            "bench n=%ld capture_ms avg=%.3f max=%.3f cb_dt_ms avg=%.2f "
            "max=%.1f blit_err=0x%x read_err=0x%x sum=%lu",
            frames, capture_ms_sum / (double)frames, capture_ms_max,
            cb_dt_sum / (double)(frames > 1 ? frames - 1 : 1), cb_dt_max,
            blit_err, read_err, checksum);
        log_line(line);
        capture_ms_max = 0;
        cb_dt_max = 0;
    }
    return 1;
}

}  // namespace

PLUGIN_API int XPluginStart(char* out_name, char* out_sig, char* out_desc) {
    std::strcpy(out_name, "PilotageCaptureProbe");
    std::strcpy(out_sig, "systems.sokoly.pilotage.captureprobe");
    std::strcpy(out_desc, "Capture-pipeline bench under the Metal GL bridge");
    return 1;
}

PLUGIN_API void XPluginStop(void) {}

PLUGIN_API int XPluginEnable(void) {
    msaa_ref = XPLMFindDataRef("sim/graphics/view/hardware_msaa_samples");
    reverse_y_ref = XPLMFindDataRef("sim/graphics/view/is_reverse_y");
    current_fbo_ref = XPLMFindDataRef("sim/graphics/view/current_gl_fbo");
    viewport_ref = XPLMFindDataRef("sim/graphics/view/viewport");
    XPLMRegisterDrawCallback(OnWindowPhase, xplm_Phase_Window, 0, nullptr);
    log_line("bench registered (window-after)");
    return 1;
}

PLUGIN_API void XPluginDisable(void) {
    XPLMUnregisterDrawCallback(OnWindowPhase, xplm_Phase_Window, 0, nullptr);
}

PLUGIN_API void XPluginReceiveMessage(XPLMPluginID, int, void*) {}
