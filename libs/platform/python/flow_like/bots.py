"""Bots: assistants that act on behalf of their owner.

These routes accept people only (a PAT or a sign-in token), not API keys or running flows.
"""

from __future__ import annotations

from dataclasses import dataclass, field, fields, is_dataclass
from typing import Any

from ._http import HTTPClient, segment


class _Unset:
    def __repr__(self) -> str:
        return "UNSET"


UNSET: Any = _Unset()
"""Default of optional update arguments: the field is not sent. ``None`` clears it instead."""

_DEFAULT_APPEARANCE: dict[str, Any] = {"kind": "pip"}


@dataclass
class BotModelPolicy:
    """The model a bot prefers; hosts try ``fallbacks`` in order where ``primary`` is unavailable."""

    primary: str | None = None
    fallbacks: list[str] = field(default_factory=list)
    # A System One Bit; None keeps the usual response rules.
    decision: str | None = None


@dataclass
class BotClassification:
    """How sensitive the data a bot handles is: ``public``, ``internal``, ``confidential`` or ``restricted``."""

    classification: str | None = None
    pii: bool = False


@dataclass
class Bot:
    """A bot as its owner sees it. ``id`` is also the bot's sub."""

    id: str
    owner_id: str
    system: str | None = None
    """``general`` (Assistant) or ``flux`` for the built-in bots; ``None`` for bots you created."""
    preset_id: str | None = None
    name: str | None = None
    """``None`` on a built-in bot that was never renamed: show its localized default name."""
    description: str | None = None
    instructions: str | None = None
    appearance: dict[str, Any] = field(default_factory=dict)
    model: BotModelPolicy = field(default_factory=BotModelPolicy)
    classification: BotClassification = field(default_factory=BotClassification)
    capabilities: list[str] = field(default_factory=list)
    profile_id: str | None = None
    location: str = "server"
    """``server``, ``desktop`` or ``device``."""
    device_id: str | None = None
    import_key: str | None = None
    created_at: str = ""
    updated_at: str = ""
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class BotApp:
    """An app the bot may use; ``owner_role`` bounds what it may do there (``None`` once you left)."""

    app_id: str
    added_at: str = ""
    owner_role: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class BotSecret:
    """A secret's metadata. No response ever carries its value."""

    name: str
    encryption: str = "server"
    """``server`` (encrypted by the server) or ``sealed`` (sealed to the bot's key)."""
    description: str | None = None
    created_at: str = ""
    updated_at: str = ""
    last_used_at: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class BotMemoryEvidence:
    """Where in a chat a fact was said."""

    seq: int
    message_id: str
    quote: str = ""
    at: str = ""
    speaker: dict[str, Any] | None = None
    """``{"id": ..., "name": ...}``; the id is the identity, the name only a display hint."""


@dataclass
class BotMemoryFact:
    """One thing the bot remembers."""

    id: str
    text: str
    kind: str = ""
    subject: str = ""
    subject_id: str | None = None
    object_id: str | None = None
    relation: str | None = None
    source: str = ""
    confidence: float = 0.0
    recorded_at: str = ""
    valid_from: str | None = None
    valid_to: str | None = None
    geojson: dict[str, Any] | None = None
    time: str | None = None
    expires_at: str | None = None
    superseded_by: str | None = None
    """The id of the fact that replaced this one; ``None`` while it still holds."""
    evidence: list[BotMemoryEvidence] = field(default_factory=list)
    conversation_scope: str | None = None
    """The conversation the fact was learned in; ``None`` in the owner's direct chat."""
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass
class BotMemoryEntity:
    """A person, place, organization, project or thing the bot knows about."""

    id: str
    """Unique only within its ``conversation_scope``."""
    name: str
    kind: str = "thing"
    """``person``, ``place``, ``organization``, ``project`` or ``thing``."""
    owner: bool = False
    """True for the entity standing for the bot's owner (``e_user``)."""
    participant: str | None = None
    aliases: list[str] = field(default_factory=list)
    fact_count: int = 0
    created_at: str = ""
    updated_at: str = ""
    conversation_scope: str | None = None
    raw: dict[str, Any] = field(default_factory=dict)


