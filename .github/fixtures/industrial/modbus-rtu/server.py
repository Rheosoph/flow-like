"""Native PyModbus RTU server with observations of its decoded requests and store."""

import asyncio
import json
import logging
import os
from pathlib import Path

from pymodbus import FramerType
from pymodbus.server import ModbusSerialServer

from modbus_server import context


async def main():
    store = context()
    directory = Path(os.environ["FLOW_LIKE_MODBUS_RTU_E2E_STATE"])
    requests = []
    corruptions = 0

    def snapshot():
        data = {
            "requests": requests,
            "corruptions": corruptions,
            "devices": {
                str(unit): {
                    name: store[unit].getValues(function, 0, 64)
                    for name, function in (("coils", 1), ("discrete_inputs", 2),
                                           ("holding_registers", 3), ("input_registers", 4))
                }
                for unit in (1, 7)
            },
        }
        temporary = directory / "observed.tmp"
        temporary.write_text(json.dumps(data))
        temporary.replace(directory / "observed.json")

    def trace_pdu(sending, pdu):
        if not sending:
            # This hook runs after the upstream RTU decoder has validated the CRC.
            requests.append({"unit": pdu.dev_id, "function": pdu.function_code})
        snapshot()
        return pdu

    def trace_packet(sending, packet):
        nonlocal corruptions
        fault = directory / "corrupt-next-crc"
        if sending and fault.exists():
            fault.unlink()
            corruptions += 1
            # Fault injection changes only one CRC byte after PyModbus frames the reply.
            packet = packet[:-1] + bytes([packet[-1] ^ 0x80])
            snapshot()
        return packet

    # Raw PTYs carry real RTU bytes, but do not model RS485 voltage, parity errors,
    # half-duplex turnaround, electrical collision, or physical character timing.
    server = ModbusSerialServer(
        store, framer=FramerType.RTU, port="/tmp/modbus-server",
        baudrate=19200, bytesize=8, parity="N", stopbits=2,
        ignore_missing_devices=True, broadcast_enable=False,
        trace_pdu=trace_pdu, trace_packet=trace_packet,
    )
    await server.serve_forever(background=True)
    snapshot()
    (directory / "ready").touch()
    try:
        await asyncio.Event().wait()
    finally:
        await server.shutdown()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    asyncio.run(main())
