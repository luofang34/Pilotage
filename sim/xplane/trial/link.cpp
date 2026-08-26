#include "link.h"

#include "XPLMProcessing.h"

#include <algorithm>
#include <arpa/inet.h>
#include <cerrno>
#include <cstdint>
#include <cstdlib>
#include <fcntl.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <unistd.h>

namespace pilotage::trial {
namespace {

constexpr unsigned kDefaultPort = 45991;
constexpr double kRetryIntervalS = 1.0;
constexpr std::size_t kMaximumInputBytes = 64 * 1024;
constexpr std::size_t kMaximumReplyBytes = 64 * 1024;

unsigned Port() {
    const char* text = std::getenv("PILOTAGE_XPLANE_TRIAL_PORT");
    if (text == nullptr || *text == '\0') {
        return kDefaultPort;
    }
    char* end = nullptr;
    const unsigned long value = std::strtoul(text, &end, 10);
    if (end == text || *end != '\0' || value == 0 || value > 65535) {
        return kDefaultPort;
    }
    return static_cast<unsigned>(value);
}

bool WouldBlock() {
    return errno == EAGAIN || errno == EWOULDBLOCK;
}

}  // namespace

void HostLink::Pump() {
    if (fd_ < 0) {
        Connect();
        return;
    }
    char buffer[4096];
    for (;;) {
        const ssize_t size = ::recv(fd_, buffer, sizeof(buffer), 0);
        if (size > 0) {
            input_.insert(input_.end(), buffer, buffer + size);
            if (input_.size() > kMaximumInputBytes) {
                Disconnect();
                return;
            }
            continue;
        }
        if (size == 0 || (size < 0 && !WouldBlock())) {
            Disconnect();
            return;
        }
        break;
    }
    Flush();
}

bool HostLink::TakeLine(std::string* line) {
    if (line == nullptr) {
        return false;
    }
    const auto end = std::find(input_.begin(), input_.end(), '\n');
    if (end == input_.end()) {
        return false;
    }
    line->assign(input_.begin(), end);
    input_.erase(input_.begin(), end + 1);
    return true;
}

void HostLink::SendReply(const std::string& line) {
    if (line.size() + 1 > kMaximumReplyBytes - replies_.size()) {
        Disconnect();
        return;
    }
    replies_.append(line);
    replies_.push_back('\n');
    Flush();
}

void HostLink::SendSample(const std::string& line) {
    if (sample_sent_ != 0) {
        return;
    }
    sample_ = line;
    sample_.push_back('\n');
    Flush();
}

void HostLink::Close() {
    Disconnect();
    next_attempt_s_ = 0.0;
}

void HostLink::Connect() {
    const double now = XPLMGetElapsedTime();
    if (now < next_attempt_s_) {
        return;
    }
    next_attempt_s_ = now + kRetryIntervalS;
    const int socket_fd = ::socket(AF_INET, SOCK_STREAM, 0);
    if (socket_fd < 0) {
        return;
    }
    sockaddr_in address{};
    address.sin_family = AF_INET;
    address.sin_port = htons(static_cast<std::uint16_t>(Port()));
    address.sin_addr.s_addr = inet_addr("127.0.0.1");
    if (::connect(socket_fd, reinterpret_cast<sockaddr*>(&address),
                  sizeof(address)) != 0) {
        ::close(socket_fd);
        return;
    }
    int enabled = 1;
    ::setsockopt(socket_fd, IPPROTO_TCP, TCP_NODELAY, &enabled,
                 sizeof(enabled));
#ifdef SO_NOSIGPIPE
    ::setsockopt(socket_fd, SOL_SOCKET, SO_NOSIGPIPE, &enabled,
                 sizeof(enabled));
#endif
    ::fcntl(socket_fd, F_SETFL,
            ::fcntl(socket_fd, F_GETFL, 0) | O_NONBLOCK);
    fd_ = socket_fd;
    input_.clear();
    replies_.clear();
    sample_.clear();
    replies_sent_ = 0;
    sample_sent_ = 0;
}

void HostLink::Flush() {
    while (fd_ >= 0) {
        std::string* active = replies_sent_ < replies_.size() ? &replies_ : &sample_;
        std::size_t* sent = active == &replies_ ? &replies_sent_ : &sample_sent_;
        if (*sent >= active->size()) {
            if (active == &replies_) {
                replies_.clear();
                replies_sent_ = 0;
                if (sample_.empty()) {
                    return;
                }
                continue;
            }
            sample_.clear();
            sample_sent_ = 0;
            return;
        }
        const ssize_t size = ::send(fd_, active->data() + *sent,
                                    active->size() - *sent, 0);
        if (size > 0) {
            *sent += static_cast<std::size_t>(size);
            continue;
        }
        if (size < 0 && WouldBlock()) {
            return;
        }
        Disconnect();
        return;
    }
}

void HostLink::Disconnect() {
    if (fd_ >= 0) {
        ::close(fd_);
    }
    fd_ = -1;
    input_.clear();
    replies_.clear();
    sample_.clear();
    replies_sent_ = 0;
    sample_sent_ = 0;
}

HostLink& Link() {
    static HostLink link;
    return link;
}

}  // namespace pilotage::trial
