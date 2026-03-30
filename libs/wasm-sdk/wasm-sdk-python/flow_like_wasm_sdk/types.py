from __future__ import annotations

import json
import re
from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from enum import IntEnum
from typing import TYPE_CHECKING, Any, get_args, get_origin

if TYPE_CHECKING:
    from flow_like_wasm_sdk.context import Context

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


def _to_snake_case(name: str) -> str:
    """Convert CamelCase to snake_case: ``MyCustomNode`` → ``my_custom_node``."""
    s = re.sub(r"([A-Z]+)([A-Z][a-z])", r"\1_\2", name)
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", s)
    return s.lower()


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


# Maps common shorthand permission names to the canonical Rust serde names.
_PERMISSION_ALIASES: dict[str, str] = {
    "http": "network:http",
    "network_http": "network:http",
    "websocket": "network:websocket",
    "network_websocket": "network:websocket",
    "tcp": "network:tcp",
    "network_tcp": "network:tcp",
    "udp": "network:udp",
    "network_udp": "network:udp",
    "dns": "network:dns",
    "network_dns": "network:dns",
    "storage_read": "storage:read",
    "storage_write": "storage:write",
    "auth_oauth": "oauth",
    "image_processing": "image:processing",
}


def _normalize_permission(perm: str) -> str:
    """Map a shorthand permission to its canonical Rust serde name."""
    return _PERMISSION_ALIASES.get(perm, perm)


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
        self.permissions.append(_normalize_permission(permission))
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


# ── Declarative pin descriptors ─────────────────────────────────────────


class Exec:
    """Sentinel type for exec pin annotations."""


_TYPE_MAP: dict[type, str] = {
    float: PinType.F64,
    int: PinType.I64,
    str: PinType.STRING,
    bool: PinType.BOOL,
    bytes: PinType.BYTES,
}

_STR_TYPE_MAP: dict[str, str] = {
    "float": PinType.F64,
    "int": PinType.I64,
    "str": PinType.STRING,
    "bool": PinType.BOOL,
    "bytes": PinType.BYTES,
}


@dataclass
class Input:
    """Declares an input data pin via class annotation.

    Usage::

        class MyNode(WasmNode):
            value: float = Input(default=0.0, title="Value", ge=0.0, le=1.0)
    """

    default: Any = None
    default_factory: Any = None
    title: str | None = None
    description: str | None = None
    gt: float | None = None
    ge: float | None = None
    lt: float | None = None
    le: float | None = None
    options: list[str] | None = None
    sensitive: bool = False
    pin_name: str | None = None
    value_type: str | None = None
    schema: str | None = None


@dataclass
class Output:
    """Declares an output data pin via class annotation.

    Usage::

        class MyNode(WasmNode):
            result: float = Output(title="Result")
    """

    title: str | None = None
    description: str | None = None
    pin_name: str | None = None
    value_type: str | None = None
    schema: str | None = None


@dataclass
class ExecInput:
    """Declares an exec input pin via class annotation.

    Usage::

        class MyNode(WasmNode):
            trigger: Exec = ExecInput()
    """

    pin_name: str | None = None
    description: str | None = None


@dataclass
class ExecOutput:
    """Declares an exec output pin via class annotation.

    Usage::

        class MyNode(WasmNode):
            on_true: Exec = ExecOutput()
            on_false: Exec = ExecOutput()
    """

    pin_name: str | None = None
    description: str | None = None


def _is_base_model(cls: Any) -> bool:
    """Check if *cls* is a pydantic BaseModel subclass (without importing pydantic at module level)."""
    try:
        from pydantic import BaseModel
        return isinstance(cls, type) and issubclass(cls, BaseModel)
    except ImportError:
        return False


def _is_interop_type(cls: Any) -> bool:
    """Check if *cls* is one of the SDK interop domain types (FlowPath, FlowImage, Bit, etc.)."""
    try:
        from flow_like_wasm_sdk.interop import FlowPath, FlowImage, Bit, CachedEmbeddingModel, NodeDBConnection
        return isinstance(cls, type) and issubclass(cls, (FlowPath, FlowImage, Bit, CachedEmbeddingModel, NodeDBConnection))
    except ImportError:
        return False


