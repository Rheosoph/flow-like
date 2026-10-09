"""Independent Modbus TCP server for the Rust adapter integration tests."""

import asyncio
import logging
import sys

from pymodbus.client import ModbusTcpClient
from pymodbus.datastore import (
    ModbusDeviceContext,
    ModbusSequentialDataBlock,
    ModbusServerContext,
)
from pymodbus.server import StartAsyncTcpServer


def block(pattern):
    # PyModbus 3.11 adds one to wire addresses before accessing a data block.
    # Starting at one exposes exactly wire addresses 0..63.
    return ModbusSequentialDataBlock(
        1, [pattern[index % len(pattern)] for index in range(64)]
    )


def context():
    return ModbusServerContext(
        devices={
            1: ModbusDeviceContext(
                co=block([False]),
                di=block([True]),
                hr=block([0x1111]),
                ir=block([0x2222]),
            ),
            7: ModbusDeviceContext(
                co=block(
                    [True, False, False, True, True, False, True,
                     False, False, True, False, True, True]
                ),
                di=block(
                    [False, True, True, False, False, True, False,
                     True, True, False, True, False, False]
                ),
                hr=block([0, 1, 0x1234, 0x8000, 0xFF00, 0xFFFF, 0x5AA5, 0xA55A]),
                ir=block([0x4321, 0x0100, 0x7FFF, 0x8001, 0x00FF, 0xFFFE, 0x55AA, 0xAA55]),
            ),
        },
        single=False,
    )


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    if sys.argv[1:] == ["--healthcheck"]:
        with ModbusTcpClient("127.0.0.1", port=5020, timeout=2) as client:
            response = client.read_input_registers(0, count=1, device_id=7)
            if response.isError() or response.registers != [0x4321]:
                sys.exit(1)
    else:
        asyncio.run(StartAsyncTcpServer(context(), address=("0.0.0.0", 5020)))
