# adapters/ — simulator and vehicle adapters

Implementations of the adapter contract from `pilotage-adapter-api`
([ADR-0008](../docs/adr/0008-engine-independent-adapter-boundary.md)). The public
client protocol never exposes adapter-native messages (Gazebo, ROS, DDS, CAN,
MAVLink, …).

Planned contents:

- `reference-headless/` — deterministic headless adapter; the conformance, replay,
  and accelerated-training anchor. A v1 deliverable.
- `gazebo/` — first graphical adapter: Gazebo integration, renderer capture,
  vehicle bridge.

Unreal, Unity, and real-vehicle gateways join as peer adapters when scheduled —
directories are created when their code lands, not before.
