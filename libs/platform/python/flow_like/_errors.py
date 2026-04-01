"""Exception hierarchy for the Flow Like Python SDK."""

from __future__ import annotations


class FlowLikeError(Exception):
    """Base exception for all Flow Like SDK errors."""

    pass


class AuthenticationError(FlowLikeError):
    """Raised when authentication credentials are invalid or malformed."""

    pass


class ConfigurationError(FlowLikeError):
    """Raised when SDK configuration is missing or contradictory."""

    pass


class APIError(FlowLikeError):
    """Raised when the API returns a non-success HTTP status code."""

    def __init__(self, status_code: int, message: str, response_body: str | None = None):
        self.status_code = status_code
        self.response_body = response_body
        super().__init__(f"HTTP {status_code}: {message}")


class NotFoundError(APIError):
    """Raised when a requested resource does not exist (HTTP 404)."""

    pass


class RateLimitError(APIError):
    """Raised when the API rate limit has been exceeded (HTTP 429)."""

    pass


class ServerError(APIError):
    """Raised when the API returns a server-side error (HTTP 5xx)."""

    pass


__all__ = [
    "FlowLikeError",
    "AuthenticationError",
    "ConfigurationError",
    "APIError",
    "NotFoundError",
    "RateLimitError",
    "ServerError",
]
