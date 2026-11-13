#!/usr/bin/env python3
"""Machine-wide queue for cargo builds started by coding agents.

bin/cargo forwards every cargo call here. Compiling subcommands (check, build, test,
clippy, doc, bench, fix, clean and the check-*/test-*/clippy-*/build-* aliases) become
jobs in a FIFO queue under ~/.cache/cargo-queue, and one detached runner executes them
one at a time. A request identical to a queued job, or to a running job whose workspace
has no Rust source change since it started, joins that job instead of adding another.
All other cargo commands run directly.

  cargo queue [status]     running, queued and recent jobs
  cargo queue wait <id>    reattach to a job and print its output
  cargo queue cancel <id>  drop a queued job or stop a running one
"""

import fcntl
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from contextlib import contextmanager
from pathlib import Path

HOME = Path(
    os.environ.get("CARGO_QUEUE_HOME") or Path.home() / ".cache" / "cargo-queue"
)
JOBS = HOME / "jobs"
SCRIPT = Path(__file__).resolve()
SHIM_DIR = SCRIPT.parent / "bin"
INSIDE = "CARGO_QUEUE_INSIDE"

QUEUED_SUBCOMMANDS = {
    "b",
    "bench",
    "build",
    "c",
    "check",
    "clean",
    "clippy",
    "d",
    "doc",
    "fix",
    "llvm-cov",
    "miri",
    "nextest",
    "rustc",
    "rustdoc",
    "t",
    "test",
}
QUEUED_ALIAS_PREFIXES = ("build-", "check-", "clippy-", "test-")
GLOBAL_VALUE_FLAGS = {"--color", "--config", "--explain", "-C", "-Z"}
PASSTHROUGH_FLAGS = {"-h", "--help", "-V", "--version", "--list"}
SOURCE_SUFFIXES = (".rs", ".toml", ".lock", ".sql", ".proto", ".wit")
SCAN_SKIP = {"node_modules", "target"}

POLL_SECONDS = 1.0
LIVE_WAITER_SECONDS = 10
ABANDON_AFTER_SECONDS = 120
STOP_GRACE_SECONDS = 20
KEEP_FINISHED_SECONDS = 24 * 3600
KEEP_FINISHED_JOBS = 200
TERMINAL = {"done", "failed", "cancelled", "abandoned"}
EX_TEMPFAIL = 75

dead_fds = set()


def say(message):
    write_fd(2, f"[cargo-queue] {message}\n".encode())


def write_fd(fd, data):
    if fd in dead_fds:
        return
    view = memoryview(data)
    while view:
        try:
            written = os.write(fd, view)
        except OSError:
            dead_fds.add(fd)
            return
        view = view[written:]


def ago(seconds):
    seconds = max(0, int(seconds))
    if seconds < 60:
        return f"{seconds}s"
    if seconds < 3600:
        return f"{seconds // 60}m{seconds % 60:02d}s"
    return f"{seconds // 3600}h{seconds % 3600 // 60:02d}m"


def describe(job):
    command = " ".join(job["args"])
    return command if len(command) <= 90 else command[:87] + "..."


def path_without_shim(path):
    shim = str(SHIM_DIR)
    return os.pathsep.join(
        p for p in path.split(os.pathsep) if p and os.path.realpath(p) != shim
    )


def shim_on_path():
    shim = str(SHIM_DIR)
    return any(
        os.path.realpath(p) == shim
        for p in os.environ.get("PATH", "").split(os.pathsep)
        if p
    )


def queue_command_hint(*words):
    cargo = "cargo" if shim_on_path() else str(SHIM_DIR / "cargo")
    return " ".join([cargo, "queue", *words])


def subcommand(args):
    i = 0
    while i < len(args):
        arg = args[i]
        if arg.startswith("+"):
            i += 1
        elif arg in GLOBAL_VALUE_FLAGS:
            i += 2
        elif arg.startswith("-"):
            i += 1
        else:
            return arg
    return None


def should_queue(args):
    own_args = args[: args.index("--")] if "--" in args else args
    if any(arg in PASSTHROUGH_FLAGS for arg in own_args):
        return False
    name = subcommand(args)
    return name is not None and (
        name in QUEUED_SUBCOMMANDS or name.startswith(QUEUED_ALIAS_PREFIXES)
    )


def exec_cargo(args):
    path = path_without_shim(os.environ.get("PATH", ""))
    cargo = shutil.which("cargo", path=path)
    if cargo is None:
        say(f"no cargo found on PATH outside {SHIM_DIR}")
        return 127
    os.environ["PATH"] = path
    os.execv(cargo, ["cargo", *args])


