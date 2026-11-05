"""Inboxes: conversations between people and bots.

Every member gets its own copy of each event (an ``InboxEntry``); copies of one event share
``message_id``. These routes accept people only (a PAT or a sign-in token), not API keys.
"""

from __future__ import annotations

import uuid
from collections.abc import AsyncIterator, Iterator
from dataclasses import dataclass, field
from typing import Any

from ._http import HTTPClient, segment


def new_inbox_message_id() -> str:
    """A fresh client-chosen message id: ``msg_`` and 32 hex digits.

    Reuse it to retry a send: the server returns the first result instead of sending twice.
    """
    return f"msg_{uuid.uuid4().hex}"


def plain_envelope(body: dict[str, Any]) -> dict[str, Any]:
    """The plain envelope v1 around ``body``."""
    return {"v": 1, "enc": "none", "body": body}


@dataclass
class ConversationMember:
    """A principal in a conversation: a user sub or a bot id."""

    principal_id: str
    kind: str
    """``user`` or ``bot``."""
    added_by: str | None = None
    joined_at: str = ""
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class InboxEntry:
    """One principal's copy of one event; pass ``cursor`` back as ``before``, ``after`` or ``up_to``."""

    id: str
    message_id: str
    conversation_id: str
    sender_id: str
    kind: str
    """``message``, ``reaction``, ``edit``, ``delete`` or ``member`` (a join or leave)."""
    envelope: dict[str, Any] = field(default_factory=dict)
    created_at: str = ""
    cursor: str = ""
    raw: dict[str, Any] = field(default_factory=dict)

    @property
    def body(self) -> dict[str, Any] | None:
        """The body of a plain envelope; ``None`` when the envelope is sealed."""
        body = self.envelope.get("body")
        return body if self.envelope.get("enc") == "none" and isinstance(body, dict) else None

    @property
    def text(self) -> str | None:
        """The text of a plain message or edit."""
        text = (self.body or {}).get("text")
        return text if isinstance(text, str) else None


@dataclass
class Conversation:
    """A conversation as the reader sees it."""

    id: str
    kind: str
    """``direct`` (two principals, one conversation per pair) or ``group``."""
    title: str | None = None
    encryption: str = "plain"
    """``plain`` or ``e2e``."""
    created_by: str = ""
    created_at: str = ""
    last_activity_at: str = ""
    members: list[ConversationMember] = field(default_factory=list)
    """Active members, in the order they joined."""
    unread_count: int = 0
    last_entry: InboxEntry | None = None
    """The newest message in the reader's inbox."""
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class InboxEntriesPage:
    """Entries newest first.

    ``next_before``: pass as ``before`` for the next older page. With ``after``, it is set when
    more entries lie between the oldest entry here and ``after``: reload instead of keeping a gap.
    """

    entries: list[InboxEntry] = field(default_factory=list)
    next_before: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class InboxReadMarker:
    """Where the reader's read marker stands; it never moves back."""

    last_read_at: str
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class InboxKey:
    """A public key others seal envelopes to."""

    principal_id: str
    key_id: str
    algorithm: str
    public_key: str
    created_at: str = ""
    raw: dict[str, Any] = field(default_factory=dict)


def _entry(data: dict[str, Any]) -> InboxEntry:
    return InboxEntry(
        id=data["id"],
        message_id=data.get("message_id", ""),
        conversation_id=data.get("conversation_id", ""),
        sender_id=data.get("sender_id", ""),
        kind=data.get("kind", ""),
        envelope=data.get("envelope") or {},
        created_at=data.get("created_at", ""),
        cursor=data.get("cursor", ""),
        raw=data,
    )


