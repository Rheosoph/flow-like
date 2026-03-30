import componentize_py_runtime
import componentize_py_async_support
import weakref

from typing import TypeVar, Generic, Self, cast
from types import TracebackType
from componentize_py_async_support import _ReturnCode

class ByteStreamReader:
    def __init__(self, type_: int, handle: int):
        self.writer_dropped = False
        self.type_ = type_
        self.handle: int | None = handle
        self.finalizer = weakref.finalize(self, componentize_py_runtime.stream_drop_readable, type_, handle)

    async def read(self, max_count: int) -> bytes:
        if self.writer_dropped:
            return bytes()

        code, values = await self._read(max_count)

        if code == _ReturnCode.DROPPED:
            self.writer_dropped = True

        return values

    async def _read(self, max_count: int) -> tuple[int, bytes]:
        if self.handle is not None:
            return cast(tuple[int, bytes], await componentize_py_async_support.await_result(
                componentize_py_runtime.stream_read(self.type_, self.handle, max_count)
            ))
        else:
            raise AssertionError

    def __enter__(self) -> Self:
        return self

    def __exit__(self,
                 exc_type: type[BaseException] | None,
                 exc_value: BaseException | None,
                 traceback: TracebackType | None) -> bool | None:
        self.finalizer.detach()
        handle = self.handle
        self.handle = None
        if handle is not None:
            componentize_py_runtime.stream_drop_readable(self.type_, handle)
        return None

class ByteStreamWriter:
    def __init__(self, type_: int, handle: int):
        self.reader_dropped = False
        self.type_ = type_
        self.handle: int | None = handle
        self.finalizer = weakref.finalize(self, componentize_py_runtime.stream_drop_writable, type_, handle)

    async def write(self, source: bytes) -> int:
        if self.reader_dropped:
            return 0

        code, count = await self._write(source)

        if code == _ReturnCode.DROPPED:
            self.reader_dropped = True

        return count

    async def _write(self, source: bytes) -> tuple[int, int]:
        if self.handle is not None:
            return await componentize_py_async_support.await_result(
                componentize_py_runtime.stream_write(self.type_, self.handle, source)
            )
        else:
            raise AssertionError

    async def write_all(self, source: bytes) -> int:
        total = 0

        while len(source) > 0 and not self.reader_dropped:
            count = await self.write(source)
            source = source[count:]
            total += count

        return total

    def __enter__(self) -> Self:
        return self

    def __exit__(self,
                 exc_type: type[BaseException] | None,
                 exc_value: BaseException | None,
                 traceback: TracebackType | None) -> bool | None:
        self.finalizer.detach()
        handle = self.handle
        self.handle = None
        if handle is not None:
            componentize_py_runtime.stream_drop_writable(self.type_, handle)
        return None

T = TypeVar('T')

class StreamReader(Generic[T]):
    def __init__(self, type_: int, handle: int):
        self.writer_dropped = False
        self.type_ = type_
        self.handle: int | None = handle
        self.finalizer = weakref.finalize(self, componentize_py_runtime.stream_drop_readable, type_, handle)

    async def read(self, max_count: int) -> list[T]:
        if self.writer_dropped:
            return []

        code, values = await self._read(max_count)

        if code == _ReturnCode.DROPPED:
            self.writer_dropped = True

        return values

    async def _read(self, max_count: int) -> tuple[int, list[T]]:
        if self.handle is not None:
            return cast(tuple[int, list[T]], await componentize_py_async_support.await_result(
                componentize_py_runtime.stream_read(self.type_, self.handle, max_count)
            ))
        else:
            raise AssertionError

    def __enter__(self) -> Self:
        return self

    def __exit__(self,
                 exc_type: type[BaseException] | None,
                 exc_value: BaseException | None,
                 traceback: TracebackType | None) -> bool | None:
        self.finalizer.detach()
        handle = self.handle
        self.handle = None
        if handle is not None:
            componentize_py_runtime.stream_drop_readable(self.type_, handle)
        return None

class StreamWriter(Generic[T]):
    def __init__(self, type_: int, handle: int):
        self.reader_dropped = False
        self.type_ = type_
        self.handle: int | None = handle
        self.finalizer = weakref.finalize(self, componentize_py_runtime.stream_drop_writable, type_, handle)

    async def write(self, source: list[T]) -> int:
        if self.reader_dropped:
            return 0

        code, count = await self._write(source)

        if code == _ReturnCode.DROPPED:
            self.reader_dropped = True

        return count

    async def _write(self, source: list[T]) -> tuple[int, int]:
        if self.handle is not None:
            return await componentize_py_async_support.await_result(
                componentize_py_runtime.stream_write(self.type_, self.handle, source)
            )
        else:
            raise AssertionError

    async def write_all(self, source: list[T]) -> int:
        total = 0

        while len(source) > 0 and not self.reader_dropped:
            count = await self.write(source)
            source = source[count:]
            total += count

        return total

    def __enter__(self) -> Self:
        return self

    def __exit__(self,
                 exc_type: type[BaseException] | None,
                 exc_value: BaseException | None,
                 traceback: TracebackType | None) -> bool | None:
        self.finalizer.detach()
        handle = self.handle
        self.handle = None
        if handle is not None:
            componentize_py_runtime.stream_drop_writable(self.type_, handle)
        return None
