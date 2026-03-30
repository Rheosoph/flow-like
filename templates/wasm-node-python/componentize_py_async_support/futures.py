import componentize_py_runtime
import componentize_py_async_support
import weakref

from typing import TypeVar, Generic, cast, Self, Any, Callable
from types import TracebackType
from componentize_py_async_support import _ReturnCode

T = TypeVar('T')

class FutureReader(Generic[T]):
    def __init__(self, type_: int, handle: int):
        self.type_ = type_
        self.handle: int | None = handle
        self.finalizer = weakref.finalize(self, componentize_py_runtime.future_drop_readable, type_, handle)

    async def read(self) -> T:
        self.finalizer.detach()
        handle = self.handle
        self.handle = None
        if handle is not None:
            result = await componentize_py_async_support.await_result(
                componentize_py_runtime.future_read(self.type_, handle)
            )
            componentize_py_runtime.future_drop_readable(self.type_, handle)
            return cast(T, result)
        else:
            raise AssertionError

    def __enter__(self) -> Self:
        return self

    def __exit__(self,
                 exc_type: type[BaseException] | None,
                 exc_value: BaseException | None,
                 traceback: TracebackType | None) -> bool | None:
        if self.handle is not None:
            self.finalizer.detach()
            handle = self.handle
            self.handle = None
            componentize_py_runtime.future_drop_readable(self.type_, handle)

        return None

async def write(type_: int, handle: int, value: Any) -> None:
    await componentize_py_async_support.await_result(
        componentize_py_runtime.future_write(type_, handle, value)
    )
    componentize_py_runtime.future_drop_writable(type_, handle)

def write_default(type_: int, handle: int, default: Callable[[], Any]) -> None:
    componentize_py_async_support.spawn(write(type_, handle, default()))

class FutureWriter(Generic[T]):
    def __init__(self, type_: int, handle: int, default: Callable[[], T]):
        self.type_ = type_
        self.handle: int | None = handle
        self.default = default
        self.finalizer = weakref.finalize(self, write_default, type_, handle, default)

    async def write(self, value: T) -> bool:
        self.finalizer.detach()
        handle = self.handle
        self.handle = None
        if handle is not None:
            code, _ = await componentize_py_async_support.await_result(
                componentize_py_runtime.future_write(self.type_, handle, value)
            )
            componentize_py_runtime.future_drop_writable(self.type_, handle)
            match code:
                case _ReturnCode.COMPLETED:
                    return True
                case _ReturnCode.DROPPED:
                    return False
                case _ReturnCode.CANCELLED:
                    # todo
                    raise NotImplementedError
                case _:
                    raise AssertionError
        else:
            raise AssertionError

    def __enter__(self) -> Self:
        return self

    def __exit__(self,
                 exc_type: type[BaseException] | None,
                 exc_value: BaseException | None,
                 traceback: TracebackType | None) -> bool | None:
        if self.handle is not None:
            componentize_py_async_support.spawn(self.write(self.default()))

        return None