@contextmanager
def queue_lock():
    JOBS.mkdir(parents=True, exist_ok=True)
    with open(HOME / "queue.lock", "a") as handle:
        fcntl.flock(handle, fcntl.LOCK_EX)
        yield


def job_dir(job_id):
    return JOBS / str(job_id)


def read_job(job_id):
    try:
        return json.loads((job_dir(job_id) / "job.json").read_text())
    except (OSError, ValueError):
        return None


def write_job(job):
    path = job_dir(job["id"]) / "job.json"
    tmp = path.with_name("job.json.tmp")
    tmp.write_text(json.dumps(job))
    os.replace(tmp, path)


def all_jobs():
    if not JOBS.exists():
        return []
    jobs = (
        read_job(int(entry.name)) for entry in JOBS.iterdir() if entry.name.isdigit()
    )
    return sorted((job for job in jobs if job), key=lambda job: job["id"])


def finish(job, state, code, note):
    job.update(state=state, exit=code, finished=time.time(), note=note)
    write_job(job)


def heartbeat(job_id):
    waiters = job_dir(job_id) / "waiters"
    waiters.mkdir(exist_ok=True)
    (waiters / str(os.getpid())).touch()


def heartbeats(job_id):
    waiters = job_dir(job_id) / "waiters"
    times = []
    try:
        entries = list(os.scandir(waiters))
    except OSError:
        return times
    for entry in entries:
        try:
            times.append(entry.stat().st_mtime)
        except OSError:
            pass
    return times


def last_heartbeat(job_id):
    return max(heartbeats(job_id), default=0.0)


def live_waiters(job_id):
    cutoff = time.time() - LIVE_WAITER_SECONDS
    return sum(1 for seen in heartbeats(job_id) if seen >= cutoff)


def job_key(cwd, args, env):
    cargo_env = sorted(
        (k, v)
        for k, v in env.items()
        if k.startswith(("CARGO", "RUST")) and not k.startswith("CARGO_QUEUE")
    )
    return hashlib.sha256(json.dumps([cwd, args, cargo_env]).encode()).hexdigest()


def workspace_root(cwd):
    for directory in (Path(cwd), *Path(cwd).parents):
        if (directory / "Cargo.lock").exists():
            return directory
    return None


def sources_changed_since(root, since):
    stack = [str(root)]
    while stack:
        try:
            entries = list(os.scandir(stack.pop()))
        except OSError:
            continue
        for entry in entries:
            try:
                if entry.is_dir(follow_symlinks=False):
                    hidden = entry.name.startswith(".") and entry.name != ".cargo"
                    if not hidden and entry.name not in SCAN_SKIP:
                        stack.append(entry.path)
                elif (
                    entry.name.endswith(SOURCE_SUFFIXES)
                    and entry.stat(follow_symlinks=False).st_mtime > since
                ):
                    return True
            except OSError:
                continue
    return False


def find_joinable(jobs, key, cwd, args):
    for job in jobs:
        if job["key"] != key:
            continue
        if job["state"] == "queued":
            return job, "queued"
        if job["state"] == "running" and not job.get("cancel"):
            root = workspace_root(cwd)
            explicit_manifest = any(arg.startswith("--manifest-path") for arg in args)
            if (
                root
                and not explicit_manifest
                and not sources_changed_since(root, job["started"])
            ):
                return job, "running"
    return None, None


def create_job(jobs, key, cwd, args, env):
    seq_path = HOME / "seq"
    try:
        last = int(seq_path.read_text())
    except (OSError, ValueError):
        last = 0
    job_id = max(last, max((job["id"] for job in jobs), default=0)) + 1
    seq_path.write_text(str(job_id))
    directory = job_dir(job_id)
    directory.mkdir(parents=True)
    (directory / "env.json").write_text(json.dumps(env))
    job = {
        "id": job_id,
        "key": key,
        "cwd": cwd,
        "args": args,
        "state": "queued",
        "created": time.time(),
        "started": None,
        "finished": None,
        "exit": None,
        "pid": None,
        "note": "",
    }
    write_job(job)
    return job


def runner_active():
    with open(HOME / "runner.lock", "a") as handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            return True
        return False


def ensure_runner():
    if runner_active():
        return
    pid = os.fork()
    if pid:
        os.waitpid(pid, 0)
        return
    try:
        os.setsid()
        if os.fork() == 0:
            devnull = os.open(os.devnull, os.O_RDWR)
            log = os.open(
                str(HOME / "runner.log"), os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o644
            )
            os.dup2(devnull, 0)
            os.dup2(log, 1)
            os.dup2(log, 2)
            os.closerange(3, 1024)
            os.execv(sys.executable, [sys.executable, str(SCRIPT), "__runner"])
    finally:
        os._exit(0)


