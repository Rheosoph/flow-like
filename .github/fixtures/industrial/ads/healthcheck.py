"""Read the vendor server's INT through the ADS router."""

import socket
import struct


def receive(connection, length):
    data = bytearray()
    while len(data) < length:
        chunk = connection.recv(length - len(data))
        if not chunk:
            raise RuntimeError("ADS response ended early")
        data.extend(chunk)
    return data


with socket.create_connection((socket.gethostname(), 48898), timeout=2) as connection:
    data = struct.pack("<III", 2, 0x1001, 2)
    ams = struct.pack(
        "<6sH6sHHHIII", bytes([42, 42, 42, 42, 1, 1]), 25000,
        bytes([1, 2, 3, 4, 1, 1]), 59000, 2, 4, len(data), 0, 1,
    )
    connection.sendall(struct.pack("<HI", 0, len(ams) + len(data)) + ams + data)
    reserved, length = struct.unpack("<HI", receive(connection, 6))
    assert reserved == 0 and 40 <= length <= 1024
    response = receive(connection, length)
    assert struct.unpack_from("<I", response, 24)[0] == 0
    assert struct.unpack_from("<II", response, 32) == (0, 2)
