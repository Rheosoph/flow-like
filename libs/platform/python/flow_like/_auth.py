"""Authentication and base-URL resolution helpers for the Flow Like SDK."""

from __future__ import annotations

import os

from ._errors import AuthenticationError, ConfigurationError

PAT_PREFIX = "pat_"
API_KEY_PREFIX = "flk_"


def resolve_auth(
    pat: str | None = None,
    api_key: str | None = None,
) -> dict[str, str]:
    """Build an auth header dict from a PAT or API key.

    Raises:
        ConfigurationError: If both or neither credential is provided.
        AuthenticationError: If the credential has an invalid prefix.
    """
    pat = pat or os.environ.get("FLOW_LIKE_PAT")
    api_key = api_key or os.environ.get("FLOW_LIKE_API_KEY")

    if pat and api_key:
        raise ConfigurationError("Provide either a PAT or an API key, not both.")

    if pat:
        if not pat.startswith(PAT_PREFIX):
            raise AuthenticationError(f"PAT must start with '{PAT_PREFIX}'")
        return {"Authorization": pat}

    if api_key:
        if not api_key.startswith(API_KEY_PREFIX):
            raise AuthenticationError(f"API key must start with '{API_KEY_PREFIX}'")
        return {"X-API-Key": api_key}

    raise ConfigurationError(
        "No credentials provided. Set FLOW_LIKE_PAT or FLOW_LIKE_API_KEY "
        "environment variable, or pass pat= / api_key= to the client."
    )


def resolve_base_url(base_url: str | None = None) -> str:
    """Return a normalised base URL, falling back to the environment variable."""
    url = base_url or os.environ.get("FLOW_LIKE_BASE_URL")
    if not url:
        raise ConfigurationError(
            "No base URL provided. Set FLOW_LIKE_BASE_URL environment variable "
            "or pass base_url= to the client."
        )
    return url.rstrip("/")


def detect_and_resolve(token: str | None = None, **kwargs: str | None) -> dict[str, str]:
    """Auto-detect token type and return the appropriate auth header."""
    if token is None:
        return resolve_auth(**kwargs)
    if token.startswith(PAT_PREFIX):
        return resolve_auth(pat=token)
    if token.startswith(API_KEY_PREFIX):
        return resolve_auth(api_key=token)
    raise AuthenticationError(
        f"Could not detect token type. Token must start with '{PAT_PREFIX}' or '{API_KEY_PREFIX}'."
    )


__all__ = ["resolve_auth", "resolve_base_url", "detect_and_resolve"]
