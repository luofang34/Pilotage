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
#include <fcntl.h>
#include <unistd.h>

#include <algorithm>
#include <cassert>
#include <cstdio>
#include <cstdlib>
#include <string>

// The one simulator symbol `link.cpp` reads, under the test's control so a
// stall can be aged without the test waiting out a real one.
namespace {
float g_now_s = 0.0F;
}

extern "C" float XPLMGetElapsedTime() { return g_now_s; }

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
    // Small, so a backlog stands after a handful of samples rather than after
    // a megabyte. What is under test is the policy, not the kernel's buffer.
    int window = 4096;
    ::setsockopt(listener, SOL_SOCKET, SO_RCVBUF, &window, sizeof(window));
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

/// Accepts the link's connection, with reads that never block.
///
/// The link stops sending once its bounded send buffer fills, so a blocking
/// read here would wait for bytes that are deliberately not coming.
int AcceptNonBlocking(int listener) {
    const int peer = ::accept(listener, nullptr, nullptr);
    assert(peer >= 0);
    ::fcntl(peer, F_SETFL, ::fcntl(peer, F_GETFL, 0) | O_NONBLOCK);
    return peer;
}

/// Reads everything the peer has to offer right now.
std::string DrainAll(int peer) {
    std::string received;
    char buffer[4096];
    for (;;) {
        const ssize_t size = ::recv(peer, buffer, sizeof(buffer), 0);
        if (size <= 0) {
            return received;
        }
        received.append(buffer, static_cast<std::size_t>(size));
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
    const int peer = AcceptNonBlocking(listener);
    assert(link.connected());

    constexpr int kSamples = 200;
    for (int index = 0; index < kSamples; ++index) {
        link.SendSample("SAMPLE " + std::to_string(index));
    }

    // Reads return nothing whenever the link is momentarily quiet, so this
    // spins rather than treating an empty read as the end of the stream.
    const std::string last = "SAMPLE " + std::to_string(kSamples - 1) + "\n";
    std::string received;
    for (int spin = 0; spin < 100000; ++spin) {
        received += DrainAll(peer);
        if (received.find(last) != std::string::npos) {
            break;
        }
        link.Pump();
    }
    assert(received.find(last) != std::string::npos);

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
///
/// Bounded by TIME, so this ages the clock rather than sending until some
/// number of bytes accumulates: how many samples fit in a buffer depends on
/// whether the vehicle is sitting still or manoeuvring, and how fast they
/// arrive depends on the frame rate.
void AConsumerThatStopsDrainingCostsTheConnection() {
    g_now_s = 0.0F;
    const int listener = ListenOnEphemeralPort();
    HostLink link;
    link.Pump();
    const int peer = AcceptNonBlocking(listener);
    assert(link.connected());

    // Never read from `peer`, so the socket buffers fill and a backlog
    // stands. Stop as soon as one does: what follows is about time, and
    // sending further would eventually meet the allocation guard instead.
    const std::string sample(512, 'x');
    for (int index = 0; index < 20000 && link.buffered_sample_bytes() == 0; ++index) {
        link.SendSample(sample);
        assert(link.connected());
    }
    assert(link.connected());
    assert(link.buffered_sample_bytes() > 0);

    // With the clock held still, the backlog may grow well past what a flat
    // 256 KB guard would have allowed and the link must stay up: how long a
    // consumer may stall is the clock's decision, and a byte bound reached
    // first would silently halve the tolerance this states.
    std::size_t produced = 0;
    while (produced < 400 * 1024) {
        link.SendSample(sample);
        produced += sample.size() + 1;
        assert(link.connected());
    }
    assert(link.unsent_sample_bytes() > 256 * 1024);

    // Still behind a moment later, which is not yet a stall.
    g_now_s = 9.0F;
    link.SendSample(sample);
    assert(link.connected());

    // Past the bound, it is.
    g_now_s = 11.0F;
    link.SendSample(sample);
    assert(!link.connected());

    ::close(peer);
    ::close(listener);
}

/// A consumer that keeps pace without ever catching up must not grow the
/// buffer without limit.
///
/// The backlog stays small, so the byte cap is content and the stall clock
/// keeps being reset. What grows is the sent prefix behind them: released only
/// on a COMPLETE drain, it would gain a sample every frame for the length of
/// the trial, inside the simulator's own process.
void ASlowConsumerDoesNotGrowTheBufferForever() {
    g_now_s = 0.0F;
    const int listener = ListenOnEphemeralPort();
    HostLink link;
    link.Pump();
    const int peer = AcceptNonBlocking(listener);
    assert(link.connected());

    // Drain less than is produced, so the socket fills, sends start going out
    // in part, and the link comes to rest holding a partly-sent buffer. That
    // is the regime where a sent prefix would accumulate.
    // The clock is held still and the loop kept short on purpose: this is
    // about the buffer, and a run that drifted into either the stall bound or
    // the allocation guard would be testing those instead — and would start
    // failing on a machine whose socket buffers differ. At a deficit of about
    // 225 bytes an iteration this ends around 225 KB, comfortably inside a
    // guard that holds ten seconds at the fastest rate a sample can be made.
    const std::string sample(480, 'y');
    std::size_t partial_rests = 0;
    char sink[256];
    for (int index = 0; index < 1000 && link.connected(); ++index) {
        link.SendSample(sample);
        ::recv(peer, sink, sizeof(sink), 0);
        // At rest the buffer holds exactly what has not been sent. Anything
        // more is a prefix already on the wire that nothing released, and it
        // grows by a sample per frame for as long as the trial runs.
        assert(link.buffered_sample_bytes() == link.unsent_sample_bytes());
        if (link.unsent_sample_bytes() > 0) {
            partial_rests += 1;
        }
    }

    // The assertion above is only worth anything if the loop actually reached
    // the regime it describes.
    assert(partial_rests > 100);

    ::close(peer);
    ::close(listener);
}

}  // namespace

int main() {
    QueuedSamplesAllArrive();
    AConsumerThatStopsDrainingCostsTheConnection();
    ASlowConsumerDoesNotGrowTheBufferForever();
    return 0;
}
