"""Wire contracts of packages/api/src/routes/bots/memory.rs."""
import json
import unittest

import httpx
from flow_like import BotMemoryEntity, BotMemoryFact, FlowLikeClient
from flow_like._errors import APIError, NotFoundError

NOW = "2026-11-02T10:00:00.000Z"

FACT = {
    "id": "f_1",
    "text": "Anna lives in Berlin",
    "kind": "fact",
    "subject": "Anna",
    "subject_id": "e_anna",
    "source": "chat",
    "confidence": 0.9,
    "recorded_at": NOW,
    "valid_from": None,
    "valid_to": None,
    "superseded_by": None,
    "evidence": [
        {"seq": 3, "message_id": "msg_1", "quote": "I live in Berlin", "at": NOW,
         "speaker": {"id": "user_1", "name": "Anna"}},
    ],
    "conversation_scope": "conv_1",
}

ENTITY = {
    "id": "e_anna",
    "name": "Anna",
    "kind": "person",
    "owner": False,
    "aliases": ["Ann"],
    "fact_count": 2,
    "created_at": NOW,
    "updated_at": NOW,
}


class BotMemory(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.requests = []
        self.replies = []
        self.client = FlowLikeClient(base_url="https://hub.example", pat="pat_owner")
        transport = httpx.MockTransport(self.handle)
        self.client._client = httpx.Client(base_url=self.client._api_base, transport=transport,
                                          headers=self.client._auth_headers)
        self.client._async_client = httpx.AsyncClient(base_url=self.client._api_base, transport=transport,
                                                      headers=self.client._auth_headers)

    async def asyncTearDown(self):
        await self.client.aclose()

    def handle(self, request):
        self.requests.append(request)
        status, body = self.replies.pop(0) if len(self.replies) > 1 else self.replies[0]
        return httpx.Response(status) if body is None else httpx.Response(status, json=body)

    def reply(self, *bodies, status=200):
        self.replies = [(204 if body is None else status, body) for body in bodies]

    def wire(self, index=-1):
        request = self.requests[index]
        return (
            request.method,
            request.url.raw_path.split(b"?")[0].decode(),
            dict(request.url.params),
            json.loads(request.content) if request.content else None,
        )

    async def both(self, method, *args, **kwargs):
        first = getattr(self.client, method)(*args, **kwargs)
        sync_wire = self.wire()
        second = await getattr(self.client, "a" + method)(*args, **kwargs)
        self.assertEqual(sync_wire, self.wire())
        return first, second

    async def test_list_facts_parses_models_and_sends_the_flag_only_when_asked(self):
        self.reply({"facts": [FACT]})
        for facts in await self.both("list_bot_memory_facts", "bot_1"):
            self.assertEqual(self.wire(), ("GET", "/api/v1/bots/bot_1/memory/facts", {}, None))
            fact = facts[0]
            self.assertIsInstance(fact, BotMemoryFact)
            self.assertEqual(fact.evidence[0].quote, "I live in Berlin")
            self.assertEqual(fact.evidence[0].speaker, {"id": "user_1", "name": "Anna"})
            self.assertEqual(fact.conversation_scope, "conv_1")
            self.assertIsNone(fact.superseded_by)
            self.assertEqual(fact.raw, FACT)
        await self.both("list_bot_memory_facts", "bot_1", include_superseded=True)
        self.assertEqual(self.wire()[2], {"include_superseded": "true"})

    async def test_forget_fact_escapes_ids_and_returns_the_fact(self):
        self.reply(FACT)
        for fact in await self.both("forget_bot_memory_fact", "bot/1", "f 1"):
            self.assertEqual(self.wire()[:2], ("DELETE", "/api/v1/bots/bot%2F1/memory/facts/f%201"))
            self.assertEqual(fact.id, "f_1")

    async def test_reset_deletes_the_whole_memory(self):
        self.reply({"status": "ok"})
        await self.both("reset_bot_memory", "bot_1")
        self.assertEqual(self.wire(), ("DELETE", "/api/v1/bots/bot_1/memory", {}, None))

    async def test_entities_are_listed_updated_merged_and_forgotten(self):
        self.reply({"entities": [ENTITY]})
        for entities in await self.both("list_bot_memory_entities", "bot_1"):
            self.assertEqual(self.wire()[:2], ("GET", "/api/v1/bots/bot_1/memory/entities"))
            self.assertIsInstance(entities[0], BotMemoryEntity)
            self.assertEqual(entities[0].aliases, ["Ann"])
            self.assertEqual(entities[0].fact_count, 2)
        self.reply(ENTITY)
        await self.both("update_bot_memory_entity", "bot_1", "e_anna", name="Anna B.")
        self.assertEqual(self.wire(),
                         ("PATCH", "/api/v1/bots/bot_1/memory/entities/e_anna", {}, {"name": "Anna B."}))
        await self.both("update_bot_memory_entity", "bot_1", "e_anna", kind="organization", scope="room/1 a&b")
        self.assertEqual(self.wire(), ("PATCH", "/api/v1/bots/bot_1/memory/entities/e_anna",
                                       {"scope": "room/1 a&b"}, {"kind": "organization"}))
        await self.both("merge_bot_memory_entity", "bot_1", "e_ann", "e_anna", scope="conv_1")
        self.assertEqual(self.wire(), ("POST", "/api/v1/bots/bot_1/memory/entities/e_ann/merge",
                                       {"scope": "conv_1"}, {"into": "e_anna"}))
        self.reply({"forgotten": [FACT]})
        for forgotten in await self.both("forget_bot_memory_entity", "bot_1", "e_anna", scope="conv_1"):
            self.assertEqual(self.wire(), ("DELETE", "/api/v1/bots/bot_1/memory/entities/e_anna",
                                           {"scope": "conv_1"}, None))
            self.assertEqual([fact.id for fact in forgotten], ["f_1"])

    async def test_busy_refused_and_missing_surface_as_errors(self):
        self.reply({"error": {"code": "MEMORY_BUSY", "message": "Memory is busy"}}, status=409)
        with self.assertRaises(APIError) as busy:
            self.client.reset_bot_memory("bot_1")
        self.assertEqual(busy.exception.status_code, 409)
        with self.assertRaises(APIError) as busy_async:
            await self.client.areset_bot_memory("bot_1")
        self.assertEqual(busy_async.exception.status_code, 409)
        self.reply({"error": {"code": "BAD_REQUEST", "message": "Owner entity"}}, status=400)
        with self.assertRaises(APIError) as refused:
            self.client.merge_bot_memory_entity("bot_1", "e_user", "e_anna")
        self.assertEqual(refused.exception.status_code, 400)
        self.reply({"error": {"code": "NOT_FOUND", "message": "No such entity"}}, status=404)
        with self.assertRaises(NotFoundError):
            self.client.forget_bot_memory_entity("bot_1", "e_nobody")

    def test_ids_that_would_change_the_route_are_refused_before_sending(self):
        self.reply({"facts": []})
        with self.assertRaises(ValueError):
            self.client.forget_bot_memory_fact("bot_1", "..")
        self.assertEqual(self.requests, [])


if __name__ == "__main__":
    unittest.main()
