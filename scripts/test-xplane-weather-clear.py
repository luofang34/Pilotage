#!/usr/bin/env python3
"""Test the X-Plane weather clear UDP protocol with a local peer."""

from __future__ import annotations

import contextlib
import io
import queue
import socket
import struct
import sys
import threading
import unittest
from unittest import mock

sys.dont_write_bytecode = True

import xplane_weather_clear as weather

WIRE_RREF_REQUEST_HEADER = b"RREF\x00"
WIRE_RREF_RESPONSE_HEADER = b"RREF,"


class MockXPlane:
    """A bounded RREF and DREF peer for one clear transaction."""

    def __init__(
        self,
        refusal_status: float | None = None,
        same_ack_actual_only: bool = False,
    ) -> None:
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.response_socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            self.socket.bind(("127.0.0.1", 0))
            self.response_socket.bind(("127.0.0.1", 0))
        except BaseException:
            self.socket.close()
            self.response_socket.close()
            raise
        self.socket.settimeout(0.02)
        self.address = self.socket.getsockname()
        self.response_address = self.response_socket.getsockname()
        self.refusal_status = refusal_status
        self.same_ack_actual_only = same_ack_actual_only
        self.same_ack_actual_sent = False
        self.client: tuple[str, int] | None = None
        self.subscriptions: dict[int, int] = {}
        self.write_seen = threading.Event()
        self.ack_sent = threading.Event()
        self.release_actual = threading.Event()
        self.unsubscribed = threading.Event()
        self.stop = threading.Event()
        self.failures: queue.Queue[BaseException] = queue.Queue()
        self.values = {
            weather.PROTOCOL: weather.PROTOCOL_VERSION,
            weather.EXPECTED_GENERATION: 7.0,
            weather.ACTUAL_SPEED: 0.0,
            weather.ACTUAL_VERTICAL: 0.0,
            weather.ACTUAL_TURBULENCE_PROFILE_MAX: 0.0,
        }
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self) -> None:
        """Serve requests until the test closes the peer."""
        try:
            while not self.stop.is_set():
                try:
                    packet, sender = self.socket.recvfrom(4096)
                    self.client = sender
                    self.handle(packet)
                except socket.timeout:
                    pass
                except OSError:
                    if not self.stop.is_set():
                        raise
                self.publish()
        except BaseException as error:  # The test thread reports the error.
            self.failures.put(error)

    def handle(self, packet: bytes) -> None:
        """Validate and apply one client datagram."""
        if packet.startswith(WIRE_RREF_REQUEST_HEADER):
            if len(packet) != 413:
                raise AssertionError(f"RREF request has {len(packet)} bytes")
            header, rate, index, name = struct.unpack("<5sii400s", packet)
            if header != WIRE_RREF_REQUEST_HEADER:
                raise AssertionError(f"invalid RREF header {header!r}")
            expected = weather.REFS.get(index)
            actual = name.split(b"\x00", 1)[0].decode("ascii")
            if expected != actual:
                raise AssertionError(f"RREF {index} requested {actual!r}")
            self.subscriptions[index] = rate
            if len(self.subscriptions) == len(weather.REFS) and all(
                value == 0 for value in self.subscriptions.values()
            ):
                self.unsubscribed.set()
            return
        if packet.startswith(b"DREF\x00"):
            if len(packet) != 509:
                raise AssertionError(f"DREF request has {len(packet)} bytes")
            generation = struct.unpack_from("<f", packet, 5)[0]
            name = packet[9:].split(b"\x00", 1)[0].decode("ascii")
            if name != "pilotage/weather/clear_generation" or generation != 7.0:
                raise AssertionError(f"invalid clear write {name}={generation}")
            self.accept_clear()
            return
        raise AssertionError(f"unknown datagram header {packet[:5]!r}")

    def accept_clear(self) -> None:
        """Publish either the selected refusal or a complete region ACK."""
        self.write_seen.set()
        self.values[weather.RESPONSE_GENERATION] = 7.0
        if self.refusal_status is not None:
            self.values[weather.STATUS] = self.refusal_status
            return
        self.values.update(
            {
                weather.APPLIED_GENERATION: 7.0,
                weather.RESPONSE_OPERATION: weather.CLEAR_OPERATION,
                weather.RESPONSE_WIND_SPEED: 0.0,
                weather.RESPONSE_WIND_DIRECTION: 0.0,
                weather.RESPONSE_TURBULENCE: 0.0,
                weather.STATUS: weather.CLEAR_STATUS,
            }
        )

    def publish(self) -> None:
        """Send current values at a test-controlled observation boundary."""
        if self.client is None:
            return
        entries = []
        for index, rate in self.subscriptions.items():
            if rate <= 0 or index not in self.values:
                continue
            if self.write_seen.is_set() and index in weather.ACTUAL_REFS:
                send_same_ack_actual = (
                    self.same_ack_actual_only and not self.same_ack_actual_sent
                )
                if not self.release_actual.is_set() and not send_same_ack_actual:
                    continue
            entries.append(struct.pack("<if", index, self.values[index]))
        if entries:
            self.response_socket.sendto(
                WIRE_RREF_RESPONSE_HEADER + b"".join(entries), self.client
            )
            if self.write_seen.is_set():
                self.ack_sent.set()
                self.same_ack_actual_sent = True

    def close(self) -> None:
        """Stop the peer and surface its thread failure."""
        self.stop.set()
        self.socket.close()
        self.response_socket.close()
        self.thread.join(timeout=2.0)
        if self.thread.is_alive():
            raise AssertionError("mock X-Plane peer did not stop")
        if not self.failures.empty():
            raise self.failures.get_nowait()


