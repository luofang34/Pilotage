// Bridges gz-transport to the length-delimited bridge protocol. This is the
// ONLY component that speaks gz-transport (ADR-0008): it subscribes odometry
// and camera image, publishes cmd_vel Twist, and translates each direction to
// / from the private `pilotage.bridge.v1` wire types before they touch the
// TCP link. No raw gz.msgs type crosses into the Rust adapter.
#ifndef PILOTAGE_GZ_BRIDGE_NODE_HPP
#define PILOTAGE_GZ_BRIDGE_NODE_HPP

#include <string>

#include <gz/msgs/image.pb.h>
#include <gz/msgs/odometry.pb.h>
#include <gz/msgs/twist.pb.h>
#include <gz/transport/Node.hh>

#include "framing.hpp"

namespace pilotage::bridge {

// Static configuration for one bridge session.
struct BridgeConfig {
  std::string vehicle;       // e.g. "vehicle_blue"
  std::string camera_topic;  // e.g. "/camera"
};

// Wires gz-transport subscriptions/publisher to a BridgeConnection. The node
// keeps a non-owning pointer to the connection; the connection must outlive
// the node.
class BridgeNode {
 public:
  BridgeNode(BridgeConfig config, BridgeConnection *connection);

  // Advertises cmd_vel and subscribes odometry + camera. Returns false with a
  // populated error_out if any gz-transport wiring step fails.
  bool Start(std::string &error_out);

  // Publishes a Twist onto <vehicle>/cmd_vel from a decoded control message.
  void PublishControl(const pilotage::bridge::v1::BridgeControl &control);

 private:
  // gz-transport member-function callbacks (run on gz reader threads).
  void OnOdometry(const gz::msgs::Odometry &msg);
  void OnImage(const gz::msgs::Image &msg);

  BridgeConfig config_;
  BridgeConnection *connection_;  // not owned
  gz::transport::Node node_;
  gz::transport::Node::Publisher cmd_vel_pub_;
};

}  // namespace pilotage::bridge

#endif  // PILOTAGE_GZ_BRIDGE_NODE_HPP