def _wire(value: Any) -> Any:
    if is_dataclass(value) and not isinstance(value, type):
        return {
            item.name: getattr(value, item.name)
            for item in fields(value)
            if item.name != "raw" and not (item.name == "decision" and getattr(value, item.name) is None)
        }
    return value


def _bot(data: dict[str, Any]) -> Bot:
    model = data.get("model") or {}
    classification = data.get("classification") or {}
    return Bot(
        id=data["id"],
        owner_id=data.get("owner_id", ""),
        system=data.get("system"),
        preset_id=data.get("preset_id"),
        name=data.get("name"),
        description=data.get("description"),
        instructions=data.get("instructions"),
        appearance=data.get("appearance") or {},
        model=BotModelPolicy(
            primary=model.get("primary"),
            fallbacks=list(model.get("fallbacks") or []),
            decision=model.get("decision"),
        ),
        classification=BotClassification(
            classification=classification.get("classification"),
            pii=bool(classification.get("pii", False)),
        ),
        capabilities=list(data.get("capabilities") or []),
        profile_id=data.get("profile_id"),
        location=data.get("location", "server"),
        device_id=data.get("device_id"),
        import_key=data.get("import_key"),
        created_at=data.get("created_at", ""),
        updated_at=data.get("updated_at", ""),
        raw=data,
    )


def _bots(data: dict[str, Any]) -> list[Bot]:
    return [_bot(item) for item in data.get("bots", [])]


def _bot_apps(data: dict[str, Any]) -> list[BotApp]:
    return [
        BotApp(app_id=item["app_id"], added_at=item.get("added_at", ""), owner_role=item.get("owner_role"), raw=item)
        for item in data.get("apps", [])
    ]


def _bot_secret(data: dict[str, Any]) -> BotSecret:
    return BotSecret(
        name=data["name"],
        encryption=data.get("encryption", "server"),
        description=data.get("description"),
        created_at=data.get("created_at", ""),
        updated_at=data.get("updated_at", ""),
        last_used_at=data.get("last_used_at"),
        raw=data,
    )


def _bot_secrets(data: dict[str, Any]) -> list[BotSecret]:
    return [_bot_secret(item) for item in data.get("secrets", [])]


def _create_body(
    name: str,
    appearance: dict[str, Any] | None,
    optional: dict[str, Any],
) -> dict[str, Any]:
    body: dict[str, Any] = {"name": name, "appearance": dict(_DEFAULT_APPEARANCE) if appearance is None else appearance}
    body.update({key: _wire(value) for key, value in optional.items() if value is not None})
    return body


def _changes(changes: dict[str, Any]) -> dict[str, Any]:
    return {key: _wire(value) for key, value in changes.items() if value is not UNSET}


def _secret_body(value: str | None, sealed: dict[str, Any] | None, description: Any) -> dict[str, Any]:
    body: dict[str, Any] = {}
    if value is not None:
        body["value"] = value
    if sealed is not None:
        body["sealed"] = sealed
    if description is not UNSET:
        body["description"] = description
    return body


def _memory_fact(data: dict[str, Any]) -> BotMemoryFact:
    return BotMemoryFact(
        id=data["id"],
        text=data.get("text", ""),
        kind=data.get("kind", ""),
        subject=data.get("subject", ""),
        subject_id=data.get("subject_id"),
        object_id=data.get("object_id"),
        relation=data.get("relation"),
        source=data.get("source", ""),
        confidence=float(data.get("confidence", 0.0)),
        recorded_at=data.get("recorded_at", ""),
        valid_from=data.get("valid_from"),
        valid_to=data.get("valid_to"),
        geojson=data.get("geojson"),
        time=data.get("time"),
        expires_at=data.get("expires_at"),
        superseded_by=data.get("superseded_by"),
        evidence=[
            BotMemoryEvidence(
                seq=item["seq"],
                message_id=item.get("message_id", ""),
                quote=item.get("quote", ""),
                at=item.get("at", ""),
                speaker=item.get("speaker"),
            )
            for item in data.get("evidence") or []
        ],
        conversation_scope=data.get("conversation_scope"),
        raw=data,
    )