def signal_group(pid, sig):
    try:
        os.killpg(pid, sig)
    except (ProcessLookupError, PermissionError):
        pass


def stop_reason(job_id):
    job = read_job(job_id) or {}
    if job.get("cancel"):
        return "cancelled", "stopped by cargo queue cancel"
    if time.time() - last_heartbeat(job_id) > ABANDON_AFTER_SECONDS:
        return (
            "abandoned",
            f"stopped after {ABANDON_AFTER_SECONDS}s without a waiting client",
        )
    return None


def execute(job):
    directory = job_dir(job["id"])
    env = json.loads((directory / "env.json").read_text())
    env[INSIDE] = "1"
    cargo = shutil.which("cargo", path=env.get("PATH", ""))
    try:
        if cargo is None:
            raise OSError("cargo is not on the requesting client's PATH")
        with (
            open(directory / "stdout", "ab") as out,
            open(directory / "stderr", "ab") as err,
        ):
            proc = subprocess.Popen(
                [cargo, *job["args"]],
                cwd=job["cwd"],
                env=env,
                stdin=subprocess.DEVNULL,
                stdout=out,
                stderr=err,
                start_new_session=True,
            )
    except OSError as error:
        with queue_lock():
            finish(
                read_job(job["id"]),
                "failed",
                None,
                f"could not start cargo in {job['cwd']}: {error}",
            )
        return
    with queue_lock():
        current = read_job(job["id"])
        current["pid"] = proc.pid
        write_job(current)

    stop, stop_at = None, 0.0
    while True:
        try:
            proc.wait(timeout=2)
            break
        except subprocess.TimeoutExpired:
            pass
        if stop is None:
            stop = stop_reason(job["id"])
            if stop:
                signal_group(proc.pid, signal.SIGINT)
                stop_at = time.time()
        elif time.time() - stop_at > STOP_GRACE_SECONDS:
            signal_group(proc.pid, signal.SIGKILL)

    state, note = stop or ("done", "")
    with queue_lock():
        finish(read_job(job["id"]), state, proc.returncode, note)


def next_job():
    now = time.time()
    for job in all_jobs():
        if job["state"] != "queued":
            continue
        if now - last_heartbeat(job["id"]) > ABANDON_AFTER_SECONDS:
            finish(
                job,
                "abandoned",
                None,
                f"dropped after {ABANDON_AFTER_SECONDS}s without a waiting client",
            )
            continue
        job.update(state="running", started=now)
        write_job(job)
        return job
    return None


def collect_garbage():
    finished = [job for job in all_jobs() if job["state"] in TERMINAL]
    cutoff = time.time() - KEEP_FINISHED_SECONDS
    overflow = max(0, len(finished) - KEEP_FINISHED_JOBS)
    for index, job in enumerate(finished):
        if index < overflow or (job["finished"] or 0) < cutoff:
            shutil.rmtree(job_dir(job["id"]), ignore_errors=True)


def run_queue():
    HOME.mkdir(parents=True, exist_ok=True)
    lock = open(HOME / "runner.lock", "a")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        return 0
    with queue_lock():
        for job in all_jobs():
            if job["state"] == "running":
                finish(
                    job,
                    "failed",
                    None,
                    "the queue runner stopped while this job was running",
                )
    while True:
        with queue_lock():
            collect_garbage()
            job = next_job()
            if job is None:
                lock.close()
                return 0
        execute(job)


def pump(job_id, offsets):
    for name, fd in (("stdout", 1), ("stderr", 2)):
        try:
            with open(job_dir(job_id) / name, "rb") as handle:
                handle.seek(offsets[name])
                data = handle.read()
        except OSError:
            continue
        offsets[name] += len(data)
        write_fd(fd, data)


def queue_position(job):
    active = [other for other in all_jobs() if other["state"] in ("queued", "running")]
    ahead = [other for other in active if other["id"] < job["id"]]
    running = next((other for other in active if other["state"] == "running"), None)
    return len(ahead), running


def exit_code(job):
    code = job["exit"]
    if job["state"] == "done" and isinstance(code, int):
        return 128 - code if code < 0 else code
    say(f"job {job['id']} {job['state']}: {job['note']}")
    return code if isinstance(code, int) and code > 0 else EX_TEMPFAIL


