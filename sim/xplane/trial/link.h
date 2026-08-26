#pragma once

#include <cstddef>
#include <string>
#include <vector>

namespace pilotage::trial {

class HostLink {
   public:
    void Pump();
    bool TakeLine(std::string* line);
    void SendReply(const std::string& line);
    void SendSample(const std::string& line);
    void Close();
    bool connected() const { return fd_ >= 0; }

   private:
    void Connect();
    void Flush();
    void Disconnect();

    int fd_ = -1;
    double next_attempt_s_ = 0.0;
    std::vector<char> input_;
    std::string replies_;
    std::string sample_;
    std::size_t replies_sent_ = 0;
    std::size_t sample_sent_ = 0;
};

HostLink& Link();

}  // namespace pilotage::trial