def _memory_facts(data: dict[str, Any]) -> list[BotMemoryFact]:
    return [_memory_fact(item) for item in data.get("facts", [])]


def _memory_entity(data: dict[str, Any]) -> BotMemoryEntity:
    return BotMemoryEntity(
        id=data["id"],
        name=data.get("name", ""),
        kind=data.get("kind", "thing"),
        owner=bool(data.get("owner", False)),
        participant=data.get("participant"),
        aliases=list(data.get("aliases") or []),
        fact_count=int(data.get("fact_count", 0)),
        created_at=data.get("created_at", ""),
        updated_at=data.get("updated_at", ""),
        conversation_scope=data.get("conversation_scope"),
        raw=data,
    )


def _memory_entities(data: dict[str, Any]) -> list[BotMemoryEntity]:
    return [_memory_entity(item) for item in data.get("entities", [])]


def _forgotten_facts(data: dict[str, Any]) -> list[BotMemoryFact]:
    return [_memory_fact(item) for item in data.get("forgotten", [])]


def _scope(scope: str | None) -> dict[str, str] | None:
    return None if scope is None else {"scope": scope}


def _entity_changes(name: str | None, kind: str | None) -> dict[str, Any]:
    return {key: value for key, value in {"name": name, "kind": kind}.items() if value is not None}


def _bot_path(bot_id: str, *rest: str) -> str:
    return "/".join(["/bots", segment(bot_id), *(segment(part) for part in rest)])


