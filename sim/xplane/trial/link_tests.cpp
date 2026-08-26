// What the link does with samples the host is not reading yet.
//
// The host requires the sample sequence to be contiguous and fails the stream
// on any gap, so what this file pins is that the link never produces one
// quietly: it queues while the consumer is behind, and gives up the connection
// outright when the consumer has stopped draining altogether.
//
// `link.cpp` needs exactly one simulator symbol, so the whole of it is
// reachable from a test binary with that symbol stubbed below.

#include "link.h"

#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <unistd.h>

#include <cassert>
#include <cstdio>
#include <cstdlib>
#include <string>

// The one simulator symbol `link.cpp` reads. Advancing it past the retry
// interval on every call means a reconnect is never refused for being early.
extern "C" float XPLMGetElapsedTime() {
    static float now = 0.0F;
    now += 10.0F;
    return now;
}

namespace {

using pilotage::trial::HostLink;

/// A listening socket on a port the kernel chooses, exported so the link
/// connects to this test rather than to a real simulator.
int ListenOnEphemeralPort() {
    const int listener = ::socket(AF_INET, SOCK_STREAM, 0);
    assert(listener >= 0);
    int reuse = 1;
    ::setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &reuse, sizeof(reuse));
    sockaddr_in address{};
    address.sin_family = AF_INET;
    address.sin_addr.s_addr = inet_addr("127.0.0.1");
    address.sin_port = 0;
    assert(::bind(listener, reinterpret_cast<sockaddr*>(&address),
                  sizeof(address)) == 0);
    assert(::listen(listener, 4) == 0);

    sockaddr_in bound{};
    socklen_t length = sizeof(bound);
    assert(::getsockname(listener, reinterpret_cast<sockaddr*>(&bound),
                         &length) == 0);
    char port[16]{};
    std::snprintf(port, sizeof(port), "%u", ntohs(bound.sin_port));
    ::setenv("PILOTAGE_XPLANE_TRIAL_PORT", port, 1);
    return listener;
}

/// Reads everything the peer has sent so far, blocking until at least one
/// read returns nothing more.
std::string DrainAll(int peer) {
    std::string received;
    char buffer[4096];
    for (;;) {
        const ssize_t size = ::recv(peer, buffer, sizeof(buffer), 0);
        if (size <= 0) {
            return received;
        }
        received.append(buffer, static_cast<std::size_t>(size));
        if (static_cast<std::size_t>(size) < sizeof(buffer)) {
            return received;
        }
    }
}

/// Every sample handed to a connected link reaches the wire, in order.
///
/// The previous behaviour kept ONE sample: a second handed over while the
/// first was still unsent replaced it. The host counts on a contiguous
/// sequence, so that silently produced a stream it would later reject.
void QueuedSamplesAllArrive() {
    const int listener = ListenOnEphemeralPort();
    HostLink link;
    link.Pump();
    const int peer = ::accept(listener, nullptr, nullptr);
    assert(peer >= 0);
    assert(link.connected());

    constexpr int kSamples = 200;
    for (int index = 0; index < kSamples; ++index) {
        link.SendSample("SAMPLE " + std::to_string(index));
    }

    std::string received;
    while (true) {
        const std::string chunk = DrainAll(peer);
        if (chunk.empty()) {
            break;
        }
        received += chunk;
        if (received.find("SAMPLE " + std::to_string(kSamples - 1) + "\n") !=
            std::string::npos) {
            break;
        }
        link.Pump();
    }

    std::size_t at = 0;
    for (int index = 0; index < kSamples; ++index) {
        const std::string expected = "SAMPLE " + std::to_string(index) + "\n";
        const std::size_t found = received.find(expected, at);
        assert(found != std::string::npos);
        assert(found == at);
        at = found + expected.size();
    }

    link.Close();
    ::close(peer);
    ::close(listener);
}

/// A consumer that has stopped reading costs the connection, not the record.
///
/// The host reads a closed peer and says so. A sample dropped instead would
/// leave a hole the host meets later as a broken sequence, blaming the wire
/// for a consumer that stalled.
void ABacklogThatStopsDrainingClosesTheLink() {
    const int listener = ListenOnEphemeralPort();
    HostLink link;
    link.Pump();
    const int peer = ::accept(listener, nullptr, nullptr);
    assert(peer >= 0);
    assert(link.connected());

    // Never read from `peer`. Once its receive buffer and the sender's send
    // buffer are both full, the queue is all that is left to grow.
    const std::string sample(512, 'x');
    for (int index = 0; index < 20000 && link.connected(); ++index) {
        link.SendSample(sample);
    }

    assert(!link.connected());

    ::close(peer);
    ::close(listener);
}

}  // namespace

int main() {
    QueuedSamplesAllArrive();
    ABacklogThatStopsDrainingClosesTheLink();
    return 0;
}
