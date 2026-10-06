import errno
import os
import socket
import sys

if sys.argv[1] == "memory":
    allocation = bytearray(256 * 1024 * 1024)
    raise AssertionError("the engine memory limit did not stop allocation")

assert os.stat("/proc/self/ns/net").st_ino != int(os.environ["OUTSIDE_NET"])
with open("/proc/self/mountinfo") as mounts:
    socket_dir = os.path.dirname(os.environ["ENGINE_SOCKET"])
    assert any(
        line.split()[4] == socket_dir and line.split(" - ", 1)[1].split()[0] == "tmpfs"
        for line in mounts
    ), "the writable engine socket mount must be tmpfs"
try:
    os.fstat(int(os.environ["INHERITED_FD"]))
except OSError as error:
    assert error.errno == errno.EBADF
else:
    raise AssertionError("engine inherited a host network descriptor")

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as internet:
    try:
        internet.bind(("0.0.0.0", 0))
    except PermissionError:
        pass
    else:
        raise AssertionError("engine can bind an Internet listener")

try:
    socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
except PermissionError:
    pass
else:
    raise AssertionError("engine can open a UDP socket")

outside = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
outside.settimeout(1)
try:
    outside.connect(("127.0.0.1", int(os.environ["OUTSIDE_PORT"])))
except OSError:
    pass
else:
    raise AssertionError("engine reached the host network")
finally:
    outside.close()

with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
    listener.bind(os.environ["ENGINE_SOCKET"])
    listener.listen(1)
    with listener.accept()[0] as request:
        request.recv(4096)
        request.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nready")
