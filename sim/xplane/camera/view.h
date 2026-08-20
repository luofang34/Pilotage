// Owns the simulator's single rendered view as a vehicle camera.
//
// X-Plane renders exactly one world view per instance, so FPV and the
// gimbal payload are ALTERNATIVES here, not simultaneous sources: the
// commanded mode decides which vehicle camera the view embodies. The
// producer takes the camera "forever" and re-asserts it when another
// plugin or an operator view change takes it away.

#ifndef PILOTAGE_CAMERA_VIEW_H
#define PILOTAGE_CAMERA_VIEW_H

namespace pilotage_camera {

class View {
   public:
    /// Takes camera control and snapshots the operator's field of view.
    void start();
    /// Releases camera control and restores the snapshotted field of
    /// view (a persisted user setting the producer only borrows).
    void stop();
    /// Re-takes control when the camera callback has stopped running.
    void reassert_if_lost();
    /// True while X-Plane is actually serving this producer's view.
    bool serving() const { return serving_; }
    /// Marks the camera as lost (the X-Plane camera callback reports
    /// this when a view change takes control away).
    void note_camera_lost() { serving_ = false; }
    /// Records that X-Plane called the camera callback this frame.
    void note_camera_served();

   private:
    /// Seconds without a camera callback that mean the request was
    /// accepted but never served (placed before the simulator was
    /// ready), or that control was taken away silently.
    static constexpr float kServeTimeoutS = 2.0F;

    bool serving_ = false;
    float last_served_s_ = 0.0F;
    bool field_of_view_saved_ = false;
    float saved_field_of_view_deg_ = 0.0F;
};

View& view();

}  // namespace pilotage_camera

#endif  // PILOTAGE_CAMERA_VIEW_H
