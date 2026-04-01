import asyncio
import componentize_py_runtime
import subprocess

from os import PathLike
from socket import AddressFamily, AddressInfo, SocketKind, socket
from concurrent.futures import Executor
from componentize_py_types import Result, Ok, Err
from dataclasses import dataclass
from typing import Any, Optional, TypeVar, TypeVarTuple, Callable, IO, Literal, cast
from contextvars import ContextVar, Context
from collections.abc import Coroutine, Awaitable, Sequence
from asyncio.protocols import BaseProtocol
from asyncio.transports import Transport, WriteTransport, DatagramTransport, SubprocessTransport, ReadTransport
from asyncio.base_events import Server

try:
    from ssl import SSLContext
except:
    pass

@dataclass
class _FutureState:
    waitable_set: int | None
    futures: dict[int, Any]
    handles: list[asyncio.Handle]
    pending_count: int

class _ReturnCode:
    COMPLETED = 0
    DROPPED = 1
    CANCELLED = 2

class _CallbackCode:
    EXIT = 0
    YIELD = 1
    WAIT = 2
    POLL = 3

class _Event:
    NONE = 0
    SUBTASK = 1
    STREAM_READ = 2
    STREAM_WRITE = 3
    FUTURE_READ = 4
    FUTURE_WRITE = 5
    CANCELLED = 6

class _Status:
    STARTING = 0
    STARTED = 1
    RETURNED = 2
    START_CANCELLED = 3
    RETURN_CANCELLED = 4

_T = TypeVar("_T")
_Ts = TypeVarTuple("_Ts")
_ProtocolT = TypeVar("_ProtocolT", bound=BaseProtocol)

async def _noop() -> None:
    pass

