"""Data types for the Flow-Like Python SDK.

Contains dataclass definitions for API responses, cloud credentials,
database access, model inference results, and board execution.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass
class AsyncInvokeResult:
    """Result of an asynchronous board invocation, containing the run ID and poll token."""

    run_id: str
    poll_token: str
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class RunStatus:
    """Status of a board run, including completion state and optional result or error."""

    run_id: str
    status: str
    result: Any = None
    error: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class PollResult:
    """Result of polling for run events, with a done flag indicating completion."""

    events: list[dict[str, Any]] = field(default_factory=list)
    done: bool = False
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class SSEEvent:
    """A parsed Server-Sent Event with optional event type, data, id, and retry interval."""

    event: str | None = None
    data: str = ""
    id: str | None = None
    retry: int | None = None


@dataclass
class FileInfo:
    """Metadata for a file stored in a remote bucket."""

    key: str
    size: int | None = None
    last_modified: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class PresignResult:
    """A presigned URL for direct file upload or download, with optional headers."""

    url: str
    headers: dict[str, str] = field(default_factory=dict)
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class BucketConfig:
    """Configuration for a cloud storage bucket endpoint."""

    endpoint: str | None = None
    express: bool = False


@dataclass
class AwsSharedCredentials:
    """Temporary AWS credentials and bucket references for accessing Flow-Like storage."""

    meta_bucket: str = ""
    content_bucket: str = ""
    logs_bucket: str = ""
    region: str = ""
    access_key_id: str | None = None
    secret_access_key: str | None = None
    session_token: str | None = None
    meta_config: BucketConfig | None = None
    content_config: BucketConfig | None = None
    logs_config: BucketConfig | None = None
    expiration: str | None = None
    content_path_prefix: str | None = None
    user_content_path_prefix: str | None = None


@dataclass
class AzureSharedCredentials:
    """Temporary Azure credentials and container references for accessing Flow-Like storage."""

    meta_container: str = ""
    content_container: str = ""
    logs_container: str = ""
    account_name: str = ""
    meta_sas_token: str | None = None
    content_sas_token: str | None = None
    user_content_sas_token: str | None = None
    logs_sas_token: str | None = None
    account_key: str | None = None
    expiration: str | None = None
    content_path_prefix: str | None = None
    user_content_path_prefix: str | None = None


@dataclass
class GcpSharedCredentials:
    """Temporary GCP credentials and bucket references for accessing Flow-Like storage."""

    service_account_key: str = ""
    meta_bucket: str = ""
    content_bucket: str = ""
    logs_bucket: str = ""
    allowed_prefixes: list[str] = field(default_factory=list)
    write_access: bool = False
    access_token: str | None = None
    expiration: str | None = None
    content_path_prefix: str | None = None
    user_content_path_prefix: str | None = None


@dataclass
class PresignDbAccessResponse:
    """Response containing credentials and connection details for presigned database access."""

    shared_credentials: dict[str, Any] = field(default_factory=dict)
    db_path: str = ""
    table_name: str = ""
    access_mode: str = "read"
    expiration: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class LanceConnectionInfo:
    """Connection parameters for a LanceDB database, including URI and storage options."""

    uri: str = ""
    storage_options: dict[str, str] = field(default_factory=dict)


@dataclass
class TableSchema:
    """Schema definition for a database table, including its name and column descriptors."""

    name: str
    columns: list[dict[str, Any]] = field(default_factory=list)
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class QueryResult:
    """Result of a database query, containing the returned rows."""

    rows: list[dict[str, Any]] = field(default_factory=list)
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class CountResult:
    """Result of a count query against a database table."""

    count: int
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class App:
    """A registered Flow-Like application."""

    id: str
    name: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class HealthStatus:
    """Health-check response from the Flow-Like API."""

    healthy: bool
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class UsageInfo:
    """Token usage statistics from an inference request."""

    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class ChatMessage:
    """A single message in a chat conversation, with a role and text content."""

    role: str
    content: str


@dataclass
class ChatChoice:
    """A single choice from a chat completion response."""

    index: int = 0
    message: ChatMessage = field(default_factory=lambda: ChatMessage(role="assistant", content=""))
    finish_reason: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class ChatCompletionResult:
    """Result of a chat completion request, including choices and token usage."""

    choices: list[ChatChoice] = field(default_factory=list)
    usage: UsageInfo = field(default_factory=UsageInfo)
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class EmbeddingResult:
    """Result of an embedding request, containing vectors and token usage."""

    embeddings: list[list[float]] = field(default_factory=list)
    usage: UsageInfo = field(default_factory=UsageInfo)
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class ModelInfo:
    """Metadata for an available AI model, including capabilities and supported languages."""

    bit_id: str
    name: str
    description: str = ""
    provider_name: str | None = None
    model_id: str | None = None
    context_length: int | None = None
    vector_length: int | None = None
    languages: list[str] = field(default_factory=list)
    tags: list[str] = field(default_factory=list)


@dataclass
class Board:
    """A Flow-Like board (workflow definition)."""

    id: str
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class UpsertBoardResponse:
    """Response from creating or updating a board."""

    id: str
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class PrerunBoardResponse:
    """Pre-run analysis of a board, listing required variables, OAuth needs, and execution mode."""

    runtime_variables: list[dict[str, Any]] = field(default_factory=list)
    oauth_requirements: list[dict[str, Any]] = field(default_factory=list)
    requires_local_execution: bool = False
    execution_mode: str = ""
    can_execute_locally: bool = False
    raw: dict[str, Any] = field(default_factory=dict)


__all__ = [
    "AsyncInvokeResult",
    "RunStatus",
    "PollResult",
    "SSEEvent",
    "FileInfo",
    "PresignResult",
    "BucketConfig",
    "AwsSharedCredentials",
    "AzureSharedCredentials",
    "GcpSharedCredentials",
    "PresignDbAccessResponse",
    "LanceConnectionInfo",
    "TableSchema",
    "QueryResult",
    "CountResult",
    "App",
    "HealthStatus",
    "UsageInfo",
    "ChatChoice",
    "ChatMessage",
    "ChatCompletionResult",
    "EmbeddingResult",
    "ModelInfo",
    "Board",
    "UpsertBoardResponse",
    "PrerunBoardResponse",
]
