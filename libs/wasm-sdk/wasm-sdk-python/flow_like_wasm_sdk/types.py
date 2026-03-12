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
    pin_type: str
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

    def with_schema_model(self, model: type) -> PinDefinition:
        """Derive a JSON Schema from a Pydantic ``BaseModel`` subclass and
        attach it to this pin in one step.

        Requires ``pydantic`` (install the ``schema`` extra::

            pip install flow-like-wasm-sdk[schema]

        Example::

            from pydantic import BaseModel

            class Config(BaseModel):
                threshold: float
                label: str

            pin = PinDefinition.input("config", "Config", "Node config", PinType.STRUCT) \\
                .with_schema_model(Config)
        """
        try:
            schema_dict = model.model_json_schema()  # type: ignore[attr-defined]
        except AttributeError as exc:
            raise TypeError(
                f"{model!r} must be a pydantic BaseModel subclass. "
                "Install pydantic: pip install 'flow-like-wasm-sdk[schema]'"
            ) from exc
        self.schema = json.dumps(schema_dict)
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
        return d

    def to_json(self) -> str:
        return json.dumps(self.to_dict())


@dataclass
class PackageNodes:
    nodes: list[NodeDefinition] = field(default_factory=list)

    def add_node(self, node: NodeDefinition) -> PackageNodes:
        self.nodes.append(node)
        return self

    def to_dict(self) -> list[dict[str, Any]]:
        return [n.to_dict() for n in self.nodes]

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