def follow(job_id):
    def interrupted(signum, _frame):
        say(
            f"stopped waiting. Job {job_id} keeps its place for {ABANDON_AFTER_SECONDS}s; "
            f"reattach with: {queue_command_hint('wait', str(job_id))}"
        )
        os._exit(128 + signum)

    for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(sig, interrupted)

    offsets = {"stdout": 0, "stderr": 0}
    last_ahead, waited = None, False
    while True:
        heartbeat(job_id)
        job = read_job(job_id)
        if job is None:
            say(f"job {job_id} no longer exists")
            return EX_TEMPFAIL
        if job["state"] == "queued":
            ahead, running = queue_position(job)
            if ahead != last_ahead and ahead:
                current = (
                    f"; running #{running['id']} {describe(running)} for {ago(time.time() - running['started'])}"
                    if running
                    else ""
                )
                say(f"job {job_id}: {ahead} ahead{current}")
            last_ahead, waited = ahead, waited or ahead > 0
        elif waited and job["started"]:
            say(
                f"job {job_id} started after {ago(job['started'] - job['created'])} in the queue"
            )
            waited = False
        pump(job_id, offsets)
        if job["state"] in TERMINAL:
            (job_dir(job_id) / "waiters" / str(os.getpid())).unlink(missing_ok=True)
            return exit_code(job)
        if not runner_active():
            ensure_runner()
        time.sleep(POLL_SECONDS)


def submit(args):
    cwd = os.getcwd()
    env = {k: v for k, v in os.environ.items() if not k.startswith("CARGO_QUEUE")}
    env["PATH"] = path_without_shim(env.get("PATH", ""))
    key = job_key(cwd, args, env)
    try:
        with queue_lock():
            jobs = all_jobs()
            job, joined = find_joinable(jobs, key, cwd, args)
            if job is None:
                job = create_job(jobs, key, cwd, args, env)
            heartbeat(job["id"])
    except OSError as error:
        say(f"queue unavailable ({error}); running cargo directly")
        return exec_cargo(args)

    hint = f"reattach after an interrupt with: {queue_command_hint('wait', str(job['id']))}"
    if joined == "queued":
        say(f"joined identical queued job {job['id']}; {hint}")
    elif joined == "running":
        say(
            f"joined identical job {job['id']}, running for {ago(time.time() - job['started'])} "
            f"with no Rust source change since it started; {hint}"
        )
    else:
        say(f"job {job['id']}: {describe(job)}; {hint}")
    return follow(job["id"])


def print_status():
    jobs = all_jobs()
    now = time.time()
    active = [job for job in jobs if job["state"] in ("running", "queued")]
    recent = [job for job in jobs if job["state"] in TERMINAL][-5:]
    home = str(Path.home())
    lines = []
    for job in active + recent:
        if job["state"] == "running":
            when = f"for {ago(now - job['started'])}"
        elif job["state"] == "queued":
            when = f"for {ago(now - job['created'])}"
        else:
            code = "" if job["exit"] is None else f", exit {job['exit']}"
            when = f"{ago(now - (job['finished'] or now))} ago{code}"
        cwd = job["cwd"].replace(home, "~", 1)
        waiting = live_waiters(job["id"]) if job["state"] not in TERMINAL else 0
        lines.append(
            f"#{job['id']:<5} {job['state']:<9} {when:<22} {waiting} waiting  {cwd}  cargo {describe(job)}"
        )
    print("\n".join(lines) if lines else "queue is empty")
    if not active:
        return 0
    print(f"runner: {'active' if runner_active() else 'not running'}")
    return 0


def cancel(job_id):
    with queue_lock():
        job = read_job(job_id)
        if job is None:
            print(f"job {job_id} does not exist")
            return 1
        if job["state"] == "queued":
            finish(job, "cancelled", None, "cancelled before it started")
        elif job["state"] == "running":
            job["cancel"] = True
            write_job(job)
        else:
            print(f"job {job_id} already {job['state']}")
            return 0
    print(f"job {job_id} cancelled")
    return 0


def queue_command(args):
    action = args[0] if args else "status"
    if action == "status":
        return print_status()
    if (
        action in ("wait", "cancel")
        and len(args) == 2
        and args[1].lstrip("#").isdigit()
    ):
        job_id = int(args[1].lstrip("#"))
        if action == "cancel":
            return cancel(job_id)
        if read_job(job_id) is None:
            print(f"job {job_id} does not exist")
            return 1
        return follow(job_id)
    print(__doc__.strip())
    return 2


def main(args):
    if args[:1] == ["__runner"]:
        return run_queue()
    if args[:1] == ["queue"]:
        return queue_command(args[1:])
    if os.environ.get(INSIDE) or not should_queue(args):
        return exec_cargo(args)
    return submit(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
