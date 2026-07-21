#include "bridge_node.hpp"

#include <cmath>
#include <cstdint>

#include <gz/msgs/vector3d.pb.h>

namespace pilotage::bridge {

namespace {

// Yaw (heading about +Z) from a unit quaternion, standard ZYX extraction.
double YawFromQuaternion(double x, double y, double z, double w) {
  const double siny_cosp = 2.0 * (w * z + x * y);
  const double cosy_cosp = 1.0 - 2.0 * (y * y + z * z);
  return std::atan2(siny_cosp, cosy_cosp);
}

// Maps a gz.msgs.Header stamp (sec + nsec) to a flat nanosecond count. A
// negative sec would only arise from a malformed message; clamp to 0 so the
// unsigned field never underflows.
std::uint64_t StampToNanos(const gz::msgs::Header &header) {
  const auto &stamp = header.stamp();
  const std::int64_t sec = stamp.sec();
  if (sec <= 0) {
    return static_cast<std::uint64_t>(stamp.nsec() < 0 ? 0 : stamp.nsec());
  }
  return static_cast<std::uint64_t>(sec) * 1000000000ULL +
         static_cast<std::uint64_t>(stamp.nsec() < 0 ? 0 : stamp.nsec());
}

// Human-readable name for a gz pixel format enum, so the host can decide how
// to interpret BridgeFrame.rgb. The camera world publishes RGB_INT8; anything
// else is passed through with its label rather than silently mislabeled.
const char *PixelFormatName(gz::msgs::PixelFormatType format) {
  switch (format) {
    case gz::msgs::PixelFormatType::RGB_INT8:
      return "RGB_INT8";
    case gz::msgs::PixelFormatType::RGBA_INT8:
      return "RGBA_INT8";
    case gz::msgs::PixelFormatType::BGR_INT8:
      return "BGR_INT8";
    case gz::msgs::PixelFormatType::BGRA_INT8:
      return "BGRA_INT8";
    case gz::msgs::PixelFormatType::L_INT8:
      return "L_INT8";
    default:
      return "UNKNOWN";
  }
}

}  // namespace

BridgeNode::BridgeNode(BridgeConfig config, BridgeConnection *connection)
    : config_(std::move(config)), connection_(connection) {}

bool BridgeNode::Start(std::string &error_out) {
  const std::string cmd_vel_topic = "/model/" + config_.vehicle + "/cmd_vel";
  const std::string odom_topic = "/model/" + config_.vehicle + "/odometry";

  cmd_vel_pub_ = node_.Advertise<gz::msgs::Twist>(cmd_vel_topic);
  if (!cmd_vel_pub_) {
    error_out = "failed to advertise " + cmd_vel_topic;
    return false;
  }

  if (!node_.Subscribe(odom_topic, &BridgeNode::OnOdometry, this)) {
    error_out = "failed to subscribe " + odom_topic;
    return false;
  }

  if (!node_.Subscribe(config_.camera_topic, &BridgeNode::OnFpvImage, this)) {
    error_out = "failed to subscribe " + config_.camera_topic;
    return false;
  }

  if (!node_.Subscribe(config_.chase_camera_topic, &BridgeNode::OnChaseImage,
                       this)) {
    error_out = "failed to subscribe " + config_.chase_camera_topic;
    return false;
  }

  // The gimbal payload camera is optional: a vehicle without a gimbal leaves
  // the topic empty and the bridge subscribes no third camera.
  if (!config_.gimbal_camera_topic.empty() &&
      !node_.Subscribe(config_.gimbal_camera_topic, &BridgeNode::OnGimbalImage,
                       this)) {
    error_out = "failed to subscribe " + config_.gimbal_camera_topic;
    return false;
  }

  return true;
}

void BridgeNode::OnOdometry(const gz::msgs::Odometry &msg) {
  pilotage::bridge::v1::BridgeEnvelope envelope;
  auto *odom = envelope.mutable_odometry();

  const auto &position = msg.pose().position();
  odom->set_x(position.x());
  odom->set_y(position.y());

  const auto &q = msg.pose().orientation();
  odom->set_heading(YawFromQuaternion(q.x(), q.y(), q.z(), q.w()));

  odom->set_speed(msg.twist().linear().x());
  odom->set_sim_time_ns(StampToNanos(msg.header()));

  // Odometry is control-critical: never dropped on congestion. A false return
  // means the host link died; exiting is handled by the reader thread on the
  // same disconnect, so here we simply drop the sample.
  connection_->WriteEnvelope(envelope, /*droppable=*/false);
}

void BridgeNode::OnFpvImage(const gz::msgs::Image &msg) { OnImage(msg, 0); }

void BridgeNode::OnChaseImage(const gz::msgs::Image &msg) { OnImage(msg, 1); }

void BridgeNode::OnGimbalImage(const gz::msgs::Image &msg) { OnImage(msg, 2); }

void BridgeNode::OnImage(const gz::msgs::Image &msg, std::uint32_t camera_id) {
  pilotage::bridge::v1::BridgeEnvelope envelope;
  auto *frame = envelope.mutable_frame();

  frame->set_width(msg.width());
  frame->set_height(msg.height());
  frame->set_pixel_format(PixelFormatName(msg.pixel_format_type()));
  frame->set_sim_time_ns(StampToNanos(msg.header()));
  frame->set_rgb(msg.data());
  frame->set_camera_id(camera_id);

  // Camera frames are best-effort: a slow host drops frames rather than
  // stalling the shared writer and starving odometry behind them. The write
  // mutex inside WriteEnvelope only guards the queue hand-off, never the
  // blocking socket send, so concurrent callbacks from both camera topics
  // never stall each other or odometry.
  connection_->WriteEnvelope(envelope, /*droppable=*/true);
}

void BridgeNode::PublishControl(
    const pilotage::bridge::v1::BridgeControl &control) {
  gz::msgs::Twist twist;
  twist.mutable_linear()->set_x(control.linear_x());
  twist.mutable_angular()->set_z(control.angular_z());
  cmd_vel_pub_.Publish(twist);
}

}  // namespace pilotage::bridge
