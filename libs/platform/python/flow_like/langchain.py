"""LangChain integrations for the Flow-Like API.

Provides :class:`FlowLikeChatModel` (a LangChain ``BaseChatModel``) and
:class:`FlowLikeEmbeddings` (a LangChain ``Embeddings`` implementation)
that delegate to the Flow-Like REST endpoints.
"""

from __future__ import annotations

from typing import Any, Optional

import httpx
from langchain_core.callbacks import CallbackManagerForLLMRun
from langchain_core.embeddings import Embeddings
from langchain_core.language_models.chat_models import BaseChatModel
from langchain_core.messages import AIMessage, BaseMessage
from langchain_core.outputs import ChatGeneration, ChatResult

_ROLE_MAP = {
    "human": "user",
    "ai": "assistant",
    "system": "system",
    "tool": "tool",
}


def _build_headers(token: str) -> dict[str, str]:
    """Build HTTP headers for a Flow-Like API request.

    Args:
        token: A PAT (``pat_…``) or API key (``key_…``).

    Returns:
        Header dict with the appropriate auth entry.
    """
    headers = {"Content-Type": "application/json"}
    if token.startswith("pat_"):
        headers["Authorization"] = token
    else:
        headers["X-API-Key"] = token
    return headers


def _to_api_message(msg: BaseMessage) -> dict[str, str]:
    """Convert a LangChain message to the Flow-Like API format.

    Args:
        msg: A LangChain ``BaseMessage`` instance.

    Returns:
        Dict with ``role`` and ``content`` keys.
    """
    return {
        "role": _ROLE_MAP.get(msg.type, "user"),
        "content": str(msg.content),
    }


class FlowLikeChatModel(BaseChatModel):
    """LangChain chat model backed by the Flow-Like completions API.

    Attributes:
        base_url: Flow-Like API base URL.
        token: Authentication token (PAT or API key).
        bit_id: Identifier of the model bit.
        temperature: Sampling temperature override.
        max_tokens: Maximum tokens to generate.
        top_p: Nucleus-sampling probability mass.
    """

    base_url: str
    token: str
    bit_id: str
    temperature: Optional[float] = None
    max_tokens: Optional[int] = None
    top_p: Optional[float] = None

    @property
    def _llm_type(self) -> str:
        """Return the LLM type identifier used by LangChain."""
        return "flow-like"

    def _generate(
        self,
        messages: list[BaseMessage],
        stop: list[str] | None = None,
        run_manager: CallbackManagerForLLMRun | None = None,
        **kwargs: Any,
    ) -> ChatResult:
        """Synchronously generate a chat completion.

        Args:
            messages: Conversation history as LangChain messages.
            stop: Optional stop sequences.
            run_manager: LangChain callback manager.
            **kwargs: Extra parameters forwarded to the API.

        Returns:
            A ``ChatResult`` containing the model's response.
        """
        body: dict[str, Any] = {
            "messages": [_to_api_message(m) for m in messages],
            "model": self.bit_id,
        }
        if self.temperature is not None:
            body["temperature"] = self.temperature
        if self.max_tokens is not None:
            body["max_tokens"] = self.max_tokens
        if self.top_p is not None:
            body["top_p"] = self.top_p
        if stop:
            body["stop"] = stop

        resp = httpx.post(
            f"{self.base_url}/api/v1/chat/completions",
            headers=_build_headers(self.token),
            json=body,
            timeout=120,
        )
        resp.raise_for_status()
        data = resp.json()

        text = data["choices"][0]["message"]["content"]
        return ChatResult(generations=[ChatGeneration(message=AIMessage(content=text))])


class FlowLikeEmbeddings(Embeddings):
    """LangChain embeddings wrapper backed by the Flow-Like embeddings API."""

    def __init__(self, *, base_url: str, token: str, bit_id: str) -> None:
        """Initialise the embeddings wrapper.

        Args:
            base_url: Flow-Like API base URL.
            token: Authentication token (PAT or API key).
            bit_id: Identifier of the embeddings bit.
        """
        super().__init__()
        self.base_url = base_url
        self.token = token
        self.bit_id = bit_id

    def _embed(self, texts: list[str], embed_type: str) -> list[list[float]]:
        """Send a synchronous embedding request.

        Args:
            texts: Input texts to embed.
            embed_type: Either ``"document"`` or ``"query"``.

        Returns:
            List of embedding vectors.
        """
        resp = httpx.post(
            f"{self.base_url}/api/v1/embeddings/embed",
            headers=_build_headers(self.token),
            json={"model": self.bit_id, "input": texts, "embed_type": embed_type},
            timeout=120,
        )
        resp.raise_for_status()
        return resp.json()["embeddings"]

    def embed_documents(self, texts: list[str]) -> list[list[float]]:
        """Embed a list of documents.

        Args:
            texts: Document texts to embed.

        Returns:
            List of embedding vectors, one per document.
        """
        return self._embed(texts, "document")

    def embed_query(self, text: str) -> list[float]:
        """Embed a single query string.

        Args:
            text: The query text.

        Returns:
            Embedding vector for the query.
        """
        return self._embed([text], "query")[0]

    async def _aembed(self, texts: list[str], embed_type: str) -> list[list[float]]:
        """Send an asynchronous embedding request.

        Args:
            texts: Input texts to embed.
            embed_type: Either ``"document"`` or ``"query"``.

        Returns:
            List of embedding vectors.
        """
        async with httpx.AsyncClient() as client:
            resp = await client.post(
                f"{self.base_url}/api/v1/embeddings/embed",
                headers=_build_headers(self.token),
                json={"model": self.bit_id, "input": texts, "embed_type": embed_type},
                timeout=120,
            )
            resp.raise_for_status()
            return resp.json()["embeddings"]

    async def aembed_documents(self, texts: list[str]) -> list[list[float]]:
        """Asynchronously embed a list of documents.

        Args:
            texts: Document texts to embed.

        Returns:
            List of embedding vectors, one per document.
        """
        return await self._aembed(texts, "document")

    async def aembed_query(self, text: str) -> list[float]:
        """Asynchronously embed a single query string.

        Args:
            text: The query text.

        Returns:
            Embedding vector for the query.
        """
        return (await self._aembed([text], "query"))[0]