class _Loop(asyncio.AbstractEventLoop):
    def __init__(self) -> None:
        self.running: bool = False
        self.exception: Optional[Any] = None

    def poll(self, future_state: _FutureState) -> None:
        while True:
            handles = future_state.handles
            future_state.handles = []
            for handle in handles:
                if not handle._cancelled:
                    handle._run()

            if self.exception is not None:
                raise self.exception

            if len(handles) == 0 and len(future_state.handles) == 0:
                return

    def get_debug(self) -> bool:
        return False

    def is_running(self) -> bool:
        return self.running

    def is_closed(self) -> bool:
        return not self.running

    def stop(self) -> None:
        self.running = False

    def close(self) -> None:
        self.running = False

    def shutdown_asyncgens(self) -> Coroutine[Any, Any, None]:
        return _noop()

    def call_exception_handler(self, context: dict[str, Any]) -> None:
        self.exception = context.get('exception', None)

    def call_soon(self,
                  callback: Callable[[*_Ts], object],
                  *args: *_Ts,
                  context: Context | None = None) -> asyncio.Handle:
        global _future_state

        if context is not None:
            future_state = context[_future_state]
            handle = asyncio.Handle(callback, args, self, context)
            future_state.handles.append(handle)
            return handle
        else:
            raise AssertionError

    def create_task(self, coroutine: Coroutine[Any, Any, _T], name: str | None = None, context: Context | None = None) -> asyncio.Task[_T]:
        return asyncio.Task(coroutine, loop=self, context=context)

    def create_future(self) -> asyncio.Future[Any]:
        return asyncio.Future(loop=self)

    # The remaining methods should be irrelevant for our purposes and thus unimplemented

    def run_until_complete(self, future: Awaitable[_T]) -> _T:
        raise NotImplementedError

    def run_forever(self) -> None:
        raise NotImplementedError

    async def shutdown_default_executor(self) -> None:
        raise NotImplementedError

    def call_later(self,
                   delay: float,
                   callback: Callable[[*_Ts], object],
                   *args: *_Ts,
                   context: Context | None = None) -> asyncio.TimerHandle:
        raise NotImplementedError

    def call_at(self,
                when: float,
                callback: Callable[[*_Ts], object],
                *args: *_Ts,
                context: Context | None = None) -> asyncio.TimerHandle:
        raise NotImplementedError

    def time(self) -> float:
        raise NotImplementedError

    def call_soon_threadsafe(self,
                             callback: Callable[[*_Ts], object],
                             *args: *_Ts,
                             context: Context | None = None) -> asyncio.Handle:
        raise NotImplementedError

    def run_in_executor(self,
                        executor: Executor | None,
                        func: Callable[[*_Ts], _T],
                        *args: *_Ts) -> asyncio.Future[_T]:
        raise NotImplementedError

    def set_default_executor(self, executor: Executor) -> None:
        raise NotImplementedError

    async def getaddrinfo(
            self,
            host: bytes | str | None,
            port: bytes | str | int | None,
            *,
            family: int = 0,
            type: int = 0,
            proto: int = 0,
            flags: int = 0,
    ) -> list[tuple[AddressFamily, SocketKind, int, str, tuple[str, int] | tuple[str, int, int, int]]]:
        raise NotImplementedError

    async def getnameinfo(self,
                          sockaddr: tuple[str, int] | tuple[str, int, int, int],
                          flags: int = 0) -> tuple[str, str]:
        raise NotImplementedError

    async def create_connection(
            self,
            protocol_factory: Callable[[], _ProtocolT],
            host: str | None = ...,
            port: int | None = ...,
            *,
            ssl: bool | SSLContext | None = None,
            family: int = 0,
            proto: int = 0,
            flags: int = 0,
            sock: socket | None = None,
            local_addr: tuple[str, int] | None = None,
            server_hostname: str | None = None,
            ssl_handshake_timeout: float | None = None,
            ssl_shutdown_timeout: float | None = None,
            happy_eyeballs_delay: float | None = None,
            interleave: int | None = None,
    ) -> tuple[Transport, _ProtocolT]:
        raise NotImplementedError

    async def create_server(
            self,
            protocol_factory: Callable[[], BaseProtocol],
            host: str | Sequence[str] | None = None,
            port: int | None = None,
            *,
            family: int = AddressFamily.AF_UNSPEC,
            flags: int = AddressInfo.AI_PASSIVE,
            sock: socket | None = ...,
            backlog: int = 100,
            ssl: bool | SSLContext | None = None,
            reuse_address: bool | None = None,
            reuse_port: bool | None = None,
            keep_alive: bool | None = None,
            ssl_handshake_timeout: float | None = None,
            ssl_shutdown_timeout: float | None = None,
            start_serving: bool = True,
    ) -> Server:
        raise NotImplementedError

    async def sendfile(self,
                       transport: WriteTransport,
                       file: IO[bytes],
                       offset: int = 0,
                       count: int | None = None,
                       *,
                       fallback: bool = True
    ) -> int:
        raise NotImplementedError

    async def start_tls(
            self,
            transport: WriteTransport,
            protocol: BaseProtocol,
            sslcontext: SSLContext,
            *,
            server_side: bool = False,
            server_hostname: str | None = None,
            ssl_handshake_timeout: float | None = None,
            ssl_shutdown_timeout: float | None = None,
        ) -> Transport | None:
        raise NotImplementedError

    async def create_unix_connection(
            self,
            protocol_factory: Callable[[], _ProtocolT],
            path: str | None = None,
            *,
            ssl: bool | SSLContext | None = None,
            sock: socket | None = None,
            server_hostname: str | None = None,
            ssl_handshake_timeout: float | None = None,
            ssl_shutdown_timeout: float | None = None,
        ) -> tuple[Transport, _ProtocolT]:
        raise NotImplementedError

    async def create_unix_server(
            self,
            protocol_factory: Callable[[], BaseProtocol],
            path: str | PathLike[str] | None = None,
            *,
            sock: socket | None = None,
            backlog: int = 100,
            ssl: bool | SSLContext | None = None,
            ssl_handshake_timeout: float | None = None,
            ssl_shutdown_timeout: float | None = None,
            start_serving: bool = True,
    ) -> Server:
        raise NotImplementedError

    async def connect_accepted_socket(
            self,
            protocol_factory: Callable[[], _ProtocolT],
            sock: socket,
            *,
            ssl: bool | SSLContext | None = None,
            ssl_handshake_timeout: float | None = None,
            ssl_shutdown_timeout: float | None = None,
        ) -> tuple[Transport, _ProtocolT]:
        raise NotImplementedError

    async def create_datagram_endpoint(
            self,
            protocol_factory: Callable[[], _ProtocolT],
            local_addr: tuple[str, int] | str | None = None,
            remote_addr: tuple[str, int] | str | None = None,
            *,
            family: int = 0,
            proto: int = 0,
            flags: int = 0,
            reuse_address: bool | None = None,
            reuse_port: bool | None = None,
            allow_broadcast: bool | None = None,
            sock: socket | None = None,
    ) -> tuple[DatagramTransport, _ProtocolT]:
        raise NotImplementedError

    async def connect_read_pipe(self,
                                protocol_factory: Callable[[], _ProtocolT],
                                pipe: Any) ->  tuple[ReadTransport, _ProtocolT]:
        raise NotImplementedError

    async def connect_write_pipe(self,
                                 protocol_factory: Callable[[], _ProtocolT],
                                 pipe: Any) -> tuple[WriteTransport, _ProtocolT]:
        raise NotImplementedError

    async def subprocess_shell(
        self,
        protocol_factory: Callable[[], _ProtocolT],
        cmd: bytes | str,
        *,
        stdin: int | IO[Any] | None = -1,
        stdout: int | IO[Any] | None = -1,
        stderr: int | IO[Any] | None = -1,
        universal_newlines: Literal[False] = False,
        shell: Literal[True] = True,
        bufsize: Literal[0] = 0,
        encoding: None = None,
        errors: None = None,
        text: Literal[False] | None = None,
        **kwargs: Any,
    ) -> tuple[SubprocessTransport, _ProtocolT]:
        raise NotImplementedError

    async def subprocess_exec(
        self,
        protocol_factory: Callable[[], _ProtocolT],
        program: Any,
        *args: Any,
        stdin: int | IO[Any] | None = -1,
        stdout: int | IO[Any] | None = -1,
        stderr: int | IO[Any] | None = -1,
        universal_newlines: Literal[False] = False,
        shell: Literal[False] = False,
        bufsize: Literal[0] = 0,
        encoding: None = None,
        errors: None = None,
        **kwargs: Any,
    ) -> tuple[SubprocessTransport, _ProtocolT]:
        raise NotImplementedError

    def add_reader(self, fd: Any, callback: Callable[[*_Ts], Any], *args: *_Ts) -> None:
        raise NotImplementedError

    def remove_reader(self, fd: Any) -> bool:
        raise NotImplementedError

    def add_writer(self, fd: Any, callback: Callable[[*_Ts], Any], *args: *_Ts) -> None:
        raise NotImplementedError

    def remove_writer(self, fd: Any) -> bool:
        raise NotImplementedError

    async def sock_recv(self, sock: socket, nbytes: int) -> bytes:
        raise NotImplementedError

    async def sock_recv_into(self, sock: socket, buf: Any) -> int:
        raise NotImplementedError

    async def sock_recvfrom(self, sock: socket, bufsize: int) -> tuple[bytes, Any]:
        raise NotImplementedError

    async def sock_recvfrom_into(self, sock: socket, buf: Any, nbytes: int = 0) -> tuple[int, Any]:
        raise NotImplementedError

    async def sock_sendall(self, sock: socket, data: Any) -> None:
        raise NotImplementedError

    async def sock_sendto(self, sock: socket, data: Any, address: Any) -> int:
        raise NotImplementedError

    async def sock_connect(self, sock: socket, address: Any) -> None:
        raise NotImplementedError

    async def sock_accept(self, sock: socket) -> tuple[socket, Any]:
        raise NotImplementedError

    async def sock_sendfile(
            self,
            sock: socket,
            file: IO[bytes],
            offset: int = 0,
            count: int | None = None,
            *,
            fallback: bool | None = None
    ) -> int:
        raise NotImplementedError

    def add_signal_handler(self, sig: int, callback: Callable[[*_Ts], object], *args: *_Ts) -> None:
        raise NotImplementedError

    def remove_signal_handler(self, sig: int) -> bool:
        raise NotImplementedError

    def set_task_factory(self, factory: Any | None) -> None:
        raise NotImplementedError

    def get_task_factory(self) -> None:
        raise NotImplementedError

    def get_exception_handler(self) -> None:
        raise NotImplementedError

    def set_exception_handler(self, handler: Any | None) -> None:
        raise NotImplementedError

    def default_exception_handler(self, context: dict[str, Any]) -> None:
        raise NotImplementedError

    def set_debug(self, enabled: bool) -> None:
        raise NotImplementedError

