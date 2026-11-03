"""Wire contracts of packages/api/src/routes/bots and packages/api/src/routes/inbox."""
import json
import re
import unittest

import httpx
from flow_like import (
    Bot,
    BotModelPolicy,
    FlowLikeClient,
    InboxEntry,
    new_inbox_message_id,
    plain_envelope,
)
from flow_like._errors import APIError, NotFoundError

MESSAGE_ID = re.compile(r"^msg_[0-9a-f]{32}$")
NOW = "2026-11-02T10:00:00.000Z"

BOT = {
    "id": "bot_1",
    "owner_id": "user_1",
    "system": None,
    "preset_id": None,
    "name": "Researcher",
    "description": None,
    "instructions": None,
    "appearance": {"kind": "pip"},
    "model": {"primary": "bit_model", "fallbacks": ["bit_fallback"]},
    "classification": {"classification": "internal", "pii": True},
    "capabilities": ["internet"],
    "profile_id": None,
    "location": "server",
    "device_id": None,
    "import_key": None,
    "created_at": NOW,
    "updated_at": NOW,
}


def entry(entry_id, **overrides):
    return {
        "id": entry_id,
        "message_id": f"m_{entry_id}",
        "conversation_id": "conv",
        "sender_id": "bot_1",
        "kind": "message",
        "envelope": {"v": 1, "enc": "none", "body": {"text": entry_id}},
        "created_at": NOW,
        "cursor": f"1700000000000000:{entry_id}",
        **overrides,
    }


CONVERSATION = {
    "id": "conv",
    "kind": "direct",
    "title": None,
    "encryption": "plain",
    "created_by": "user_1",
    "created_at": NOW,
    "last_activity_at": NOW,
    "members": [
        {"principal_id": "user_1", "kind": "user", "added_by": None, "joined_at": NOW},
        {"principal_id": "bot_1", "kind": "bot", "added_by": "user_1", "joined_at": NOW},
    ],
    "unread_count": 2,
    "last_entry": entry("last"),
}