class BotsMixin(HTTPClient):
    """Create, configure and remove your bots, their apps, their write-only secrets and what they remember."""

    def list_bots(self) -> list[Bot]:
        """Assistant and Flux first (set up on the first call), then your bots, oldest first."""
        return _bots(self._json("GET", "/bots"))

    async def alist_bots(self) -> list[Bot]:
        """Async version of ``list_bots``."""
        return _bots(await self._ajson("GET", "/bots"))

    def get_bot(self, bot_id: str) -> Bot:
        """One of your bots; someone else's bot reads as missing (``NotFoundError``)."""
        return _bot(self._json("GET", _bot_path(bot_id)))

    async def aget_bot(self, bot_id: str) -> Bot:
        """Async version of ``get_bot``."""
        return _bot(await self._ajson("GET", _bot_path(bot_id)))

    def create_bot(
        self,
        name: str,
        *,
        appearance: dict[str, Any] | None = None,
        preset_id: str | None = None,
        description: str | None = None,
        instructions: str | None = None,
        model: BotModelPolicy | dict[str, Any] | None = None,
        classification: BotClassification | dict[str, Any] | None = None,
        capabilities: list[str] | None = None,
        profile_id: str | None = None,
        location: str | None = None,
        device_id: str | None = None,
    ) -> Bot:
        """Create a bot. It starts with ``capabilities`` and nothing else.

        Args:
            name: 1 to 64 characters.
            appearance: The look from the bot builder, an object with a string ``kind``;
                defaults to ``{"kind": "pip"}``.
            capabilities: Capability ids such as ``internet``; ``profile_apps`` needs ``profile_id``,
                ``computer_use``/``browser_use``/``command_line`` need a desktop or device location.
            location: ``server`` (default), ``desktop`` or ``device`` (needs ``device_id``).
        """
        optional = {
            "preset_id": preset_id,
            "description": description,
            "instructions": instructions,
            "model": model,
            "classification": classification,
            "capabilities": capabilities,
            "profile_id": profile_id,
            "location": location,
            "device_id": device_id,
        }
        return _bot(self._json("POST", "/bots", json=_create_body(name, appearance, optional)))

    async def acreate_bot(
        self,
        name: str,
        *,
        appearance: dict[str, Any] | None = None,
        preset_id: str | None = None,
        description: str | None = None,
        instructions: str | None = None,
        model: BotModelPolicy | dict[str, Any] | None = None,
        classification: BotClassification | dict[str, Any] | None = None,
        capabilities: list[str] | None = None,
        profile_id: str | None = None,
        location: str | None = None,
        device_id: str | None = None,
    ) -> Bot:
        """Async version of ``create_bot``."""
        optional = {
            "preset_id": preset_id,
            "description": description,
            "instructions": instructions,
            "model": model,
            "classification": classification,
            "capabilities": capabilities,
            "profile_id": profile_id,
            "location": location,
            "device_id": device_id,
        }
        return _bot(await self._ajson("POST", "/bots", json=_create_body(name, appearance, optional)))

    def update_bot(
        self,
        bot_id: str,
        *,
        name: str | None = UNSET,
        description: str | None = UNSET,
        instructions: str | None = UNSET,
        appearance: dict[str, Any] = UNSET,
        model: BotModelPolicy | dict[str, Any] = UNSET,
        classification: BotClassification | dict[str, Any] = UNSET,
        capabilities: list[str] = UNSET,
        profile_id: str | None = UNSET,
        location: str = UNSET,
        device_id: str | None = UNSET,
    ) -> Bot:
        """Change a bot. Arguments left out stay as they are; ``None`` clears a nullable field.

        ``name=None`` resets a built-in bot to its localized default name. ``model`` and
        ``capabilities`` replace the whole value; Flux's capabilities can't be changed.
        """
        changes = _changes({
            "name": name,
            "description": description,
            "instructions": instructions,
            "appearance": appearance,
            "model": model,
            "classification": classification,
            "capabilities": capabilities,
            "profile_id": profile_id,
            "location": location,
            "device_id": device_id,
        })
        return _bot(self._json("PATCH", _bot_path(bot_id), json=changes))

    async def aupdate_bot(
        self,
        bot_id: str,
        *,
        name: str | None = UNSET,
        description: str | None = UNSET,
        instructions: str | None = UNSET,
        appearance: dict[str, Any] = UNSET,
        model: BotModelPolicy | dict[str, Any] = UNSET,
        classification: BotClassification | dict[str, Any] = UNSET,
        capabilities: list[str] = UNSET,
        profile_id: str | None = UNSET,
        location: str = UNSET,
        device_id: str | None = UNSET,
    ) -> Bot:
        """Async version of ``update_bot``."""
        changes = _changes({
            "name": name,
            "description": description,
            "instructions": instructions,
            "appearance": appearance,
            "model": model,
            "classification": classification,
            "capabilities": capabilities,
            "profile_id": profile_id,
            "location": location,
            "device_id": device_id,
        })
        return _bot(await self._ajson("PATCH", _bot_path(bot_id), json=changes))

    def delete_bot(self, bot_id: str) -> None:
        """Delete a bot you created; Assistant and Flux can't be deleted (409 ``BOT_SYSTEM_UNDELETABLE``)."""
        self._request("DELETE", _bot_path(bot_id))

    async def adelete_bot(self, bot_id: str) -> None:
        """Async version of ``delete_bot``."""
        await self._arequest("DELETE", _bot_path(bot_id))

    def list_bot_apps(self, bot_id: str) -> list[BotApp]:
        """The apps attached to the bot, oldest first, each with your current role there."""
        return _bot_apps(self._json("GET", _bot_path(bot_id, "apps")))

    async def alist_bot_apps(self, bot_id: str) -> list[BotApp]:
        """Async version of ``list_bot_apps``."""
        return _bot_apps(await self._ajson("GET", _bot_path(bot_id, "apps")))

    def attach_bot_app(self, bot_id: str, app_id: str) -> None:
        """Let the bot use an app you are a member of, with your role there."""
        self._request("PUT", _bot_path(bot_id, "apps", app_id))

    async def aattach_bot_app(self, bot_id: str, app_id: str) -> None:
        """Async version of ``attach_bot_app``."""
        await self._arequest("PUT", _bot_path(bot_id, "apps", app_id))

    def detach_bot_app(self, bot_id: str, app_id: str) -> None:
        """Stop the bot from using an app."""
        self._request("DELETE", _bot_path(bot_id, "apps", app_id))

    async def adetach_bot_app(self, bot_id: str, app_id: str) -> None:
        """Async version of ``detach_bot_app``."""
        await self._arequest("DELETE", _bot_path(bot_id, "apps", app_id))

    def list_bot_secrets(self, bot_id: str) -> list[BotSecret]:
        """Names and metadata of the bot's secrets, sorted by name; never their values."""
        return _bot_secrets(self._json("GET", _bot_path(bot_id, "secrets")))

    async def alist_bot_secrets(self, bot_id: str) -> list[BotSecret]:
        """Async version of ``list_bot_secrets``."""
        return _bot_secrets(await self._ajson("GET", _bot_path(bot_id, "secrets")))

    def set_bot_secret(
        self,
        bot_id: str,
        name: str,
        value: str | None = None,
        *,
        sealed: dict[str, Any] | None = None,
        description: str | None = UNSET,
    ) -> BotSecret:
        """Store or replace a secret. Values are write-only.

        Args:
            name: Like an environment variable: ``^[A-Z][A-Z0-9_]{0,63}$``.
            value: The secret, encrypted by the server. Pass exactly one of ``value`` and ``sealed``.
            sealed: An envelope sealed to the bot's key; only the bot's host can open it.
            description: Left out keeps the current description; ``None`` clears it.
        """
        return _bot_secret(
            self._json("PUT", _bot_path(bot_id, "secrets", name), json=_secret_body(value, sealed, description))
        )

    async def aset_bot_secret(
        self,
        bot_id: str,
        name: str,
        value: str | None = None,
        *,
        sealed: dict[str, Any] | None = None,
        description: str | None = UNSET,
    ) -> BotSecret:
        """Async version of ``set_bot_secret``."""
        return _bot_secret(
            await self._ajson("PUT", _bot_path(bot_id, "secrets", name), json=_secret_body(value, sealed, description))
        )

    def delete_bot_secret(self, bot_id: str, name: str) -> None:
        """Delete a secret; deleting a missing one succeeds."""
        self._request("DELETE", _bot_path(bot_id, "secrets", name))

    async def adelete_bot_secret(self, bot_id: str, name: str) -> None:
        """Async version of ``delete_bot_secret``."""
        await self._arequest("DELETE", _bot_path(bot_id, "secrets", name))

    def list_bot_memory_facts(self, bot_id: str, *, include_superseded: bool = False) -> list[BotMemoryFact]:
        """What the bot remembers, newest first. ``include_superseded`` adds facts newer ones replaced."""
        params = {"include_superseded": True} if include_superseded else None
        return _memory_facts(self._json("GET", _bot_path(bot_id, "memory", "facts"), params=params))

    async def alist_bot_memory_facts(self, bot_id: str, *, include_superseded: bool = False) -> list[BotMemoryFact]:
        """Async version of ``list_bot_memory_facts``."""
        params = {"include_superseded": True} if include_superseded else None
        return _memory_facts(await self._ajson("GET", _bot_path(bot_id, "memory", "facts"), params=params))

    def forget_bot_memory_fact(self, bot_id: str, fact_id: str) -> BotMemoryFact:
        """Make the bot forget one fact; returns it."""
        return _memory_fact(self._json("DELETE", _bot_path(bot_id, "memory", "facts", fact_id)))

    async def aforget_bot_memory_fact(self, bot_id: str, fact_id: str) -> BotMemoryFact:
        """Async version of ``forget_bot_memory_fact``."""
        return _memory_fact(await self._ajson("DELETE", _bot_path(bot_id, "memory", "facts", fact_id)))

    def reset_bot_memory(self, bot_id: str) -> None:
        """Forget everything the bot remembered. 409 while another process works on the memory: retry later."""
        self._request("DELETE", _bot_path(bot_id, "memory"))

    async def areset_bot_memory(self, bot_id: str) -> None:
        """Async version of ``reset_bot_memory``."""
        await self._arequest("DELETE", _bot_path(bot_id, "memory"))

    def list_bot_memory_entities(self, bot_id: str) -> list[BotMemoryEntity]:
        """The people, places, organizations, projects and things the bot knows about."""
        return _memory_entities(self._json("GET", _bot_path(bot_id, "memory", "entities")))

    async def alist_bot_memory_entities(self, bot_id: str) -> list[BotMemoryEntity]:
        """Async version of ``list_bot_memory_entities``."""
        return _memory_entities(await self._ajson("GET", _bot_path(bot_id, "memory", "entities")))

    def update_bot_memory_entity(
        self,
        bot_id: str,
        entity_id: str,
        *,
        name: str | None = None,
        kind: str | None = None,
        scope: str | None = None,
    ) -> BotMemoryEntity:
        """Rename an entity or change its kind.

        Args:
            scope: The entity's ``conversation_scope``; ids are unique only within one.
                Leave out for the owner's direct chat.
        """
        return _memory_entity(
            self._json(
                "PATCH",
                _bot_path(bot_id, "memory", "entities", entity_id),
                json=_entity_changes(name, kind),
                params=_scope(scope),
            )
        )

    async def aupdate_bot_memory_entity(
        self,
        bot_id: str,
        entity_id: str,
        *,
        name: str | None = None,
        kind: str | None = None,
        scope: str | None = None,
    ) -> BotMemoryEntity:
        """Async version of ``update_bot_memory_entity``."""
        return _memory_entity(
            await self._ajson(
                "PATCH",
                _bot_path(bot_id, "memory", "entities", entity_id),
                json=_entity_changes(name, kind),
                params=_scope(scope),
            )
        )

    def merge_bot_memory_entity(
        self, bot_id: str, entity_id: str, into: str, *, scope: str | None = None
    ) -> BotMemoryEntity:
        """Fold an entity into ``into``. Merging away the owner entity ``e_user`` is refused (400)."""
        return _memory_entity(
            self._json(
                "POST",
                _bot_path(bot_id, "memory", "entities", entity_id, "merge"),
                json={"into": into},
                params=_scope(scope),
            )
        )

    async def amerge_bot_memory_entity(
        self, bot_id: str, entity_id: str, into: str, *, scope: str | None = None
    ) -> BotMemoryEntity:
        """Async version of ``merge_bot_memory_entity``."""
        return _memory_entity(
            await self._ajson(
                "POST",
                _bot_path(bot_id, "memory", "entities", entity_id, "merge"),
                json={"into": into},
                params=_scope(scope),
            )
        )

    def forget_bot_memory_entity(
        self, bot_id: str, entity_id: str, *, scope: str | None = None
    ) -> list[BotMemoryFact]:
        """Forget an entity and every fact about it; returns those facts. Refused (400) for ``e_user``."""
        return _forgotten_facts(
            self._json("DELETE", _bot_path(bot_id, "memory", "entities", entity_id), params=_scope(scope))
        )

    async def aforget_bot_memory_entity(
        self, bot_id: str, entity_id: str, *, scope: str | None = None
    ) -> list[BotMemoryFact]:
        """Async version of ``forget_bot_memory_entity``."""
        return _forgotten_facts(
            await self._ajson("DELETE", _bot_path(bot_id, "memory", "entities", entity_id), params=_scope(scope))
        )


__all__ = [
    "UNSET",
    "Bot",
    "BotApp",
    "BotClassification",
    "BotMemoryEntity",
    "BotMemoryEvidence",
    "BotMemoryFact",
    "BotModelPolicy",
    "BotSecret",
    "BotsMixin",
]
