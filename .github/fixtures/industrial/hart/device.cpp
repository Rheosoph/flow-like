#include <string>
#include <vector>
#include <cstring>
#include "appconnector.h"

template<> bool AppConnector<AppPdu>::time2stop = false;

class FixtureDevice : public App {
public:
    FixtureDevice() : App("Flow-Like HART fixture ", "1.0", HART_IP) {}
    errVal_t ready() override { return NO_ERROR; }
    errVal_t stop() override { return NO_ERROR; }

    int handleMessage(AppPdu* pdu) override {
        const uint16_t command = pdu->CmdNum();
        std::vector<uint8_t> response;
        switch (command) {
        case 0:
            response = {254, 0x12, 0x34, 5, 7, 1, 1, 0, 0, 1, 2, 3, 5, 0, 0, 0, 0, 0};
            break;
        case 1:
            // Unit code followed by IEEE-754 big-endian 21.5.
            response = {32, 0x41, 0xac, 0, 0};
            break;
        case 20:
        case 520:
            response.resize(32);
            std::memcpy(response.data(), "flow-like-fixture", 17);
            break;
        case 128:
            response.assign(pdu->RequestBytes(), pdu->RequestBytes() + pdu->RequestByteCount());
            break;
        default:
            pdu->ProcessErrResponse(64);
            return NO_ERROR;
        }
        pdu->setSavedDeviceStatus(0);
        pdu->ProcessOkResponse(0, response.size() + 2 + (command > 255 ? 2 : 0));
        std::memcpy(pdu->ResponseBytes(), response.data(), response.size());
        pdu->InsertCheckSum();
        return NO_ERROR;
    }
};

int main() {
    p_toolLogPtr = stderr;
    FixtureDevice device;
    AppConnector<AppPdu> connection;
    connection.run(&device);
    connection.cleanup();
}