def _conversation(data: dict[str, Any]) -> Conversation:
    last_entry = data.get("last_entry")
    return Conversation(
        id=data["id"],
        kind=data.get("kind", ""),
        title=data.get("title"),
        encryption=data.get("encryption", "plain"),
        created_by=data.get("created_by", ""),
        created_at=data.get("created_at", ""),
        last_activity_at=data.get("last_activity_at", ""),
        members=[
            ConversationMember(
                principal_id=member["principal_id"],
                kind=member.get("kind", ""),
                added_by=member.get("added_by"),
                joined_at=member.get("joined_at", ""),
                raw=member,
            )
            for member in data.get("members", [])
        ],
        unread_count=int(data.get("unread_count", 0)),
        last_entry=_entry(last_entry) if last_entry else None,
        raw=data,
    )


def _conversations(data: dict[str, Any]) -> list[Conversation]:
    return [_conversation(item) for item in data.get("conversations", [])]


def _page(data: dict[str, Any]) -> InboxEntriesPage:
    return InboxEntriesPage(
        entries=[_entry(item) for item in data.get("entries", [])],
        next_before=data.get("next_before"),
        raw=data,
    )


def _read_marker(data: dict[str, Any]) -> InboxReadMarker:
    return InboxReadMarker(last_read_at=data.get("last_read_at", ""), raw=data)


def _key(data: dict[str, Any]) -> InboxKey:
    return InboxKey(
        principal_id=data["principal_id"],
        key_id=data["key_id"],
        algorithm=data.get("algorithm", ""),
        public_key=data.get("public_key", ""),
        created_at=data.get("created_at", ""),
        raw=data,
    )


def _keys(data: dict[str, Any]) -> list[InboxKey]:
    return [_key(item) for item in data.get("keys", [])]


def _present(values: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in values.items() if value is not None}


def _create_body(kind: str, members: list[str], title: str | None, encryption: str | None) -> dict[str, Any]:
    return {"kind": kind, "members": list(members), **_present({"title": title, "encryption": encryption})}


def _send_body(
    kind: str,
    envelope: dict[str, Any] | None,
    envelopes: dict[str, dict[str, Any]] | None,
    message_id: str | None,
) -> dict[str, Any]:
    return {
        "message_id": new_inbox_message_id() if message_id is None else message_id,
        "kind": kind,
        **_present({"envelope": envelope, "envelopes": envelopes}),
    }


def _message_body(text: str, reply_to: str | None) -> dict[str, Any]:
    return {"text": text, **_present({"reply_to": reply_to})}


def _read_body(up_to: str | None) -> dict[str, Any]:
    return _present({"up_to": up_to})


def _conversation_path(conversation_id: str, *rest: str) -> str:
    return "/".join(["/inbox/conversations", segment(conversation_id), *(segment(part) for part in rest)])


async def _ahistory(
    client: InboxMixin,
    conversation_id: str,
    as_bot: str | None,
    before: str | None,
    after: str | None,
    page_size: int | None,
) -> AsyncIterator[InboxEntry]:
    while True:
        page = await client.alist_inbox_entries(
            conversation_id, as_bot=as_bot, before=before, after=after, limit=page_size
        )
        for entry in page.entries:
            yield entry
        if not page.next_before:
            return
        before = page.next_before


