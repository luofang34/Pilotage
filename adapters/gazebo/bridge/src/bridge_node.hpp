// Bridges gz-transport to the length-delimited bridge protocol. This is the
// ONLY component that speaks gz-transport (ADR-0008): it subscribes odometry
// and camera image, publishes cmd_vel Twist, and translates each direction to
// / from the private `pilotage.bridge.v1` wire types before they touch the
// TCP link. No raw gz.msgs type crosses into the Rust adapter.
#ifndef PILOTAGE_GZ_BRIDGE_NODE_HPP
#define PILOTAGE_GZ_BRIDGE_NODE_HPP

#include <cstdint>
#include <string>

#include <gz/msgs/image.pb.h>
#include <gz/msgs/odometry.pb.h>
#include <gz/msgs/twist.pb.h>
#include <gz/transport/Node.hh>

#include "framing.hpp"

namespace pilotage::bridge {

// Static configuration for one bridge session.
struct BridgeConfig {
  std::string vehicle;             // e.g. "vehicle_blue"
  std::string camera_topic;        // e.g. "/camera" (FPV, camera_id = 0)
  std::string chase_camera_topic;  // e.g. "/chase_camera" (camera_id = 1)
  // The gimbal payload camera (camera_id = 2). Empty when the vehicle carries
  // no gimbal, in which case the bridge subscribes no third camera.
  std::string gimbal_camera_topic;
};

// Wires gz-transport subscriptions/publisher to a BridgeConnection. The node
// keeps a non-owning pointer to the connection; the connection must outlive
// the node.
class BridgeNode {
 public:
  BridgeNode(BridgeConfig config, BridgeConnection *connection);

  // Advertises cmd_vel and subscribes odometry + both cameras. Returns false
  // with a populated error_out if any gz-transport wiring step fails.
  bool Start(std::string &error_out);

  // Publishes a Twist onto <vehicle>/cmd_vel from a decoded control message.
  void PublishControl(const pilotage::bridge::v1::BridgeControl &control);

 private:
  // gz-transport member-function callbacks (run on gz reader threads).
  void OnOdometry(const gz::msgs::Odometry &msg);
  // Per-topic thunks (gz-transport::Subscribe needs a fixed-arity callback)
  // that each forward to the shared OnImage body with their camera_id.
  void OnFpvImage(const gz::msgs::Image &msg);
  void OnChaseImage(const gz::msgs::Image &msg);
  void OnGimbalImage(const gz::msgs::Image &msg);
  // Shared onImage body for every camera subscription; camera_id tags the
  // emitted BridgeFrame so the host can route it to the right video source.
  void OnImage(const gz::msgs::Image &msg, std::uint32_t camera_id);

  BridgeConfig config_;
  BridgeConnection *connection_;  // not owned
  gz::transport::Node node_;
  gz::transport::Node::Publisher cmd_vel_pub_;
};

}  // namespace pilotage::bridge

#endif  // PILOTAGE_GZ_BRIDGE_NODE_HPP
