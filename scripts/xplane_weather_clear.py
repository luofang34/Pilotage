#!/usr/bin/env python3
"""Clear X-Plane weather through the Pilotage transaction datarefs."""

from __future__ import annotations

import argparse
import ipaddress
import math
import socket
import struct
import time
from collections.abc import Mapping

PROTOCOL_VERSION = 1.0
MAXIMUM_GENERATION = 16_777_215
READ_HZ = 10
CLEAR_STATUS = 3.0
CLEAR_OPERATION = 2.0
WIND_TOLERANCE_MPS = 0.5
TURBULENCE_TOLERANCE = 0.05
REQUIRED_CALM_SAMPLES = 3
DEFAULT_RESPONSE_PORT = 49001
RREF_REQUEST_HEADER = b"RREF\x00"
RREF_RESPONSE_HEADER = b"RREF,"
DREF_HEADER = b"DREF\x00"

PROTOCOL = 101
EXPECTED_GENERATION = 102
RESPONSE_GENERATION = 103
APPLIED_GENERATION = 104
RESPONSE_OPERATION = 105
RESPONSE_WIND_SPEED = 106
RESPONSE_WIND_DIRECTION = 107
RESPONSE_TURBULENCE = 108
STATUS = 109
ACTUAL_SPEED = 110
ACTUAL_VERTICAL = 111
ACTUAL_TURBULENCE_PROFILE_MAX = 112

REFS = {
    PROTOCOL: "pilotage/weather/protocol_version",
    EXPECTED_GENERATION: "pilotage/weather/expected_generation",
    RESPONSE_GENERATION: "pilotage/weather/response_generation",
    APPLIED_GENERATION: "pilotage/weather/applied_generation",
    RESPONSE_OPERATION: "pilotage/weather/response_operation",
    RESPONSE_WIND_SPEED: "pilotage/weather/response_wind_speed_mps",
    RESPONSE_WIND_DIRECTION: "pilotage/weather/response_wind_direction_deg",
    RESPONSE_TURBULENCE: "pilotage/weather/response_turbulence_scale",
    STATUS: "pilotage/weather/status",
    ACTUAL_SPEED: "pilotage/weather/actual_speed_mps",
    ACTUAL_VERTICAL: "pilotage/weather/actual_vertical_mps",
    ACTUAL_TURBULENCE_PROFILE_MAX: (
        "pilotage/weather/actual_turbulence_profile_max"
    ),
}

ACTUAL_REFS = {
    ACTUAL_SPEED,
    ACTUAL_VERTICAL,
    ACTUAL_TURBULENCE_PROFILE_MAX,
}


class WeatherClearError(RuntimeError):
    """The weather transaction did not prove a calm simulator state."""


def rref_request(rate: int, index: int, dataref: str) -> bytes:
    """Make one X-Plane RREF subscription datagram."""
    encoded = dataref.encode("ascii")
    if len(encoded) >= 400:
        raise WeatherClearError(f"dataref name is too long: {dataref}")
    return struct.pack("<5sii400s", RREF_REQUEST_HEADER, rate, index, encoded)


def dref_request(dataref: str, value: float) -> bytes:
    """Make one X-Plane DREF write datagram."""
    encoded = dataref.encode("ascii") + b"\x00"
    if len(encoded) > 500:
        raise WeatherClearError(f"dataref name is too long: {dataref}")
    return DREF_HEADER + struct.pack("<f500s", value, encoded)


def decode_rref(
    packet: bytes,
    sender: tuple[str, int],
    expected_sender: tuple[str, int],
) -> dict[int, float]:
    """Decode one complete RREF response from the selected simulator."""
    if sender != expected_sender:
        return {}
    if not packet.startswith(RREF_RESPONSE_HEADER):
        raise WeatherClearError("X-Plane returned an invalid RREF header")
    body = packet[len(RREF_RESPONSE_HEADER) :]
    if len(body) < 8 or len(body) % 8 != 0:
        raise WeatherClearError("X-Plane returned a truncated RREF response")
    values = {}
    for offset in range(0, len(body), 8):
        index, value = struct.unpack_from("<if", body, offset)
        if index in REFS:
            values[index] = value
    return values


def subscribe(
    sock: socket.socket,
    address: tuple[str, int],
    rate: int,
) -> None:
    """Set the update rate for all weather transaction datarefs."""
    for index, dataref in REFS.items():
        sock.sendto(rref_request(rate, index, dataref), address)


def receive(
    sock: socket.socket,
    response_address: tuple[str, int],
    deadline: float,
) -> dict[int, float]:
    """Receive one valid response before a monotonic deadline."""
    while time.monotonic() < deadline:
        sock.settimeout(min(0.25, max(0.001, deadline - time.monotonic())))
        try:
            packet, sender = sock.recvfrom(4096)
        except socket.timeout:
            continue
        values = decode_rref(packet, sender, response_address)
        if values:
            return values
    return {}


def validate_generation(value: float | None) -> int:
    """Decode the exact integer generation that float32 can carry."""
    if (
        value is None
        or not math.isfinite(value)
        or value < 1.0
        or value > MAXIMUM_GENERATION
        or value != math.floor(value)
    ):
        raise WeatherClearError(f"invalid expected_generation {value}")
    return int(value)


