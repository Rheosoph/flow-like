package com.flow_like.app

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.util.UUID

class BotDeliveryPolicyTest {
    private val scope = "[\"https://hub.example\",\"profile\",\"alice\"]"
    private fun snapshot() = JSONObject().put("scope", scope).put("bots", JSONArray().put(JSONObject().put("id", "bot")))
    private fun reply() = JSONObject().put("id", UUID.randomUUID().toString()).put("scope", scope).put("backgroundDelivery", true)
        .put("action", JSONObject().put("kind", "ask_bot").put("botId", "bot").put("conversationId", "room").put("text", "Please continue"))

    @Test fun replyHasStableMessageIdAndMatchesForegroundEnvelope() {
        val item = reply()
        val request = BotDeliveryPolicy.request(item, snapshot())!!
        assertEquals("https://hub.example/api/v1/inbox/conversations/room/entries", request.url)
        assertEquals(item.getString("id"), request.body.getString("message_id"))
        assertEquals("bot", request.body.getJSONObject("envelope").getJSONObject("body").getJSONArray("mentions").getString(0))
    }

    @Test fun refusesCrossScopeEncryptedLocalAndFileDelivery() {
        assertNull(BotDeliveryPolicy.request(reply().put("scope", scope.replace("alice", "bob")), snapshot()))
        for (flag in listOf("encrypted", "local")) {
            val snapshot = snapshot()
            snapshot.getJSONArray("bots").getJSONObject(0).put(flag, true)
            assertNull(BotDeliveryPolicy.request(reply(), snapshot))
        }
        val item = reply()
        item.getJSONObject("action").put("files", JSONArray().put("/secret"))
        assertNull(BotDeliveryPolicy.request(item, snapshot()))
    }

    @Test fun rejectsUntrustedDestinationAndPersistedApprovalGrant() {
        for (origin in listOf("http://hub.example", "https://alice@hub.example", "https://hub.example?redirect=1")) {
            val unsafe = JSONArray().put(origin).put("profile").put("alice").toString()
            assertNull(BotDeliveryPolicy.request(reply().put("scope", unsafe), snapshot().put("scope", unsafe)))
        }
        val item = reply()
        item.getJSONObject("action").put("kind", "respond_bot_approval").put("turnId", "turn").put("approvalId", "approval").put("decision", "always")
        assertNull(BotDeliveryPolicy.request(item, snapshot()))
        item.getJSONObject("action").put("decision", "once")
        assertEquals("https://hub.example/api/v1/bots/bot/turns/turn/approval", BotDeliveryPolicy.request(item, snapshot())!!.url)
    }

    @Test fun credentialMustMatchSubjectAndRemainUnexpired() {
        val key = "native-auth:v1:" + "a".repeat(64)
        val binding = "credential:native-auth-binding:[\"profile\",\"https://hub.example\"]"
        val user = JSONObject().put("profile", JSONObject().put("sub", "alice")).put("expires_at", 2000).put("access_token", "opaque-token")
        val records = object : BotRecordStore {
            override fun read(name: String): ByteArray? = when (name) {
                binding -> key.toByteArray()
                "credential:$key" -> JSONObject().put("user", user).toString().toByteArray()
                else -> null
            }
            override fun write(key: String, bytes: ByteArray) = Unit
            override fun delete(key: String) = Unit
        }
        assertEquals("opaque-token", BotDeliveryPolicy.accessToken(scope, records, 1000))
        assertNull(BotDeliveryPolicy.accessToken(scope, records, 1980))
        user.getJSONObject("profile").put("sub", "bob")
        assertNull(BotDeliveryPolicy.accessToken(scope, records, 1000))
    }
}