def run_clear(
    peer: MockXPlane,
    clear_timeout_s: float = 1.0,
    use_default_response: bool = False,
) -> tuple[threading.Event, queue.Queue[object]]:
    """Run one client transaction without blocking the test thread."""
    done = threading.Event()
    result: queue.Queue[object] = queue.Queue()

    def target() -> None:
        try:
            response_address = (
                None if use_default_response else peer.response_address
            )
            result.put(
                weather.clear_weather(
                    peer.address,
                    1.0,
                    clear_timeout_s,
                    response_address=response_address,
                )
            )
        except BaseException as error:
            result.put(error)
        finally:
            done.set()

    threading.Thread(target=target, daemon=True).start()
    return done, result


class WeatherClearTests(unittest.TestCase):
    """Weather clear protocol behavior tests."""

    def test_clear_requires_fresh_actual_samples_after_ack(self) -> None:
        peer = MockXPlane()
        try:
            done, result = run_clear(peer)
            self.assertTrue(peer.write_seen.wait(1.0))
            self.assertTrue(peer.ack_sent.wait(1.0))
            self.assertFalse(done.is_set(), "pre-write actual values cannot prove calm")
            peer.release_actual.set()
            self.assertTrue(done.wait(2.0))
            self.assertEqual(result.get_nowait(), 7)
            self.assertTrue(peer.unsubscribed.wait(1.0))
        finally:
            peer.close()

    def test_default_response_port_reaches_the_xplane_sender(self) -> None:
        peer = MockXPlane()
        self.assertEqual(weather.DEFAULT_RESPONSE_PORT, 49001)
        prior_port = weather.DEFAULT_RESPONSE_PORT
        weather.DEFAULT_RESPONSE_PORT = peer.response_address[1]
        try:
            done, result = run_clear(peer, use_default_response=True)
            self.assertTrue(peer.write_seen.wait(1.0))
            self.assertTrue(peer.ack_sent.wait(1.0))
            peer.release_actual.set()
            self.assertTrue(done.wait(2.0))
            self.assertEqual(result.get_nowait(), 7)
        finally:
            weather.DEFAULT_RESPONSE_PORT = prior_port
            peer.close()

    def test_negative_transaction_status_fails_closed(self) -> None:
        peer = MockXPlane(refusal_status=-3.0)
        try:
            done, result = run_clear(peer)
            self.assertTrue(done.wait(2.0))
            error = result.get_nowait()
            self.assertIsInstance(error, weather.WeatherClearError)
            self.assertIn("refused clear generation 7", str(error))
            self.assertTrue(peer.unsubscribed.wait(1.0))
        finally:
            peer.close()

    def test_same_ack_frame_actual_values_do_not_prove_convergence(self) -> None:
        peer = MockXPlane(same_ack_actual_only=True)
        try:
            done, result = run_clear(peer, clear_timeout_s=0.1)
            self.assertTrue(done.wait(1.0))
            error = result.get_nowait()
            self.assertIsInstance(error, weather.WeatherClearError)
            self.assertIn("did not converge", str(error))
            self.assertTrue(peer.unsubscribed.wait(1.0))
        finally:
            peer.close()

    def test_wrong_source_is_ignored_and_truncated_frame_is_rejected(self) -> None:
        packet = WIRE_RREF_RESPONSE_HEADER + struct.pack(
            "<if", weather.PROTOCOL, 1.0
        )
        self.assertEqual(
            weather.decode_rref(packet, ("127.0.0.1", 1), ("127.0.0.1", 2)),
            {},
        )
        with self.assertRaisesRegex(weather.WeatherClearError, "truncated"):
            weather.decode_rref(packet + b"x", ("127.0.0.1", 2), ("127.0.0.1", 2))
        with self.assertRaisesRegex(weather.WeatherClearError, "invalid RREF header"):
            weather.decode_rref(
                WIRE_RREF_REQUEST_HEADER
                + struct.pack("<if", weather.PROTOCOL, 1.0),
                ("127.0.0.1", 2),
                ("127.0.0.1", 2),
            )

    def test_request_and_response_headers_follow_the_wire_contract(self) -> None:
        request = weather.rref_request(
            10, weather.PROTOCOL, weather.REFS[weather.PROTOCOL]
        )
        self.assertEqual(request[:5], WIRE_RREF_REQUEST_HEADER)
        packet = WIRE_RREF_RESPONSE_HEADER + struct.pack(
            "<if", weather.PROTOCOL, weather.PROTOCOL_VERSION
        )
        self.assertEqual(
            weather.decode_rref(packet, ("127.0.0.1", 2), ("127.0.0.1", 2)),
            {weather.PROTOCOL: weather.PROTOCOL_VERSION},
        )

    def test_unknown_response_index_cannot_change_contract_values(self) -> None:
        packet = WIRE_RREF_RESPONSE_HEADER + struct.pack("<if", 999, 42.0)
        self.assertEqual(
            weather.decode_rref(packet, ("127.0.0.1", 2), ("127.0.0.1", 2)),
            {},
        )

    def test_actual_calm_bounds_reject_invalid_measurements(self) -> None:
        valid = {
            weather.ACTUAL_SPEED: weather.WIND_TOLERANCE_MPS,
            weather.ACTUAL_VERTICAL: -weather.WIND_TOLERANCE_MPS,
            weather.ACTUAL_TURBULENCE_PROFILE_MAX: (
                weather.TURBULENCE_TOLERANCE
            ),
        }
        self.assertTrue(weather.actual_is_calm(valid))
        valid[weather.ACTUAL_VERTICAL] = weather.WIND_TOLERANCE_MPS
        self.assertTrue(weather.actual_is_calm(valid))
        for index, invalid_value in [
            (weather.ACTUAL_SPEED, -0.01),
            (weather.ACTUAL_SPEED, float("nan")),
            (weather.ACTUAL_VERTICAL, float("inf")),
            (weather.ACTUAL_TURBULENCE_PROFILE_MAX, -0.01),
        ]:
            invalid = dict(valid)
            invalid[index] = invalid_value
            self.assertFalse(weather.actual_is_calm(invalid))

    def test_response_endpoint_must_be_loopback_with_a_valid_port(self) -> None:
        for response_address in [("192.0.2.1", 49001), ("127.0.0.1", 0)]:
            with self.assertRaisesRegex(
                weather.WeatherClearError, "response address must be loopback"
            ):
                weather.clear_weather(response_address=response_address)

    def test_cli_passes_the_selected_response_port(self) -> None:
        captured: dict[str, tuple[str, int]] = {}

        def clear(
            address: tuple[str, int],
            *,
            response_address: tuple[str, int],
        ) -> int:
            captured["request"] = address
            captured["response"] = response_address
            return 7

        argv = [
            "xplane_weather_clear.py",
            "--host",
            "127.0.0.1",
            "--port",
            "49010",
            "--response-port",
            "49011",
        ]
        with mock.patch.object(sys, "argv", argv), mock.patch.object(
            weather, "clear_weather", side_effect=clear
        ), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(weather.main(), 0)
        self.assertEqual(captured["request"], ("127.0.0.1", 49010))
        self.assertEqual(captured["response"], ("127.0.0.1", 49011))


if __name__ == "__main__":
    unittest.main()