class BotsAndInbox(unittest.IsolatedAsyncioTestCase):
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
        """Replies in order; the last one repeats. ``None`` answers 204 without a body."""
        self.replies = [(204 if body is None else status, body) for body in bodies]

    def wire(self, index=-1):
        request = self.requests[index]
        return (
            request.method,
            request.url.raw_path.split(b"?")[0].decode(),
            dict(request.url.params),
            json.loads(request.content) if request.content else None,
        )

    async def both(self, name, *args, **kwargs):
        """Results of the sync method and its async twin, after checking they sent the same request."""
        first = getattr(self.client, name)(*args, **kwargs)
        sync_wire = self.wire()
        second = await getattr(self.client, "a" + name)(*args, **kwargs)
        async_wire = self.wire()
        for body in (sync_wire[3], async_wire[3]):
            if isinstance(body, dict):
                body.pop("message_id", None)
        self.assertEqual(sync_wire, async_wire)
        return first, second

    def test_requests_carry_the_pat(self):
        self.reply({"bots": []})
        self.client.list_bots()
        self.assertEqual(self.requests[-1].headers["authorization"], "pat_owner")
        self.assertNotIn("x-api-key", self.requests[-1].headers)

    async def test_list_and_get_bots_parse_typed_models(self):
        self.reply({"bots": [BOT]})
        for bots in await self.both("list_bots"):
            self.assertEqual(self.wire(), ("GET", "/api/v1/bots", {}, None))
            bot = bots[0]
            self.assertIsInstance(bot, Bot)
            self.assertEqual(bot.model, BotModelPolicy(primary="bit_model", fallbacks=["bit_fallback"]))
            self.assertEqual(bot.classification.classification, "internal")
            self.assertTrue(bot.classification.pii)
            self.assertEqual(bot.capabilities, ["internet"])
            self.assertEqual(bot.raw, BOT)
        self.reply(BOT)
        for bot in await self.both("get_bot", "bot /1"):
            self.assertEqual(self.wire()[:2], ("GET", "/api/v1/bots/bot%20%2F1"))
            self.assertEqual(bot.name, "Researcher")

    async def test_create_bot_defaults_the_look_and_sends_only_given_fields(self):
        self.reply(BOT)
        await self.both("create_bot", "Researcher", capabilities=["internet"],
                        model=BotModelPolicy(primary="bit_model"), classification={"pii": True})
        self.assertEqual(self.wire(), ("POST", "/api/v1/bots", {}, {
            "name": "Researcher",
            "appearance": {"kind": "pip"},
            "capabilities": ["internet"],
            "model": {"primary": "bit_model", "fallbacks": []},
            "classification": {"pii": True},
        }))
        await self.both("create_bot", "Desk", appearance={"kind": "robo"}, location="desktop", device_id=None)
        self.assertEqual(self.wire()[3], {"name": "Desk", "appearance": {"kind": "robo"}, "location": "desktop"})

    async def test_update_bot_sends_none_as_null_and_skips_left_out_fields(self):
        self.reply(BOT)
        await self.both("update_bot", "bot_1", description=None, capabilities=[], instructions="Be brief")
        self.assertEqual(self.wire(), ("PATCH", "/api/v1/bots/bot_1", {}, {
            "description": None,
            "capabilities": [],
            "instructions": "Be brief",
        }))
        with self.assertRaises(TypeError):
            self.client.update_bot("bot_1", descripton="typo")

    async def test_bot_apps_and_secret_routes(self):
        self.reply(None)
        for result in await self.both("delete_bot", "bot_1"):
            self.assertIsNone(result)
            self.assertEqual(self.wire(), ("DELETE", "/api/v1/bots/bot_1", {}, None))
        await self.both("attach_bot_app", "bot_1", "app/1")
        self.assertEqual(self.wire(), ("PUT", "/api/v1/bots/bot_1/apps/app%2F1", {}, None))
        await self.both("detach_bot_app", "bot_1", "app")
        self.assertEqual(self.wire(), ("DELETE", "/api/v1/bots/bot_1/apps/app", {}, None))
        await self.both("delete_bot_secret", "bot_1", "API_TOKEN")
        self.assertEqual(self.wire(), ("DELETE", "/api/v1/bots/bot_1/secrets/API_TOKEN", {}, None))

        self.reply({"apps": [{"app_id": "app", "added_at": NOW, "owner_role": None}]})
        for apps in await self.both("list_bot_apps", "bot_1"):
            self.assertEqual(self.wire()[:2], ("GET", "/api/v1/bots/bot_1/apps"))
            self.assertEqual((apps[0].app_id, apps[0].owner_role), ("app", None))

        secret = {"name": "API_TOKEN", "encryption": "server", "description": "CRM",
                  "created_at": NOW, "updated_at": NOW, "last_used_at": None}
        self.reply({"secrets": [secret]})
        for secrets in await self.both("list_bot_secrets", "bot_1"):
            self.assertEqual(self.wire()[:2], ("GET", "/api/v1/bots/bot_1/secrets"))
            self.assertEqual((secrets[0].name, secrets[0].description), ("API_TOKEN", "CRM"))

        self.reply(secret)
        for stored in await self.both("set_bot_secret", "bot_1", "API_TOKEN", "s3cr3t", description="CRM"):
            self.assertEqual(stored.encryption, "server")
            self.assertEqual(self.wire(), ("PUT", "/api/v1/bots/bot_1/secrets/API_TOKEN", {},
                                           {"value": "s3cr3t", "description": "CRM"}))
            self.assertNotIn("s3cr3t", repr(stored))
        sealed = {"v": 1, "enc": "hpke", "key_id": "k", "enc_key": "e", "ct": "c"}
        await self.both("set_bot_secret", "bot_1", "API_TOKEN", sealed=sealed, description=None)
        self.assertEqual(self.wire()[3], {"sealed": sealed, "description": None})
        await self.both("set_bot_secret", "bot_1", "API_TOKEN", "rotated")
        self.assertEqual(self.wire()[3], {"value": "rotated"})

    async def test_errors_keep_status_and_the_server_code(self):
        self.reply({"error": {"code": "BOT_LIMIT_REACHED", "message": "You have 50 bots"}}, status=409)
        with self.assertRaises(APIError) as caught:
            self.client.create_bot("One more")
        self.assertEqual(caught.exception.status_code, 409)
        self.assertEqual(json.loads(caught.exception.response_body)["error"]["code"], "BOT_LIMIT_REACHED")
        self.reply({"error": {"code": "NOT_FOUND", "message": "Bot 'x' not found"}}, status=404)
        with self.assertRaises(NotFoundError):
            await self.client.aget_bot("x")
        self.reply({"error": {"code": "FORBIDDEN", "message": "APIKey user is not allowed"}}, status=403)
        with self.assertRaises(APIError) as refused:
            await self.client.alist_conversations()
        self.assertEqual(refused.exception.status_code, 403)
        with self.assertRaises(ValueError):
            self.client.get_bot("..")

    async def test_conversations_are_read_as_yourself_or_as_your_bot(self):
        self.reply({"conversations": [CONVERSATION]})
        for conversations in await self.both("list_conversations"):
            self.assertEqual(self.wire(), ("GET", "/api/v1/inbox/conversations", {}, None))
            conversation = conversations[0]
            self.assertEqual([m.kind for m in conversation.members], ["user", "bot"])
            self.assertEqual(conversation.unread_count, 2)
            self.assertIsInstance(conversation.last_entry, InboxEntry)
            self.assertEqual(conversation.last_entry.text, "last")
        await self.both("list_conversations", as_bot="bot_1")
        self.assertEqual(self.wire()[2], {"as": "bot_1"})
        self.reply(CONVERSATION)
        for conversation in await self.both("get_conversation", "conv/1", as_bot="bot_1"):
            self.assertEqual(self.wire(), ("GET", "/api/v1/inbox/conversations/conv%2F1", {"as": "bot_1"}, None))
            self.assertEqual(conversation.id, "conv")

    async def test_conversation_management(self):
        self.reply(CONVERSATION)
        await self.both("create_conversation", "direct", ["bot_1"])
        self.assertEqual(len(self.requests), 2)
        self.assertEqual(self.wire(), ("POST", "/api/v1/inbox/conversations", {},
                                       {"kind": "direct", "members": ["bot_1"]}))
        await self.both("create_conversation", "group", ["bot_1", "user_2"], title="Launch", encryption="e2e")
        self.assertEqual(self.wire()[3], {"kind": "group", "members": ["bot_1", "user_2"],
                                          "title": "Launch", "encryption": "e2e"})
        await self.both("rename_conversation", "conv", None)
        self.assertEqual(self.wire(), ("PATCH", "/api/v1/inbox/conversations/conv", {}, {"title": None}))
        await self.both("add_conversation_members", "conv", ["bot_2"])
        self.assertEqual(self.wire(), ("POST", "/api/v1/inbox/conversations/conv/members", {},
                                       {"members": ["bot_2"]}))
        self.reply(None)
        for result in await self.both("remove_conversation_member", "conv", "bot 2"):
            self.assertIsNone(result)
            self.assertEqual(self.wire(), ("DELETE", "/api/v1/inbox/conversations/conv/members/bot%202", {}, None))

    async def test_entry_pages_pass_the_window_and_expose_next_before(self):
        self.reply({"entries": [entry("b"), entry("a")], "next_before": "1:a"})
        for page in await self.both("list_inbox_entries", "conv", as_bot="bot_1", before="9:z", after="1:0", limit=2):
            self.assertEqual(self.wire(), ("GET", "/api/v1/inbox/conversations/conv/entries",
                                           {"as": "bot_1", "before": "9:z", "after": "1:0", "limit": "2"}, None))
            self.assertEqual([item.id for item in page.entries], ["b", "a"])
            self.assertEqual(page.next_before, "1:a")
        await self.both("list_inbox_entries", "conv")
        self.assertEqual(self.wire()[2], {})

    def history_pages(self):
        self.reply(
            {"entries": [entry("e"), entry("d")], "next_before": "2:d"},
            {"entries": [entry("c"), entry("b")], "next_before": "1:b"},
            {"entries": [entry("a")], "next_before": None},
        )

    async def test_history_iterators_follow_next_before_to_the_end(self):
        expected_params = [
            {"as": "bot_1", "after": "0:start", "limit": "2"},
            {"as": "bot_1", "before": "2:d", "after": "0:start", "limit": "2"},
            {"as": "bot_1", "before": "1:b", "after": "0:start", "limit": "2"},
        ]
        self.history_pages()
        ids = [item.id for item in self.client.iter_inbox_entries("conv", as_bot="bot_1", after="0:start", page_size=2)]
        self.assertEqual(ids, ["e", "d", "c", "b", "a"])
        self.assertEqual([self.wire(i)[2] for i in range(3)], expected_params)

        self.requests.clear()
        self.history_pages()
        ids = [item.id async for item in await self.client.aiter_inbox_entries(
            "conv", as_bot="bot_1", after="0:start", page_size=2)]
        self.assertEqual(ids, ["e", "d", "c", "b", "a"])
        self.assertEqual([self.wire(i)[2] for i in range(3)], expected_params)

    def test_history_is_fetched_lazily(self):
        self.reply({"entries": [entry("b"), entry("a")], "next_before": "1:a"})
        history = self.client.iter_inbox_entries("conv", before="9:z")
        self.assertEqual(self.requests, [])
        self.assertEqual(next(history).id, "b")
        self.assertEqual(len(self.requests), 1)
        self.assertEqual(self.wire()[2], {"before": "9:z"})

    async def test_send_message_generates_ids_and_wraps_a_plain_envelope(self):
        self.reply(entry("sent"))
        for sent in await self.both("send_inbox_message", "conv", "Hello"):
            self.assertEqual(sent.text, "sent")
        method, path, params, body = self.wire()
        self.assertEqual((method, path, params), ("POST", "/api/v1/inbox/conversations/conv/entries", {}))
        self.assertRegex(body["message_id"], MESSAGE_ID)
        self.assertEqual(body["kind"], "message")
        self.assertEqual(body["envelope"], {"v": 1, "enc": "none", "body": {"text": "Hello"}})
        ids = {json.loads(request.content)["message_id"] for request in self.requests}
        self.assertEqual(len(ids), 2)
        await self.both("send_inbox_message", "conv", "Retry", reply_to="m_a", message_id="msg_fixed")
        self.assertEqual(self.wire()[3], {
            "message_id": "msg_fixed",
            "kind": "message",
            "envelope": plain_envelope({"text": "Retry", "reply_to": "m_a"}),
        })

    async def test_reactions_edits_and_deletions_name_their_target(self):
        self.reply(entry("x"))
        await self.both("react_to_inbox_message", "conv", "m_a", "👍")
        self.assertEqual(self.wire()[3]["envelope"]["body"], {"target": "m_a", "emoji": "👍", "op": "add"})
        await self.both("react_to_inbox_message", "conv", "m_a", "👍", op="remove", message_id="msg_unreact")
        self.assertEqual(self.wire()[3], {"message_id": "msg_unreact", "kind": "reaction",
                                          "envelope": plain_envelope({"target": "m_a", "emoji": "👍", "op": "remove"})})
        await self.both("edit_inbox_message", "conv", "m_a", "Fixed")
        self.assertEqual((self.wire()[3]["kind"], self.wire()[3]["envelope"]),
                         ("edit", plain_envelope({"target": "m_a", "text": "Fixed"})))
        await self.both("delete_inbox_message", "conv", "m_a")
        self.assertEqual((self.wire()[3]["kind"], self.wire()[3]["envelope"]), ("delete", plain_envelope({"target": "m_a"})))
        for request in self.requests:
            body = json.loads(request.content)
            if body["message_id"] != "msg_unreact":
                self.assertRegex(body["message_id"], MESSAGE_ID)

    async def test_sealed_envelopes_pass_through(self):
        self.reply(entry("x"))
        sealed = {"v": 1, "enc": "hpke", "key_id": "k", "enc_key": "e", "ct": "c"}
        await self.both("send_inbox_entry", "conv", "message", envelopes={"user_1": sealed, "bot_1": sealed})
        body = self.wire()[3]
        self.assertEqual(body["envelopes"], {"user_1": sealed, "bot_1": sealed})
        self.assertNotIn("envelope", body)
        self.assertRegex(body["message_id"], MESSAGE_ID)

    async def test_send_conflict_surfaces_the_server_code(self):
        self.reply({"error": {"code": "INBOX_MESSAGE_ID_TAKEN", "message": "used"}}, status=409)
        with self.assertRaises(APIError) as caught:
            await self.client.asend_inbox_message("conv", "Hi", message_id="msg_used")
        self.assertEqual(caught.exception.status_code, 409)
        self.assertIn("INBOX_MESSAGE_ID_TAKEN", caught.exception.response_body)

    async def test_mark_read_always_sends_a_json_body(self):
        self.reply({"last_read_at": NOW})
        for marker in await self.both("mark_conversation_read", "conv"):
            self.assertEqual(marker.last_read_at, NOW)
            self.assertEqual(self.wire(), ("POST", "/api/v1/inbox/conversations/conv/read", {}, {}))
            self.assertEqual(self.requests[-1].headers["content-type"], "application/json")
        await self.both("mark_conversation_read", "conv", "5:e")
        self.assertEqual(self.wire()[3], {"up_to": "5:e"})

    async def test_keys_are_fetched_registered_and_revoked(self):
        key = {"principal_id": "bot_1", "key_id": "k:1", "algorithm": "hpke-x25519-sha256-chacha20poly1305",
               "public_key": "cHVi", "created_at": NOW}
        self.reply({"keys": [key]})
        for keys in await self.both("get_inbox_keys", ["user_1", "bot_1"]):
            self.assertEqual(self.wire(), ("GET", "/api/v1/inbox/keys", {"principals": "user_1,bot_1"}, None))
            self.assertEqual((keys[0].principal_id, keys[0].key_id), ("bot_1", "k:1"))
        self.reply(key)
        for registered in await self.both("register_inbox_key", "k:1", key["algorithm"], "cHVi", principal_id="bot_1"):
            self.assertEqual(registered.public_key, "cHVi")
            self.assertEqual(self.wire(), ("PUT", "/api/v1/inbox/keys", {}, {
                "key_id": "k:1", "algorithm": key["algorithm"], "public_key": "cHVi", "principal_id": "bot_1"}))
        self.reply(None)
        await self.both("revoke_inbox_key", "k:1", principal_id="bot_1")
        self.assertEqual(self.wire(), ("DELETE", "/api/v1/inbox/keys/k%3A1", {"principal_id": "bot_1"}, None))
        await self.both("revoke_inbox_key", "k:2")
        self.assertEqual(self.wire()[1:3], ("/api/v1/inbox/keys/k%3A2", {}))


class InboxHelpers(unittest.TestCase):
    def test_message_ids_are_unique_and_valid(self):
        ids = {new_inbox_message_id() for _ in range(1000)}
        self.assertEqual(len(ids), 1000)
        for message_id in ids:
            self.assertRegex(message_id, MESSAGE_ID)

    def test_entry_body_and_text(self):
        from flow_like.inbox import _entry

        reaction = _entry(entry("r", kind="reaction",
                                envelope=plain_envelope({"target": "m_a", "emoji": "👍", "op": "add"})))
        sealed = _entry(entry("s", envelope={"v": 1, "enc": "hpke", "key_id": "k", "enc_key": "e", "ct": "c",
                                             "body": {"text": "not plain"}}))
        self.assertEqual(_entry(entry("hello")).text, "hello")
        self.assertIsNone(reaction.text)
        self.assertEqual(reaction.body["emoji"], "👍")
        self.assertIsNone(sealed.body)
        self.assertIsNone(sealed.text)


if __name__ == "__main__":
    unittest.main()