def response_is_clear(values: Mapping[int, float], generation: int) -> bool:
    """Test the complete region-commit acknowledgement tuple."""
    return (
        values.get(RESPONSE_GENERATION) == generation
        and values.get(APPLIED_GENERATION) == generation
        and values.get(RESPONSE_OPERATION) == CLEAR_OPERATION
        and values.get(RESPONSE_WIND_SPEED) == 0.0
        and values.get(RESPONSE_WIND_DIRECTION) == 0.0
        and values.get(RESPONSE_TURBULENCE) == 0.0
        and values.get(STATUS) == CLEAR_STATUS
    )


def actual_is_calm(values: Mapping[int, float]) -> bool:
    """Test one complete aircraft-point observation."""
    actual = [
        values.get(ACTUAL_SPEED),
        values.get(ACTUAL_VERTICAL),
        values.get(ACTUAL_TURBULENCE_PROFILE_MAX),
    ]
    if any(value is None or not math.isfinite(value) for value in actual):
        return False
    speed, vertical, turbulence = actual
    return (
        0.0 <= speed <= WIND_TOLERANCE_MPS
        and abs(vertical) <= WIND_TOLERANCE_MPS
        and 0.0 <= turbulence <= TURBULENCE_TOLERANCE
    )


def discover_generation(
    sock: socket.socket,
    response_address: tuple[str, int],
    timeout_s: float,
) -> int:
    """Read the plugin protocol and next generation."""
    values: dict[int, float] = {}
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        values.update(receive(sock, response_address, deadline))
        if PROTOCOL in values and EXPECTED_GENERATION in values:
            break
    if values.get(PROTOCOL) != PROTOCOL_VERSION:
        raise WeatherClearError(
            f"PilotageWeather protocol is {values.get(PROTOCOL)}, "
            f"expected {PROTOCOL_VERSION}"
        )
    return validate_generation(values.get(EXPECTED_GENERATION))


def wait_for_clear(
    sock: socket.socket,
    response_address: tuple[str, int],
    generation: int,
    timeout_s: float,
) -> None:
    """Wait for a region ACK and fresh stable aircraft-point calm samples."""
    response: dict[int, float] = {}
    actual: dict[int, float] = {}
    ack_seen = False
    calm_samples = 0
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        update = receive(sock, response_address, deadline)
        response.update(
            {
                key: value
                for key, value in update.items()
                if key not in ACTUAL_REFS
            }
        )
        status = response.get(STATUS)
        if (
            response.get(RESPONSE_GENERATION) == generation
            and status is not None
            and status < 0.0
        ):
            raise WeatherClearError(
                f"PilotageWeather refused clear generation {generation}: status {status}"
            )
        if not ack_seen:
            if response_is_clear(response, generation):
                ack_seen = True
                actual.clear()
            continue
        if not response_is_clear(response, generation):
            raise WeatherClearError("PilotageWeather clear acknowledgement changed")
        actual.update({key: value for key, value in update.items() if key in ACTUAL_REFS})
        if ACTUAL_REFS.issubset(actual):
            calm_samples = calm_samples + 1 if actual_is_calm(actual) else 0
            actual.clear()
            if calm_samples == REQUIRED_CALM_SAMPLES:
                return
    raise WeatherClearError(
        f"PilotageWeather clear generation {generation} did not converge"
    )


def clear_weather(
    address: tuple[str, int] = ("127.0.0.1", 49000),
    discovery_timeout_s: float = 5.0,
    clear_timeout_s: float = 15.0,
    response_address: tuple[str, int] | None = None,
) -> int:
    """Clear weather and return the acknowledged generation."""
    try:
        is_loopback = ipaddress.ip_address(address[0]).is_loopback
    except ValueError as error:
        raise WeatherClearError(f"invalid X-Plane address {address[0]}") from error
    if not is_loopback or not 1 <= address[1] <= 65535:
        raise WeatherClearError(f"X-Plane address must be loopback: {address}")
    if response_address is None:
        response_address = (address[0], DEFAULT_RESPONSE_PORT)
    try:
        response_is_loopback = ipaddress.ip_address(response_address[0]).is_loopback
    except ValueError as error:
        raise WeatherClearError(
            f"invalid X-Plane response address {response_address[0]}"
        ) from error
    if not response_is_loopback or not 1 <= response_address[1] <= 65535:
        raise WeatherClearError(
            f"X-Plane response address must be loopback: {response_address}"
        )
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("127.0.0.1", 0))
    try:
        subscribe(sock, address, READ_HZ)
        generation = discover_generation(sock, response_address, discovery_timeout_s)
        sock.sendto(
            dref_request("pilotage/weather/clear_generation", float(generation)),
            address,
        )
        wait_for_clear(sock, response_address, generation, clear_timeout_s)
        return generation
    finally:
        subscribe(sock, address, 0)
        sock.close()


def main() -> int:
    """Run the weather clear command."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=49000)
    parser.add_argument("--response-port", type=int, default=DEFAULT_RESPONSE_PORT)
    args = parser.parse_args()
    try:
        generation = clear_weather(
            (args.host, args.port),
            response_address=(args.host, args.response_port),
        )
    except WeatherClearError as error:
        parser.exit(1, f"{error}\n")
    print(f"weather clear generation {generation} acknowledged")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