def _scalar_data_type(annotation: Any) -> tuple[str, type | None]:
    """Map a scalar Python type to ``(PinType string, model_class_or_None)``."""
    if isinstance(annotation, type):
        if _is_base_model(annotation):
            return PinType.STRUCT, annotation
        if _is_interop_type(annotation):
            return PinType.STRUCT, annotation
        return _TYPE_MAP.get(annotation, PinType.GENERIC), None
    if isinstance(annotation, str):
        return _STR_TYPE_MAP.get(annotation, PinType.GENERIC), None
    return PinType.GENERIC, None


def _resolve_type_info(annotation: Any) -> tuple[str, str, type | None]:
    """Resolve a type annotation to ``(data_type, value_type, model_class)``.

    Handles scalars, BaseModel subclasses, and generic collections:

    - ``list[X]``    \u2192 ``(resolve(X), Array, model_if_basemodel)``
    - ``dict[K, V]`` \u2192 ``(resolve(V), HashMap, model_if_basemodel)``
    - ``set[X]``     \u2192 ``(resolve(X), HashSet, model_if_basemodel)``
    """
    origin = get_origin(annotation)
    if origin is list:
        args = get_args(annotation)
        inner = args[0] if args else None
        dt, model = _scalar_data_type(inner)
        return dt, ValueType.ARRAY, model
    if origin is dict:
        args = get_args(annotation)
        inner = args[1] if len(args) > 1 else None
        dt, model = _scalar_data_type(inner)
        return dt, ValueType.HASH_MAP, model
    if origin is set:
        args = get_args(annotation)
        inner = args[0] if args else None
        dt, model = _scalar_data_type(inner)
        return dt, ValueType.HASH_SET, model
    dt, model = _scalar_data_type(annotation)
    return dt, ValueType.NORMAL, model


def _collect_pins(cls: type) -> None:
    """Inspect class annotations and populate pin metadata on *cls*."""
    input_pins: dict[str, tuple[str, str, Any]] = {}   # name \u2192 (data_type, value_type, default)
    output_pins: dict[str, tuple[str, str]] = {}       # name \u2192 (data_type, value_type)
    exec_inputs: list[str] = []
    exec_outputs: list[str] = []
    pin_descriptors: dict[str, Input | Output | ExecInput | ExecOutput] = {}
    pin_models: dict[str, type] = {}

    annotations: dict[str, Any] = {}
    for klass in reversed(cls.__mro__):
        if klass is object:
            continue
        annotations.update(getattr(klass, "__annotations__", {}))

    for field_name, annotation in annotations.items():
        value = getattr(cls, field_name, None)
        if isinstance(value, Input):
            data_type, value_type, model = _resolve_type_info(annotation)
            default = value.default
            if default is None and value.default_factory is not None:
                default = value.default_factory()
            input_pins[field_name] = (data_type, value_type, default)
            pin_descriptors[field_name] = value
            if model is not None:
                pin_models[field_name] = model
        elif isinstance(value, Output):
            data_type, value_type, model = _resolve_type_info(annotation)
            output_pins[field_name] = (data_type, value_type)
            pin_descriptors[field_name] = value
            if model is not None:
                pin_models[field_name] = model
        elif isinstance(value, ExecInput):
            exec_inputs.append(value.pin_name or field_name)
            pin_descriptors[field_name] = value
        elif isinstance(value, ExecOutput):
            exec_outputs.append(value.pin_name or field_name)
            pin_descriptors[field_name] = value

    cls.__input_pins__ = input_pins
    cls.__output_pins__ = output_pins
    cls.__exec_inputs__ = exec_inputs
    cls.__exec_outputs__ = exec_outputs
    cls.__pin_descriptors__ = pin_descriptors
    cls.__pin_models__ = pin_models


_NODE_META_ATTRS = frozenset({"name", "title", "category", "icon", "permissions",
                              "long_running", "docs", "scores"})


