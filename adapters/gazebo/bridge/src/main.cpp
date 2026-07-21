// pilotage-gz-bridge: C++ gz-transport sidecar (ADR-0008).
//
// Connects a TCP client to the Rust adapter on 127.0.0.1:<port>, subscribes
// the vehicle's odometry and camera topics, advertises its cmd_vel, and
// translates in both directions over the length-delimited bridge protocol.
// It is the only process in Pilotage that links gz-transport.
//
// Exit codes: 0 clean shutdown (signal or peer close); non-zero on a fatal
// wiring/connect error, with a diagnostic on stderr.

#include <atomic>
#include <csignal>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <optional>
#include <string>

#include "bridge_node.hpp"
#include "framing.hpp"

namespace {

// Set from a signal handler; async-signal-safe atomic flag only.
std::atomic<bool> g_shutdown_requested{false};

// Non-owning handle to the live connection so the signal handler can unblock a
// parked ReadEnvelope. Written once before the reader loop starts.
std::atomic<pilotage::bridge::BridgeConnection *> g_connection{nullptr};

void HandleSignal(int /*signum*/) {
  g_shutdown_requested.store(true, std::memory_order_relaxed);
  auto *conn = g_connection.load(std::memory_order_relaxed);
  if (conn != nullptr) {
    conn->Shutdown();
  }
}

struct Args {
  std::uint16_t port = 0;
  std::string vehicle = "vehicle_blue";
  std::string camera_topic = "/camera";
  std::string chase_camera_topic = "/chase_camera";
  // Empty by default: only a gimbal-bearing vehicle sets a payload-camera topic.
  std::string gimbal_camera_topic;
};

// Parses --port/--vehicle/--camera-topic/--chase-camera-topic. Returns
// nullopt and writes a usage diagnostic on a missing or malformed argument.
std::optional<Args> ParseArgs(int argc, char **argv) {
  Args args;
  bool have_port = false;

  for (int i = 1; i < argc; ++i) {
    const std::string flag = argv[i];
    const bool has_value = (i + 1) < argc;

    if (flag == "--port" && has_value) {
      const long value = std::strtol(argv[++i], nullptr, 10);
      if (value <= 0 || value > 65535) {
        std::cerr << "pilotage-gz-bridge: --port must be 1..65535\n";
        return std::nullopt;
      }
      args.port = static_cast<std::uint16_t>(value);
      have_port = true;
    } else if (flag == "--vehicle" && has_value) {
      args.vehicle = argv[++i];
    } else if (flag == "--camera-topic" && has_value) {
      args.camera_topic = argv[++i];
    } else if (flag == "--chase-camera-topic" && has_value) {
      args.chase_camera_topic = argv[++i];
    } else if (flag == "--gimbal-camera-topic" && has_value) {
      args.gimbal_camera_topic = argv[++i];
    } else {
      std::cerr << "pilotage-gz-bridge: unexpected argument '" << flag << "'\n";
      return std::nullopt;
    }
  }

  if (!have_port) {
    std::cerr << "usage: pilotage-gz-bridge --port N [--vehicle NAME] "
                 "[--camera-topic TOPIC] [--chase-camera-topic TOPIC] "
                 "[--gimbal-camera-topic TOPIC]\n";
    return std::nullopt;
  }
  return args;
}

void InstallSignalHandlers() {
  struct sigaction sa{};
  sa.sa_handler = HandleSignal;
  sigemptyset(&sa.sa_mask);
  sa.sa_flags = 0;
  sigaction(SIGINT, &sa, nullptr);
  sigaction(SIGTERM, &sa, nullptr);
  // A dead peer would otherwise raise SIGPIPE and kill us mid-write; ignore it
  // and rely on send() returning an error instead.
  std::signal(SIGPIPE, SIG_IGN);
}

}  // namespace

int main(int argc, char **argv) {
  const std::optional<Args> parsed = ParseArgs(argc, argv);
  if (!parsed.has_value()) {
    return EXIT_FAILURE;
  }
  const Args &args = *parsed;

  InstallSignalHandlers();

  std::string error;
  std::optional<pilotage::bridge::BridgeConnection> connection =
      pilotage::bridge::BridgeConnection::Connect(args.port, error);
  if (!connection.has_value()) {
    std::cerr << "pilotage-gz-bridge: " << error << "\n";
    return EXIT_FAILURE;
  }
  g_connection.store(&(*connection), std::memory_order_relaxed);

  pilotage::bridge::BridgeConfig config{args.vehicle, args.camera_topic,
                                        args.chase_camera_topic,
                                        args.gimbal_camera_topic};
  pilotage::bridge::BridgeNode bridge(config, &(*connection));
  if (!bridge.Start(error)) {
    std::cerr << "pilotage-gz-bridge: " << error << "\n";
    return EXIT_FAILURE;
  }

  std::cerr << "pilotage-gz-bridge: connected to 127.0.0.1:" << args.port
            << ", vehicle=" << args.vehicle
            << ", camera=" << args.camera_topic
            << ", chase_camera=" << args.chase_camera_topic
            << ", gimbal_camera="
            << (args.gimbal_camera_topic.empty() ? "(none)"
                                                 : args.gimbal_camera_topic)
            << "\n";

  // Inbound control loop: block on the socket, publish each BridgeControl as a
  // Twist. A false read is EOF/error/shutdown -> exit so the parent detects
  // our death. gz callbacks continue to fire on their own threads meanwhile.
  pilotage::bridge::v1::BridgeEnvelope envelope;
  while (!g_shutdown_requested.load(std::memory_order_relaxed)) {
    if (!connection->ReadEnvelope(envelope)) {
      break;
    }
    if (envelope.has_control()) {
      bridge.PublishControl(envelope.control());
    }
  }

  g_connection.store(nullptr, std::memory_order_relaxed);
  std::cerr << "pilotage-gz-bridge: shutting down\n";
  return EXIT_SUCCESS;
}
