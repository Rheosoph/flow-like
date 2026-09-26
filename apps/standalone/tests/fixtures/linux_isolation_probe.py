import ctypes
import errno
import os
import signal
import socket
import time

# Invoked only by the ignored Linux kernel acceptance test, inside the worker
# boundary. The parent creates and removes its own quota-backed fixture mount.
mode = os.environ["FLOW_LIKE_TEST_MODE"]
data = os.environ["FLOW_LIKE_TEST_DATA"]

if mode == "boundary":
    assert os.getppid() == 1, "missing namespace init"
    status = open("/proc/self/status").read()
    assert "NoNewPrivs:\t1" in status and "Seccomp:\t2" in status
    assert "CapEff:\t0000000000000000" in status
    for path in [os.environ["FLOW_LIKE_TEST_SECRET"], os.environ["FLOW_LIKE_TEST_SIBLING"]]:
        assert not os.path.exists(path), "private host path is visible"
        assert not os.path.exists("/proc/1/root" + path), "proc root escape"
    project = os.environ["FLOW_LIKE_TEST_PROJECT"]
    assert open(project + "/public").read() == "project"
    assert open(project + "/.secrets/acceptance/token").read() == "own setting"
    assert not os.path.exists(project + "/.secrets/other-placement/token")
    try:
        open(project + "/modified", "w").close()
        raise AssertionError("read-only project accepted a write")
    except OSError as error:
        assert error.errno in (errno.EROFS, errno.EACCES)
    os.symlink(os.environ["FLOW_LIKE_TEST_SECRET"], data + "/escape")
    assert not os.path.exists(data + "/escape"), "symlink escape"
    os.unlink(data + "/escape")
    libc = ctypes.CDLL(None, use_errno=True)
    for syscall in [425, 426, 427]:  # io_uring on supported x86_64 and aarch64.
        assert libc.syscall(syscall, 0, 0, 0, 0, 0, 0) == -1
        assert ctypes.get_errno() == errno.EPERM
    assert libc.ptrace(16, int(os.environ["FLOW_LIKE_TEST_PARENT"]), 0, 0) == -1
    assert ctypes.get_errno() == errno.EPERM
    assert libc.unshare(0x10000000) == -1
    assert ctypes.get_errno() == errno.EPERM
    # FSSETXATTR could otherwise move new files outside their project quota.
    fd = os.open(data, os.O_RDONLY | os.O_DIRECTORY)
    assert libc.ioctl(fd, 0x401C5820, 0) == -1
    assert ctypes.get_errno() == errno.EPERM
    os.close(fd)
    abstract = socket.socket(socket.AF_UNIX)
    try:
        abstract.connect("\0" + os.environ["FLOW_LIKE_TEST_ABSTRACT"])
        raise AssertionError("cross-boundary abstract Unix connection")
    except OSError as error:
        assert error.errno == errno.EPERM
    finally:
        abstract.close()
    private = socket.socket(socket.AF_UNIX)
    private.bind("/tmp/replica.sock")
    private.listen(1)
    client = socket.socket(socket.AF_UNIX)
    client.connect("/tmp/replica.sock")
    accepted, _ = private.accept()
    client.sendall(b"replica")
    assert accepted.recv(7) == b"replica"
    accepted.close()
    client.close()
    private.close()
    os.unlink("/tmp/replica.sock")
    # These are the production supervisor's inherited broker, read-only lock,
    # and hosted listener slots. No other host file is writable through a slot.
    assert os.write(3, b"broker") == 6
    try:
        os.fstat(int(os.environ["FLOW_LIKE_TEST_AMBIENT_FD"]))
        raise AssertionError("ambient host descriptor survived exec")
    except OSError as error:
        assert error.errno == errno.EBADF
    try:
        os.write(4, b"forbidden")
        raise AssertionError("placement lock descriptor is writable")
    except OSError as error:
        assert error.errno == errno.EBADF
    listener = socket.socket(fileno=5)
    listener.settimeout(5)
    connection, _ = listener.accept()
    assert connection.recv(6) == b"hosted"
    connection.sendall(b"served")
    connection.close()
    listener.close()

elif mode == "disk":
    path = data + "/quota-probe"
    try:
        with open(path, "wb", buffering=0) as output:
            for _ in range(32):
                output.write(b"a" * 1024 * 1024)
                os.fsync(output.fileno())
        raise AssertionError("disk quota did not stop writes")
    except OSError as error:
        assert error.errno == errno.EDQUOT, str(error)
    finally:
        os.unlink(path)

elif mode == "memory":
    allocations = []
    for _ in range(256):
        allocations.append(bytearray(2 * 1024 * 1024))
    raise AssertionError("memory limit did not terminate the cgroup")

elif mode == "pids":
    children = []
    try:
        for _ in range(64):
            try:
                pid = os.fork()
            except OSError as error:
                assert error.errno == errno.EAGAIN
                break
            if pid == 0:
                time.sleep(30)
                os._exit(0)
            children.append(pid)
        else:
            raise AssertionError("process limit did not stop forks")
    finally:
        for pid in children:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)

elif mode == "cpu":
    wall = time.monotonic()
    used = time.process_time()
    while time.process_time() - used < 0.35:
        pass
    assert time.monotonic() - wall > 1.8, "CPU quota did not throttle execution"

else:
    raise AssertionError("unknown probe")

print(mode + ":passed")