def _build_node_definition(cls: type) -> NodeDefinition:
    """Build a *NodeDefinition* from class-level attributes.

    Every field is optional — sensible defaults are derived from the class name.
    Common fields (``name``, ``title``, ``category``, ``icon``) can be
    passed as subclass kwargs; rarer fields (``permissions``, ``scores``,
    ``long_running``, ``docs``) as plain class attributes.
    """
    derived_name = _to_snake_case(cls.__name__)
    name = getattr(cls, "__node_name__", None) or derived_name
    title = getattr(cls, "__node_title__", None) or _humanize(name)
    description = (cls.__doc__ or getattr(cls, "__node_description__", "") or f"Node: {name}").strip()
    category = getattr(cls, "__node_category__", "Custom")
    icon = getattr(cls, "__node_icon__", None)
    permissions: list[str] = getattr(cls, "permissions", [])
    long_running = getattr(cls, "long_running", None)
    docs = getattr(cls, "docs", None)
    scores: NodeScores | None = getattr(cls, "scores", None)

    nd = NodeDefinition(name, title, description, category, icon=icon)
    if scores is not None:
        nd.set_scores(scores)
    if long_running is not None:
        nd.set_long_running(long_running)
    if docs is not None:
        nd.docs = docs
    for perm in permissions:
        nd.add_permission(perm)

    exec_ins: list[str] = getattr(cls, "__exec_inputs__", [])
    if not exec_ins:
        nd.add_pin(PinDefinition.input_exec("exec"))
    else:
        for pin_name in exec_ins:
            nd.add_pin(PinDefinition.input_exec(pin_name))

    input_pins: dict[str, tuple[str, str, Any]] = getattr(cls, "__input_pins__", {})
    descs: dict[str, Any] = getattr(cls, "__pin_descriptors__", {})
    pin_models: dict[str, type] = getattr(cls, "__pin_models__", {})
    for field_name, (data_type, value_type, default) in input_pins.items():
        desc: Input | None = descs.get(field_name)  # type: ignore[assignment]
        pin_name = (desc.pin_name if desc else None) or field_name
        friendly = (desc.title if desc else None) or _humanize(field_name)
        desc_text = (desc.description if desc else None) or f"Input: {pin_name}"
        pin = PinDefinition.input_pin(
            pin_name, data_type,
            description=desc_text,
            default=default,
            friendly_name=friendly,
        )
        # Auto-set value_type from annotation
        if value_type != ValueType.NORMAL:
            pin.with_value_type(value_type)
        if desc is not None:
            if desc.options:
                pin.with_valid_values(desc.options)
            if desc.sensitive:
                pin.with_sensitive(True)
            if desc.value_type:
                pin.with_value_type(desc.value_type)
            if desc.schema:
                pin.with_schema(desc.schema)
            lo = desc.ge if desc.ge is not None else desc.gt
            hi = desc.le if desc.le is not None else desc.lt
            if lo is not None and hi is not None:
                pin.with_range(lo, hi)
        # Auto-inject JSON schema for BaseModel or interop type annotations
        if field_name in pin_models and pin.schema is None:
            model = pin_models[field_name]
            if hasattr(model, "model_json_schema"):
                pin.with_schema(json.dumps(model.model_json_schema()))
            elif hasattr(model, "json_schema"):
                pin.with_schema(json.dumps(model.json_schema()))
            pin.with_enforce_schema(True)
        nd.add_pin(pin)

    exec_outs: list[str] = getattr(cls, "__exec_outputs__", [])
    if not exec_outs:
        nd.add_pin(PinDefinition.output_exec("exec_out"))
    else:
        for pin_name in exec_outs:
            nd.add_pin(PinDefinition.output_exec(pin_name))

    output_pins: dict[str, tuple[str, str]] = getattr(cls, "__output_pins__", {})
    for field_name, (data_type, value_type) in output_pins.items():
        desc_out: Output | None = descs.get(field_name)  # type: ignore[assignment]
        pin_name = (desc_out.pin_name if desc_out else None) or field_name
        friendly = (desc_out.title if desc_out else None) or _humanize(field_name)
        desc_text = (desc_out.description if desc_out else None) or f"Output: {pin_name}"
        pin = PinDefinition.output_pin(
            pin_name, data_type,
            description=desc_text,
            friendly_name=friendly,
        )
        # Auto-set value_type from annotation
        if value_type != ValueType.NORMAL:
            pin.with_value_type(value_type)
        if desc_out is not None:
            if desc_out.value_type:
                pin.with_value_type(desc_out.value_type)
            if desc_out.schema:
                pin.with_schema(desc_out.schema)
        # Auto-inject JSON schema for BaseModel or interop type annotations
        if field_name in pin_models and pin.schema is None:
            model = pin_models[field_name]
            if hasattr(model, "model_json_schema"):
                pin.with_schema(json.dumps(model.model_json_schema()))
            elif hasattr(model, "json_schema"):
                pin.with_schema(json.dumps(model.json_schema()))
        nd.add_pin(pin)

    return nd


