from typing import TypeVar, Generic, Union
from dataclasses import dataclass

S = TypeVar('S')
@dataclass
class Some(Generic[S]):
    value: S

T = TypeVar('T')
@dataclass
class Ok(Generic[T]):
    value: T

E = TypeVar('E')
@dataclass(frozen=True)
class Err(Generic[E], Exception):
    value: E

Result = Union[Ok[T], Err[E]]
