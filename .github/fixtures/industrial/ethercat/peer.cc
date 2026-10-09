#include <algorithm>
#include <csignal>
#include <cstdio>
#include <fstream>
#include <iostream>
#include <stdexcept>
#include <nlohmann/json.hpp>
#include "kickcat/EmulatedNetwork.h"
#include "kickcat/Frame.h"
#include "kickcat/OS/Linux/Socket.h"
#include "kickcat/simulation/SimulatedSlave.h"

using namespace kickcat;
using json = nlohmann::json;
static volatile std::sig_atomic_t running = 1;
static void stop(int) { running = 0; }

int main(int argc, char** argv) {
    if (argc != 4) { return 2; }
    std::signal(SIGINT, stop);
    std::signal(SIGTERM, stop);
    try {
        auto peer = sim::buildSlave(argv[2]);
        // Device application parameters. All SDO parsing, access checks, AL state,
        // EEPROM, FMMU, mailbox and PDO handling remain in upstream KickCAT.
        const CoE::DataType types[] = {CoE::DataType::UNSIGNED8, CoE::DataType::UNSIGNED16,
            CoE::DataType::UNSIGNED32, CoE::DataType::INTEGER8, CoE::DataType::INTEGER16,
            CoE::DataType::INTEGER32};
        const uint16_t bits[] = {8, 16, 32, 8, 16, 32};
        for (int i = 0; i < 6; ++i) {
            CoE::Object object;
            object.index = 0x2000 + i;
            object.code = CoE::ObjectCode::VAR;
            object.name = "Startup parameter";
            CoE::addEntry(object, 0, bits[i], 0, CoE::Access::READ | CoE::Access::WRITE_PREOP,
                          types[i], "Startup parameter", uint32_t{0});
            peer.dictionary->push_back(std::move(object));
        }
        const std::vector<uint8_t> input = {0x00, 0xff, 0x80, 0x7f, 0x34, 0x12,
                                          0x78, 0x56, 0xab, 0xcd, 0xef, 0x42};
        std::copy(input.begin(), input.end(), peer.input.begin());
        EmulatedNetwork network({peer.esc.get()});
        Socket socket;
        socket.open(argv[1]);
        socket.setTimeout(10ms);
        peer.slave->start();
        json states = json::array();
        uint64_t frames = 0;
        bool eeprom_read_pending = false;
        bool delayed_eeprom = false;
        auto save = [&] {
            const auto state = static_cast<unsigned>(peer.slave->state());
            if (states.empty() || states.back() != state) { states.push_back(state); }
            json parameters = json::array();
            for (int i = 0; i < 6; ++i) {
                auto [object, entry] = CoE::findObject(*peer.dictionary, 0x2000 + i, 0);
                if (!object || !entry || !entry->data) { throw std::runtime_error("Missing parameter"); }
                auto bytes = static_cast<uint8_t*>(entry->data);
                parameters.push_back(std::vector<uint8_t>(bytes, bytes + bits[i] / 8));
            }
            json value = {{"ready", true}, {"frames", frames}, {"state", state},
                          {"states", states}, {"parameters", parameters},
                          {"delayed_eeprom", delayed_eeprom},
                          {"outputs", std::vector<uint8_t>(peer.output.begin(), peer.output.begin() + 3)}};
            std::string temporary = std::string(argv[3]) + ".new";
            { std::ofstream out(temporary); out << value.dump(); }
            if (std::rename(temporary.c_str(), argv[3]) != 0) { throw std::runtime_error("Cannot save observation"); }
        };
        save();
        auto saved_at = now();
        while (running) {
            Frame frame;
            const auto size = socket.read(frame.data(), ETH_MAX_SIZE);
            if (size <= 0) { continue; }
            frame.resetContext();
            while (true) {
                auto [header, data, wkc] = frame.peekDatagram();
                if (!header) { break; }
                const auto address = static_cast<uint16_t>(header->address >> 16);
                if (header->command == Command::FPWR && address == reg::EEPROM_CONTROL &&
                    header->len >= 2 && (static_cast<uint8_t*>(data)[1] & 0x01)) {
                    eeprom_read_pending = true;
                }
                if (!delayed_eeprom && eeprom_read_pending && header->command == Command::FPRD &&
                    address == reg::EEPROM_CONTROL) {
                    // A slow status response must respect the configured response timeout.
                    // This transport fault exceeds EtherCrab's default 10 ms EEPROM deadline.
                    sleep(35ms);
                    delayed_eeprom = true;
                }
            }
            if (!network.route(frame, false)) { continue; }
            // Link shim: Beckhoff ESC Technology v2.5, section 3, page I-13,
            // specifies setting SOURCE_MAC[1] on returned frames:
            // https://download.beckhoff.com/download/Document/io/ethercat-development-products/ethercat_esc_datasheet_sec1_technology_v2.5.pdf#page=35
            // KickCAT independently handles datagrams/state/mailbox/PDO, not this bit.
            frame.ethernet()->src[0] |= 0x02;
            if (socket.write(frame.data(), size) != size) { throw std::runtime_error("Frame write failed"); }
            peer.slave->routine();
            if (peer.slave->state() == State::SAFE_OP &&
                std::any_of(peer.output.begin(), peer.output.begin() + 3, [](uint8_t value) { return value != 0xff; })) {
                peer.slave->validateOutputData();
            }
            std::copy(input.begin(), input.end(), peer.input.begin());
            ++frames;
            const auto state = static_cast<unsigned>(peer.slave->state());
            if (states.back() != state || now() - saved_at >= 10ms) {
                save();
                saved_at = now();
            }
        }
        socket.close();
        return 0;
    } catch (const std::exception& error) {
        std::cerr << error.what() << std::endl;
        return 1;
    }
}