def _validate_collection(val: Any, value_type: str, model_cls: type) -> Any:
    """Validate/deserialize elements inside a collection."""
    if value_type == ValueType.ARRAY and isinstance(val, list):
        return [_deserialize_struct(item, model_cls) for item in val]
    if value_type == ValueType.HASH_MAP and isinstance(val, dict):
        return {k: _deserialize_struct(v, model_cls) for k, v in val.items()}
    if value_type == ValueType.HASH_SET and isinstance(val, (list, set)):
        return [_deserialize_struct(item, model_cls) for item in val]
    return val


def _serialize_value(val: Any) -> Any:
    """Serialize a value that may be a BaseModel or interop type to JSON-safe form."""
    if hasattr(val, "model_dump"):
        return val.model_dump()
    if hasattr(val, "to_dict"):
        return val.to_dict()
    return val


def _serialize_collection(val: Any) -> Any:
    """Serialize a collection that may contain BaseModel/interop instances to JSON-safe types."""
    if isinstance(val, list):
        return [_serialize_value(item) for item in val]
    if isinstance(val, dict):
        return {k: _serialize_value(v) for k, v in val.items()}
    if isinstance(val, (set, frozenset)):
        return [_serialize_value(item) for item in val]
    return _serialize_value(val)


def _deserialize_struct(val: Any, model_cls: type | None) -> Any:
    """Deserialize a dict into a BaseModel or interop type instance."""
    if model_cls is None or not isinstance(val, dict):
        return val
    if hasattr(model_cls, "model_validate"):
        return model_cls.model_validate(val)
    if hasattr(model_cls, "from_dict"):
        return model_cls.from_dict(val)
    return val


class TypedContext:
    """Context wrapper providing typed attribute access to declared pins.

    Handles scalar types, BaseModel structs, and collections (``list``, ``dict``, ``set``).

    For BaseModel-typed pins (scalar or inside collections):
    - On read: raw dicts are validated and returned as model instances
    - On write: model instances are serialized to dicts via ``model_dump()``
    """

    def __init__(
        self,
        raw_ctx: Context,
        input_pins: dict[str, tuple[str, str, Any]],
        output_pins: dict[str, tuple[str, str]],
        pin_models: dict[str, type] | None = None,
    ) -> None:
        object.__setattr__(self, "_raw", raw_ctx)
        object.__setattr__(self, "_inputs", input_pins)
        object.__setattr__(self, "_outputs", output_pins)
        object.__setattr__(self, "_models", pin_models or {})

    def __getattr__(self, name: str) -> Any:
        inputs = object.__getattribute__(self, "_inputs")
        if name in inputs:
            raw = object.__getattribute__(self, "_raw")
            data_type, value_type, default = inputs[name]
            # Collections — use get_input, optionally validate model elements
            if value_type != ValueType.NORMAL:
                val = raw.get_input(name)
                if val is None:
                    return default
                models = object.__getattribute__(self, "_models")
                model_cls = models.get(name)
                if model_cls is not None:
                    return _validate_collection(val, value_type, model_cls)
                return val
            # Scalar types
            if data_type == PinType.F64:
                return raw.get_f64(name, default)
            if data_type == PinType.I64:
                return raw.get_i64(name, default)
            if data_type == PinType.STRING:
                return raw.get_string(name, default)
            if data_type == PinType.BOOL:
                return raw.get_bool(name, default)
            if data_type == PinType.STRUCT:
                models = object.__getattribute__(self, "_models")
                val = raw.get_input(name)
                model_cls = models.get(name)
                if val is None:
                    return default
                return _deserialize_struct(val, model_cls)
            return raw.get_input(name)
        return getattr(object.__getattribute__(self, "_raw"), name)

    def __setattr__(self, name: str, value: Any) -> None:
        outputs = object.__getattribute__(self, "_outputs")
        if name in outputs:
            raw = object.__getattribute__(self, "_raw")
            _data_type, value_type = outputs[name]
            if value_type != ValueType.NORMAL:
                value = _serialize_collection(value)
            else:
                value = _serialize_value(value)
            raw.set_output(name, value)
        else:
            object.__setattr__(self, name, value)