class InboxMixin(HTTPClient):
    """Conversations, their entries and the public keys for end-to-end encryption."""

    def list_conversations(self, *, as_bot: str | None = None) -> list[Conversation]:
        """Your conversations, newest activity first (at most 200).

        Args:
            as_bot: Read the inbox of one of your bots instead (read-only); lists only the
                conversations you are in too.
        """
        return _conversations(self._json("GET", "/inbox/conversations", params=_present({"as": as_bot})))

    async def alist_conversations(self, *, as_bot: str | None = None) -> list[Conversation]:
        """Async version of ``list_conversations``."""
        return _conversations(await self._ajson("GET", "/inbox/conversations", params=_present({"as": as_bot})))

    def get_conversation(self, conversation_id: str, *, as_bot: str | None = None) -> Conversation:
        """One conversation with its members, the reader's unread count and newest message."""
        return _conversation(self._json("GET", _conversation_path(conversation_id), params=_present({"as": as_bot})))

    async def aget_conversation(self, conversation_id: str, *, as_bot: str | None = None) -> Conversation:
        """Async version of ``get_conversation``."""
        return _conversation(
            await self._ajson("GET", _conversation_path(conversation_id), params=_present({"as": as_bot}))
        )

    def create_conversation(
        self,
        kind: str,
        members: list[str],
        *,
        title: str | None = None,
        encryption: str | None = None,
    ) -> Conversation:
        """Start a conversation; you are always a member.

        Args:
            kind: ``direct`` (exactly one other member; an existing one is returned instead of a
                new one) or ``group``.
            members: Everyone except you: your own bots and people who share an app with you.
            title: Groups only; at most 120 characters.
            encryption: ``plain`` (default) or ``e2e``.
        """
        return _conversation(
            self._json("POST", "/inbox/conversations", json=_create_body(kind, members, title, encryption))
        )

    async def acreate_conversation(
        self,
        kind: str,
        members: list[str],
        *,
        title: str | None = None,
        encryption: str | None = None,
    ) -> Conversation:
        """Async version of ``create_conversation``."""
        return _conversation(
            await self._ajson("POST", "/inbox/conversations", json=_create_body(kind, members, title, encryption))
        )

    def rename_conversation(self, conversation_id: str, title: str | None) -> Conversation:
        """Rename a group; an empty title or ``None`` removes it."""
        return _conversation(self._json("PATCH", _conversation_path(conversation_id), json={"title": title}))

    async def arename_conversation(self, conversation_id: str, title: str | None) -> Conversation:
        """Async version of ``rename_conversation``."""
        return _conversation(await self._ajson("PATCH", _conversation_path(conversation_id), json={"title": title}))

    def add_conversation_members(self, conversation_id: str, members: list[str]) -> Conversation:
        """Add your own bots or people who share an app with you; current members are skipped."""
        return _conversation(
            self._json("POST", _conversation_path(conversation_id, "members"), json={"members": list(members)})
        )

    async def aadd_conversation_members(self, conversation_id: str, members: list[str]) -> Conversation:
        """Async version of ``add_conversation_members``."""
        return _conversation(
            await self._ajson("POST", _conversation_path(conversation_id, "members"), json={"members": list(members)})
        )

    def remove_conversation_member(self, conversation_id: str, principal_id: str) -> None:
        """Leave (your own id), remove one of your bots, or anyone from a group you started."""
        self._request("DELETE", _conversation_path(conversation_id, "members", principal_id))

    async def aremove_conversation_member(self, conversation_id: str, principal_id: str) -> None:
        """Async version of ``remove_conversation_member``."""
        await self._arequest("DELETE", _conversation_path(conversation_id, "members", principal_id))

    def list_inbox_entries(
        self,
        conversation_id: str,
        *,
        as_bot: str | None = None,
        before: str | None = None,
        after: str | None = None,
        limit: int | None = None,
    ) -> InboxEntriesPage:
        """One page of the reader's copy of a conversation, newest first.

        Args:
            as_bot: Read one of your bots' copy instead (read-only).
            before: Only entries older than this cursor.
            after: Only entries newer than this cursor.
            limit: 1 to 100 entries; 50 by default.
        """
        params = _present({"as": as_bot, "before": before, "after": after, "limit": limit})
        return _page(self._json("GET", _conversation_path(conversation_id, "entries"), params=params))

    async def alist_inbox_entries(
        self,
        conversation_id: str,
        *,
        as_bot: str | None = None,
        before: str | None = None,
        after: str | None = None,
        limit: int | None = None,
    ) -> InboxEntriesPage:
        """Async version of ``list_inbox_entries``."""
        params = _present({"as": as_bot, "before": before, "after": after, "limit": limit})
        return _page(await self._ajson("GET", _conversation_path(conversation_id, "entries"), params=params))

    def iter_inbox_entries(
        self,
        conversation_id: str,
        *,
        as_bot: str | None = None,
        before: str | None = None,
        after: str | None = None,
        page_size: int | None = None,
    ) -> Iterator[InboxEntry]:
        """Every entry from ``before`` (or the newest) down to ``after`` (or the start), newest first.

        Fetches page after page, following ``next_before``, as you iterate.
        """
        while True:
            page = self.list_inbox_entries(
                conversation_id, as_bot=as_bot, before=before, after=after, limit=page_size
            )
            yield from page.entries
            if not page.next_before:
                return
            before = page.next_before

    async def aiter_inbox_entries(
        self,
        conversation_id: str,
        *,
        as_bot: str | None = None,
        before: str | None = None,
        after: str | None = None,
        page_size: int | None = None,
    ) -> AsyncIterator[InboxEntry]:
        """Async version of ``iter_inbox_entries``: ``async for entry in await client.aiter_inbox_entries(...)``."""
        return _ahistory(self, conversation_id, as_bot, before, after, page_size)

    def send_inbox_entry(
        self,
        conversation_id: str,
        kind: str,
        *,
        envelope: dict[str, Any] | None = None,
        envelopes: dict[str, dict[str, Any]] | None = None,
        message_id: str | None = None,
    ) -> InboxEntry:
        """Send any event and return your copy.

        Args:
            kind: ``message``, ``reaction``, ``edit`` or ``delete``.
            envelope: Plain conversations: ``{"v": 1, "enc": "none", "body": {...}}``.
            envelopes: End-to-end encrypted conversations: one sealed envelope per active member,
                keyed by principal id.
            message_id: 1 to 64 characters of ``[A-Za-z0-9_-]``; generated when left out.
                Sending the same id again returns the first result.
        """
        body = _send_body(kind, envelope, envelopes, message_id)
        return _entry(self._json("POST", _conversation_path(conversation_id, "entries"), json=body))

    async def asend_inbox_entry(
        self,
        conversation_id: str,
        kind: str,
        *,
        envelope: dict[str, Any] | None = None,
        envelopes: dict[str, dict[str, Any]] | None = None,
        message_id: str | None = None,
    ) -> InboxEntry:
        """Async version of ``send_inbox_entry``."""
        body = _send_body(kind, envelope, envelopes, message_id)
        return _entry(await self._ajson("POST", _conversation_path(conversation_id, "entries"), json=body))

    def send_inbox_message(
        self, conversation_id: str, text: str, *, reply_to: str | None = None, message_id: str | None = None
    ) -> InboxEntry:
        """Send a text message; ``reply_to`` names the ``message_id`` it answers."""
        return self.send_inbox_entry(
            conversation_id,
            "message",
            envelope=plain_envelope(_message_body(text, reply_to)),
            message_id=message_id,
        )

    async def asend_inbox_message(
        self, conversation_id: str, text: str, *, reply_to: str | None = None, message_id: str | None = None
    ) -> InboxEntry:
        """Async version of ``send_inbox_message``."""
        return await self.asend_inbox_entry(
            conversation_id,
            "message",
            envelope=plain_envelope(_message_body(text, reply_to)),
            message_id=message_id,
        )

    def react_to_inbox_message(
        self, conversation_id: str, target: str, emoji: str, *, op: str = "add", message_id: str | None = None
    ) -> InboxEntry:
        """Add (or with ``op="remove"`` remove) a reaction to the message ``target`` (a ``message_id``)."""
        envelope = plain_envelope({"target": target, "emoji": emoji, "op": op})
        return self.send_inbox_entry(conversation_id, "reaction", envelope=envelope, message_id=message_id)

    async def areact_to_inbox_message(
        self, conversation_id: str, target: str, emoji: str, *, op: str = "add", message_id: str | None = None
    ) -> InboxEntry:
        """Async version of ``react_to_inbox_message``."""
        envelope = plain_envelope({"target": target, "emoji": emoji, "op": op})
        return await self.asend_inbox_entry(conversation_id, "reaction", envelope=envelope, message_id=message_id)

    def edit_inbox_message(
        self, conversation_id: str, target: str, text: str, *, message_id: str | None = None
    ) -> InboxEntry:
        """Replace the text of a message you sent."""
        envelope = plain_envelope({"target": target, "text": text})
        return self.send_inbox_entry(conversation_id, "edit", envelope=envelope, message_id=message_id)

    async def aedit_inbox_message(
        self, conversation_id: str, target: str, text: str, *, message_id: str | None = None
    ) -> InboxEntry:
        """Async version of ``edit_inbox_message``."""
        envelope = plain_envelope({"target": target, "text": text})
        return await self.asend_inbox_entry(conversation_id, "edit", envelope=envelope, message_id=message_id)

    def delete_inbox_message(self, conversation_id: str, target: str, *, message_id: str | None = None) -> InboxEntry:
        """Delete a message you sent."""
        envelope = plain_envelope({"target": target})
        return self.send_inbox_entry(conversation_id, "delete", envelope=envelope, message_id=message_id)

    async def adelete_inbox_message(
        self, conversation_id: str, target: str, *, message_id: str | None = None
    ) -> InboxEntry:
        """Async version of ``delete_inbox_message``."""
        envelope = plain_envelope({"target": target})
        return await self.asend_inbox_entry(conversation_id, "delete", envelope=envelope, message_id=message_id)

    def mark_conversation_read(self, conversation_id: str, up_to: str | None = None) -> InboxReadMarker:
        """Move your read marker to the entry with cursor ``up_to``, or to now."""
        return _read_marker(self._json("POST", _conversation_path(conversation_id, "read"), json=_read_body(up_to)))

    async def amark_conversation_read(self, conversation_id: str, up_to: str | None = None) -> InboxReadMarker:
        """Async version of ``mark_conversation_read``."""
        return _read_marker(
            await self._ajson("POST", _conversation_path(conversation_id, "read"), json=_read_body(up_to))
        )

    def get_inbox_keys(self, principal_ids: list[str]) -> list[InboxKey]:
        """The active public keys of up to 50 people and bots."""
        return _keys(self._json("GET", "/inbox/keys", params={"principals": ",".join(principal_ids)}))

    async def aget_inbox_keys(self, principal_ids: list[str]) -> list[InboxKey]:
        """Async version of ``get_inbox_keys``."""
        return _keys(await self._ajson("GET", "/inbox/keys", params={"principals": ",".join(principal_ids)}))

    def register_inbox_key(
        self, key_id: str, algorithm: str, public_key: str, *, principal_id: str | None = None
    ) -> InboxKey:
        """Register a public key; keys never change, so register a new ``key_id`` to rotate.

        Args:
            principal_id: Left out for your own key; the id of a bot you own registers that bot's key.
        """
        body = {"key_id": key_id, "algorithm": algorithm, "public_key": public_key, **_present({"principal_id": principal_id})}
        return _key(self._json("PUT", "/inbox/keys", json=body))

    async def aregister_inbox_key(
        self, key_id: str, algorithm: str, public_key: str, *, principal_id: str | None = None
    ) -> InboxKey:
        """Async version of ``register_inbox_key``."""
        body = {"key_id": key_id, "algorithm": algorithm, "public_key": public_key, **_present({"principal_id": principal_id})}
        return _key(await self._ajson("PUT", "/inbox/keys", json=body))

    def revoke_inbox_key(self, key_id: str, *, principal_id: str | None = None) -> None:
        """Revoke your key, or with ``principal_id`` one of your bots' keys."""
        self._request("DELETE", f"/inbox/keys/{segment(key_id)}", params=_present({"principal_id": principal_id}))

    async def arevoke_inbox_key(self, key_id: str, *, principal_id: str | None = None) -> None:
        """Async version of ``revoke_inbox_key``."""
        await self._arequest("DELETE", f"/inbox/keys/{segment(key_id)}", params=_present({"principal_id": principal_id}))


__all__ = [
    "Conversation",
    "ConversationMember",
    "InboxEntriesPage",
    "InboxEntry",
    "InboxKey",
    "InboxMixin",
    "InboxReadMarker",
    "new_inbox_message_id",
    "plain_envelope",
]
