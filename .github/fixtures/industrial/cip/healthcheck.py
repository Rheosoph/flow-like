"""Check the simulator's EtherNet/IP RegisterSession exchange."""

import socket
import struct

with socket.create_connection(("127.0.0.1", 44818), timeout=2) as connection:
    connection.sendall(struct.pack("<HHII8sIHH", 0x65, 4, 0, 0, bytes(8), 0, 1, 0))
    response = bytearray()
    while len(response) < 28:
        chunk = connection.recv(28 - len(response))
        if not chunk:
            raise RuntimeError("RegisterSession response ended early")
        response.extend(chunk)
    command, length, session, status, context, _, version, flags = struct.unpack(
        "<HHII8sIHH", response
    )
    assert (command, length, status, context, version, flags) == (
        0x65, 4, 0, bytes(8), 1, 0
    )
    assert session != 0