# ── Node registry & abstract base ──────────────────────────────────────

_NODE_REGISTRY: list[type[WasmNode]] = []


class WasmNode(ABC):
    """Abstract base for Flow-Like WASM nodes.

    **Declarative style** (recommended) — annotate pins with
    :class:`Input` / :class:`Output`; ``get_node()`` and ``TypedContext``
    wrapping are auto-generated.

    Minimal example — everything auto-derived from the class::

        class Add(WasmNode):
            \"\"\"Adds two numbers\"\"\"
            a: float = Input(default=0.0)
            b: float = Input(default=0.0)
            result: float = Output()

            def run(self, ctx) -> ExecutionResult:
                ctx.result = ctx.a + ctx.b
                return ctx.success()

    Common metadata via **subclass kwargs** (all optional)::

        class Add(WasmNode, name="math_add", category="Math"):
            ...

    Rare metadata as **class-level attributes**::

        class HttpFetch(WasmNode, category="Network", icon="/icons/http.svg"):
            permissions = ["network:http"]
            long_running = True
            scores = NodeScores(security=3, privacy=2)
            ...

    **Manual style** (full control) — override both ``get_node()`` and
    ``run()`` directly.

    Subclasses are auto-registered on definition.
    """

    def __init_subclass__(
        cls,
        *,
        name: str | None = None,
        title: str | None = None,
        category: str | None = None,
        icon: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init_subclass__(**kwargs)

        # Store kwargs as dunder attrs so they survive inheritance
        if name is not None:
            cls.__node_name__ = name
        if title is not None:
            cls.__node_title__ = title
        if category is not None:
            cls.__node_category__ = category
        if icon is not None:
            cls.__node_icon__ = icon

        _collect_pins(cls)

        input_pins = getattr(cls, "__input_pins__", {})
        output_pins = getattr(cls, "__output_pins__", {})
        exec_inputs = getattr(cls, "__exec_inputs__", [])
        exec_outputs = getattr(cls, "__exec_outputs__", [])
        has_declarative_pins = bool(input_pins or output_pins or exec_inputs or exec_outputs)

        # Auto-implement get_node() when not manually overridden and there
        # are declarative pins (Meta is optional — everything is derived).
        if has_declarative_pins and "get_node" not in cls.__dict__:
            cls.get_node = lambda self: _build_node_definition(type(self))

        # Wrap run() to inject TypedContext when declarative pins exist
        if has_declarative_pins and "run" in cls.__dict__:
            original = cls.__dict__["run"]

            def _make_wrapper(orig: Any) -> Any:
                def _wrapped(self: Any, ctx: Any) -> ExecutionResult:
                    if not isinstance(ctx, TypedContext):
                        ctx = TypedContext(
                            ctx,
                            type(self).__input_pins__,
                            type(self).__output_pins__,
                            getattr(type(self), "__pin_models__", {}),
                        )
                    return orig(self, ctx)
                return _wrapped

            cls.run = _make_wrapper(original)

        if getattr(cls, "__abstractmethods__", None):
            return
        _NODE_REGISTRY.append(cls)

    def get_node(self) -> NodeDefinition:
        raise NotImplementedError(
            f"{type(self).__name__} must either declare pins with "
            "Input/Output annotations or override get_node()"
        )

    @abstractmethod
    def run(self, ctx: Any) -> ExecutionResult:
        ...


def get_registered_nodes() -> list[WasmNode]:
    return [cls() for cls in _NODE_REGISTRY]


def get_all_definitions() -> list[NodeDefinition]:
    return [n.get_node() for n in get_registered_nodes()]


def run_node(node_name: str, ctx: Context) -> ExecutionResult:
    for cls in _NODE_REGISTRY:
        instance = cls()
        if instance.get_node().name == node_name:
            return instance.run(ctx)
    return ExecutionResult.fail(f"Unknown node: {node_name}")