_future_state: ContextVar[_FutureState] = ContextVar("_future_state")
_loop = _Loop()
asyncio.set_event_loop(_loop)
_loop.running = True
asyncio.events._set_running_loop(_loop)

def _set_future_state(future_state: _FutureState) -> None:
    global _future_state

    _future_state.set(future_state)

async def _return_result(export_index: int, borrows: int, coroutine: Any) -> None:
    global _future_state

    try:
        try:
            result: Result[Any, Any] = Ok(await coroutine)
        except Err as e:
            result = e

        componentize_py_runtime.call_task_return(export_index, borrows, result)
    except Exception as e:
        _loop.exception = e

    assert _future_state.get().pending_count > 0
    _future_state.get().pending_count -= 1

def first_poll(export_index: int, borrows: int, coroutine: Any) -> int:
    context = Context()
    future_state = _FutureState(None, {}, [], 1)
    context.run(_set_future_state, future_state)
    asyncio.create_task(_return_result(export_index, borrows, coroutine), context=context)
    return _poll(future_state)

def _poll(future_state: _FutureState) -> int:
    global _loop

    _loop.poll(future_state)

    if future_state.pending_count == 0:
        if future_state.waitable_set is not None:
            componentize_py_runtime.waitable_set_drop(future_state.waitable_set)

        return _CallbackCode.EXIT
    else:
        waitable_set = future_state.waitable_set
        assert waitable_set is not None
        componentize_py_runtime.context_set(future_state)
        return _CallbackCode.WAIT | (waitable_set << 4)

