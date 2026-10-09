"""Start the vendor router before its symbolic server and reap both on shutdown."""

import os
import signal
import socket
import struct
import subprocess
import sys
import time
from pathlib import Path

children = []


def shutdown(*_):
    for process in reversed(children):
        process.terminate()
    for process in children:
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
signal.signal(signal.SIGINT, lambda *_: sys.exit(0))
try:
    # The vendor router's route table needs literal addresses, including the
    # bridge gateway that Docker uses for forwarded host connections.
    routes = [line.split() for line in Path("/proc/net/route").read_text().splitlines()[1:]]
    gateway = next(route[2] for route in routes if route[1] == "00000000")
    os.environ["AmsRouter__RemoteConnections__0__Address"] = socket.inet_ntoa(
        struct.pack("<I", int(gateway, 16))
    )
    os.environ["AmsRouter__RemoteConnections__1__Address"] = socket.gethostbyname(
        socket.gethostname()
    )
    children.append(subprocess.Popen(["dotnet", "/app/router/AdsRouterConsole.dll"]))
    deadline = time.monotonic() + 30
    while True:
        if children[0].poll() is not None:
            raise RuntimeError("ADS router stopped before startup")
        try:
            socket.create_connection(("127.0.0.1", 48900), timeout=1).close()
            break
        except OSError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.1)
    children.append(subprocess.Popen(["dotnet", "/app/server/AdsServer.dll"]))
    while all(process.poll() is None for process in children):
        time.sleep(0.2)
    raise RuntimeError("ADS fixture process exited")
finally:
    shutdown()
