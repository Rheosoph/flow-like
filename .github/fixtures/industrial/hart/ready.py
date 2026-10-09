import socket
import struct


def receive_exact(connection, count):
    result = bytearray()
    while len(result) < count:
        block = connection.recv(count - len(result))
        if not block:
            raise RuntimeError("HART-IP server closed before its response was complete")
        result.extend(block)
    return bytes(result)


def exchange(connection, identifier, sequence, body=b""):
    connection.sendall(struct.pack("!BBBBHH", 1, 0, identifier, 0, sequence, 8 + len(body)) + body)
    header = receive_exact(connection, 8)
    version, kind, message, status, response_sequence, length = struct.unpack("!BBBBHH", header)
    assert (version, kind, message, status, response_sequence) == (1, 1, identifier, 0, sequence)
    assert 8 <= length <= 4096
    return receive_exact(connection, length - 8)


with socket.create_connection(("127.0.0.1", 5094), timeout=3) as connection:
    exchange(connection, 0, 1, struct.pack("!BI", 1, 30_000))
    # Polling command zero: delimiter, primary-master address, command, length, XOR.
    identity = exchange(connection, 3, 2, bytes.fromhex("02 80 00 00 82"))
    assert identity[0] == 0x06 and identity[2] == 0
    assert identity[3] + 5 == len(identity)
    assert identity[4:9] == bytes([0, 0, 254, 0x12, 0x34])
    assert identity[15:18] == bytes([1, 2, 3])
    checksum = 0
    for byte in identity:
        checksum ^= byte
    assert checksum == 0
    exchange(connection, 1, 3)
