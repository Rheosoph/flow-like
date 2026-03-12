"""
Flow-Like WASM SDK for Python

Provides types, utilities, and base classes for building WASM nodes
that can be executed by the Flow-Like runtime.

Usage:
    from sdk import NodeDefinition, PinDefinition, Context, ExecutionResult

    def get_definition() -> NodeDefinition:
        node = NodeDefinition("my_node", "My Node", "Does something", "Custom/Python")
        node.add_pin(PinDefinition.input_exec("exec"))
        node.add_pin(PinDefinition.input_pin("text", "String", default=""))
        node.add_pin(PinDefinition.output_exec("exec_out"))
        node.add_pin(PinDefinition.output_pin("result", "String"))
        return node

    def run(ctx: Context) -> ExecutionResult:
        text = ctx.get_string("text") or ""
        ctx.set_output("result", text.upper())
        return ctx.success()
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import IntEnum
from typing import Any


ABI_VERSION = 1


class LogLevel(IntEnum):
    DEBUG = 0
    INFO = 1
    WARN = 2
    ERROR = 3
    FATAL = 4


class PinType:
    EXEC = "Exec"
    STRING = "String"
    I64 = "I64"
    F64 = "F64"
    BOOL = "Bool"
    GENERIC = "Generic"
    BYTES = "Bytes"
    DATE = "Date"
    PATH_BUF = "PathBuf"
    STRUCT = "Struct"

    _ALL = {EXEC, STRING, I64, F64, BOOL, GENERIC, BYTES, DATE, PATH_BUF, STRUCT}

    @classmethod
    def validate(cls, data_type: str) -> str:
        if data_type not in cls._ALL:
            raise ValueError(f"Invalid pin data type: {data_type}. Must be one of {cls._ALL}")
        return data_type


class ValueType:
    """How the data is contained (scalar vs collection)."""
    NORMAL = "Normal"
    ARRAY = "Array"
    HASH_MAP = "HashMap"
    HASH_SET = "HashSet"

    _ALL = {NORMAL, ARRAY, HASH_MAP, HASH_SET}

    @classmethod
    def validate(cls, value_type: str) -> str:
        if value_type not in cls._ALL:
            raise ValueError(f"Invalid value type: {value_type}. Must be one of {cls._ALL}")
        return value_type


# Alias: PinType was confusingly named (in core, PinType means Input/Output).
# Prefer DataType for clarity.
DataType = PinType


def _humanize(name: str) -> str:
    return " ".join(w.capitalize() for w in name.split("_") if w)


@dataclass
class NodeScores:
    privacy: int = 0
    security: int = 0
    performance: int = 0
    governance: int = 0
    reliability: int = 0
    cost: int = 0

    def to_dict(self) -> dict[str, int]:
        return {
            "privacy": self.privacy,
            "security": self.security,
            "performance": self.performance,
            "governance": self.governance,
            "reliability": self.reliability,
            "cost": self.cost,
        }


@dataclass
class PinDefinition:
    name: str
    friendly_name: str
    description: str
    pin_type: str  # "Input" or "Output"
    data_type: str
    default_value: Any = None
    value_type: str | None = None
    schema: str | None = None
    valid_values: list[str] | None = None
    range: tuple[float, float] | None = None
    step: float | None = None
    sensitive: bool | None = None
    enforce_schema: bool | None = None
    enforce_generic_value_type: bool | None = None

    @classmethod
    def input_pin(
        cls,
        name: str,
        data_type: str,
        *,
        description: str = "",
        default: Any = None,
        friendly_name: str | None = None,
    ) -> PinDefinition:
        PinType.validate(data_type)
        return cls(
            name=name,
            friendly_name=friendly_name or _humanize(name),
            description=description or f"Input: {name}",
            pin_type="Input",
            data_type=data_type,
            default_value=default,
        )

    @classmethod
    def output_pin(
        cls,
        name: str,
        data_type: str,
        *,
        description: str = "",
        friendly_name: str | None = None,
    ) -> PinDefinition:
        PinType.validate(data_type)
        return cls(
            name=name,
            friendly_name=friendly_name or _humanize(name),
            description=description or f"Output: {name}",
            pin_type="Output",
            data_type=data_type,
        )

    @classmethod
    def input_exec(cls, name: str = "exec", *, description: str = "") -> PinDefinition:
        return cls(
            name=name,
            friendly_name=_humanize(name),
            description=description or f"Input: {name}",
            pin_type="Input",
            data_type=PinType.EXEC,
        )

    @classmethod
    def output_exec(cls, name: str = "exec_out", *, description: str = "") -> PinDefinition:
        return cls(
            name=name,
            friendly_name=_humanize(name),
            description=description or f"Output: {name}",
            pin_type="Output",
            data_type=PinType.EXEC,
        )

    def with_default(self, value: Any) -> PinDefinition:
        self.default_value = value
        return self

    def with_value_type(self, value_type: str) -> PinDefinition:
        ValueType.validate(value_type)
        self.value_type = value_type
        return self

    def with_schema(self, schema: str) -> PinDefinition:
        self.schema = schema
        return self

    def with_valid_values(self, values: list[str]) -> PinDefinition:
        self.valid_values = values
        return self

    def with_range(self, min_val: float, max_val: float) -> PinDefinition:
        self.range = (min_val, max_val)
        return self

    def with_step(self, step: float) -> PinDefinition:
        self.step = step
        return self

    def with_sensitive(self, sensitive: bool = True) -> PinDefinition:
        self.sensitive = sensitive
        return self

    def with_enforce_schema(self, enforce: bool = True) -> PinDefinition:
        self.enforce_schema = enforce
        return self

    def with_enforce_generic_value_type(self, enforce: bool = True) -> PinDefinition:
        self.enforce_generic_value_type = enforce
        return self

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "name": self.name,
            "friendly_name": self.friendly_name,
            "description": self.description,
            "pin_type": self.pin_type,
            "data_type": self.data_type,
        }
        if self.default_value is not None:
            d["default_value"] = self.default_value
        if self.value_type is not None:
            d["value_type"] = self.value_type
        if self.schema is not None:
            d["schema"] = self.schema
        if self.valid_values is not None:
            d["valid_values"] = self.valid_values
        if self.range is not None:
            d["range"] = list(self.range)
        if self.step is not None:
            d["step"] = self.step
        if self.sensitive is not None:
            d["sensitive"] = self.sensitive
        if self.enforce_schema is not None:
            d["enforce_schema"] = self.enforce_schema
        if self.enforce_generic_value_type is not None:
            d["enforce_generic_value_type"] = self.enforce_generic_value_type
        return d


@dataclass
class NodeDefinition:
    name: str
    friendly_name: str
    description: str
    category: str
    icon: str | None = None
    pins: list[PinDefinition] = field(default_factory=list)
    scores: NodeScores | None = None
    long_running: bool | None = None
    docs: str | None = None
    permissions: list[str] = field(default_factory=list)
    abi_version: int = ABI_VERSION

    def add_pin(self, pin: PinDefinition) -> NodeDefinition:
        self.pins.append(pin)
        return self

    def set_scores(self, scores: NodeScores) -> NodeDefinition:
        self.scores = scores
        return self

    def set_long_running(self, long_running: bool) -> NodeDefinition:
        self.long_running = long_running
        return self

    def add_permission(self, permission: str) -> NodeDefinition:
        self.permissions.append(permission)
        return self

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "name": self.name,
            "friendly_name": self.friendly_name,
            "description": self.description,
            "category": self.category,
            "pins": [p.to_dict() for p in self.pins],
            "abi_version": self.abi_version,
        }
        if self.icon is not None:
            d["icon"] = self.icon
        if self.scores is not None:
            d["scores"] = self.scores.to_dict()
        if self.long_running is not None:
            d["long_running"] = self.long_running
        if self.docs is not None:
            d["docs"] = self.docs
        if self.permissions:
            d["permissions"] = self.permissions
        return d

    def to_json(self) -> str:
        return json.dumps(self.to_dict())


@dataclass
class ExecutionInput:
    inputs: dict[str, Any]
    node_id: str = ""
    run_id: str = ""
    app_id: str = ""
    board_id: str = ""
    user_id: str = ""
    stream_state: bool = False
    log_level: int = LogLevel.INFO
    node_name: str = ""

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> ExecutionInput:
        return cls(
            inputs=data.get("inputs", {}),
            node_id=data.get("node_id", ""),
            run_id=data.get("run_id", ""),
            app_id=data.get("app_id", ""),
            board_id=data.get("board_id", ""),
            user_id=data.get("user_id", ""),
            stream_state=data.get("stream_state", False),
            log_level=data.get("log_level", LogLevel.INFO),
            node_name=data.get("node_name", ""),
        )

    @classmethod
    def from_json(cls, json_str: str) -> ExecutionInput:
        return cls.from_dict(json.loads(json_str))


@dataclass
class ExecutionResult:
    outputs: dict[str, Any] = field(default_factory=dict)
    error: str | None = None
    activate_exec: list[str] = field(default_factory=list)
    pending: bool | None = None

    @classmethod
    def ok(cls) -> ExecutionResult:
        return cls()

    @classmethod
    def fail(cls, message: str) -> ExecutionResult:
        return cls(error=message)

    def set_output(self, name: str, value: Any) -> ExecutionResult:
        self.outputs[name] = value
        return self

    def exec(self, pin_name: str) -> ExecutionResult:
        self.activate_exec.append(pin_name)
        return self

    def set_pending(self, pending: bool) -> ExecutionResult:
        self.pending = pending
        return self

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "outputs": self.outputs,
            "activate_exec": self.activate_exec,
        }
        if self.error is not None:
            d["error"] = self.error
        if self.pending is not None:
            d["pending"] = self.pending
        return d

    def to_json(self) -> str:
        return json.dumps(self.to_dict())


class HostBridge:
    """Interface for host function calls. Replaced at runtime by actual WASM host imports."""

    def log(self, level: int, message: str) -> None:
        pass

    def stream(self, event_type: str, data: str) -> None:
        pass

    def get_variable(self, name: str) -> Any:
        return None

    def set_variable(self, name: str, value: Any) -> bool:
        return False

    def time_now(self) -> int:
        return 0

    def random(self) -> int:
        return 0


class MockHostBridge(HostBridge):
    """Host bridge for local testing with captured logs and streams."""

    def __init__(self) -> None:
        self.logs: list[tuple[int, str]] = []
        self.streams: list[tuple[str, str]] = []
        self.variables: dict[str, Any] = {}
        self._time: int = 0
        self._random_value: int = 42

    def log(self, level: int, message: str) -> None:
        self.logs.append((level, message))

    def stream(self, event_type: str, data: str) -> None:
        self.streams.append((event_type, data))

    def get_variable(self, name: str) -> Any:
        return self.variables.get(name)

    def set_variable(self, name: str, value: Any) -> bool:
        self.variables[name] = value
        return True

    def time_now(self) -> int:
        return self._time

    def random(self) -> int:
        return self._random_value


# Global host bridge (swapped for testing or WASM runtime)
_host: HostBridge = HostBridge()


def set_host(host: HostBridge) -> None:
    global _host
    _host = host


def get_host() -> HostBridge:
    return _host


class Context:
    """Execution context providing typed input access, output setting, logging, and streaming."""

    def __init__(self, execution_input: ExecutionInput, host: HostBridge | None = None) -> None:
        self._input = execution_input
        self._result = ExecutionResult.ok()
        self._host = host or _host

    @classmethod
    def from_dict(cls, data: dict[str, Any], host: HostBridge | None = None) -> Context:
        return cls(ExecutionInput.from_dict(data), host)

    @classmethod
    def from_json(cls, json_str: str, host: HostBridge | None = None) -> Context:
        return cls(ExecutionInput.from_json(json_str), host)

    # -- Metadata --

    @property
    def node_id(self) -> str:
        return self._input.node_id

    @property
    def node_name(self) -> str:
        return self._input.node_name

    @property
    def run_id(self) -> str:
        return self._input.run_id

    @property
    def app_id(self) -> str:
        return self._input.app_id

    @property
    def board_id(self) -> str:
        return self._input.board_id

    @property
    def user_id(self) -> str:
        return self._input.user_id

    @property
    def stream_enabled(self) -> bool:
        return self._input.stream_state

    @property
    def log_level(self) -> int:
        return self._input.log_level

    # -- Input getters --

    def get_input(self, name: str) -> Any:
        return self._input.inputs.get(name)

    def get_string(self, name: str, default: str | None = None) -> str | None:
        val = self.get_input(name)
        if val is None:
            return default
        return str(val)

    def get_i64(self, name: str, default: int | None = None) -> int | None:
        val = self.get_input(name)
        if val is None:
            return default
        return int(val)

    def get_f64(self, name: str, default: float | None = None) -> float | None:
        val = self.get_input(name)
        if val is None:
            return default
        return float(val)

    def get_bool(self, name: str, default: bool | None = None) -> bool | None:
        val = self.get_input(name)
        if val is None:
            return default
        return bool(val)

    def require_input(self, name: str) -> Any:
        val = self.get_input(name)
        if val is None:
            raise ValueError(f"Required input '{name}' not provided")
        return val

    # -- Output setters --

    def set_output(self, name: str, value: Any) -> None:
        self._result.set_output(name, value)

    def activate_exec(self, pin_name: str) -> None:
        self._result.exec(pin_name)

    def set_pending(self, pending: bool) -> None:
        self._result.set_pending(pending)

    # -- Logging (level-gated) --

    def debug(self, message: str) -> None:
        if self._input.log_level <= LogLevel.DEBUG:
            self._host.log(LogLevel.DEBUG, message)

    def info(self, message: str) -> None:
        if self._input.log_level <= LogLevel.INFO:
            self._host.log(LogLevel.INFO, message)

    def warn(self, message: str) -> None:
        if self._input.log_level <= LogLevel.WARN:
            self._host.log(LogLevel.WARN, message)

    def error(self, message: str) -> None:
        if self._input.log_level <= LogLevel.ERROR:
            self._host.log(LogLevel.ERROR, message)

    # -- Streaming --

    def stream_text(self, text: str) -> None:
        if self._input.stream_state:
            self._host.stream("text", text)

    def stream_json(self, data: Any) -> None:
        if self._input.stream_state:
            self._host.stream("json", json.dumps(data))

    def stream_progress(self, progress: float, message: str) -> None:
        if self._input.stream_state:
            payload = json.dumps({"progress": progress, "message": message})
            self._host.stream("progress", payload)

    # -- Variables --

    def get_variable(self, name: str) -> Any:
        return self._host.get_variable(name)

    def set_variable(self, name: str, value: Any) -> bool:
        return self._host.set_variable(name, value)

    # -- Finalize --

    def success(self) -> ExecutionResult:
        self._result.exec("exec_out")
        return self._result

    def fail(self, error: str) -> ExecutionResult:
        self._result.error = error
        return self._result

    def finish(self) -> ExecutionResult:
        return self._result


def node(
    name: str,
    friendly_name: str,
    description: str,
    category: str,
    **kwargs: Any,
) -> NodeDefinition:
    """Shorthand to create a NodeDefinition."""
    nd = NodeDefinition(name, friendly_name, description, category, **kwargs)
    return nd