def callback(event0: int, event1: int, event2: int) -> int:
    future_state = componentize_py_runtime.context_get()
    componentize_py_runtime.context_set(None)

    match event0:
        case _Event.NONE:
            pass
        case _Event.SUBTASK:
            match event2:
                case _Status.STARTING:
                    raise AssertionError
                case _Status.STARTED:
                    pass
                case _Status.RETURNED:
                    componentize_py_runtime.waitable_join(event1, 0)
                    componentize_py_runtime.subtask_drop(event1)
                    future_state.futures.pop(event1).set_result(event2)
                case _:
                    # todo
                    raise NotImplementedError
        case _Event.STREAM_READ | _Event.STREAM_WRITE | _Event.FUTURE_READ | _Event.FUTURE_WRITE:
            componentize_py_runtime.waitable_join(event1, 0)
            future_state.futures.pop(event1).set_result(event2)
        case _:
            # todo
            raise NotImplementedError

    return _poll(future_state)

async def await_result[T](result: Result[T, tuple[int, int]]) -> T:
    global _loop
    global _future_state

    if isinstance(result, Ok):
        return result.value
    else:
        future_state = _future_state.get()
        waitable, promise = result.value
        future = _loop.create_future()
        future_state.futures[waitable] = future

        if future_state.waitable_set is None:
            future_state.waitable_set = componentize_py_runtime.waitable_set_new()
        componentize_py_runtime.waitable_join(waitable, future_state.waitable_set)

        return cast(T, componentize_py_runtime.promise_get_result(await future, promise))

async def _wrap_spawned(coroutine: Any) -> None:
    global _future_state

    try:
        await coroutine
    except Exception as e:
        _loop.exception = e

    assert _future_state.get().pending_count > 0
    _future_state.get().pending_count -= 1

def spawn(coroutine: Any) -> None:
    global _future_state

    _future_state.get().pending_count += 1

    asyncio.create_task(_wrap_spawned(coroutine))
