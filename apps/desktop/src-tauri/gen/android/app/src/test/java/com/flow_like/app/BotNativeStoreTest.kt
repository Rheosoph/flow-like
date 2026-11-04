package com.flow_like.app

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.time.Instant
import java.util.UUID

class BotNativeStoreTest {
    private class Records : BotRecordStore {
        val values = mutableMapOf<String, ByteArray>()
        override fun read(key: String) = values[key]?.copyOf()
        override fun write(key: String, bytes: ByteArray) { values[key] = bytes.copyOf() }
        override fun delete(key: String) { values.remove(key) }
    }
    private val scope = "[\"https://hub.example\",\"profile\",\"alice\"]"
    private fun snapshot(owner: String = scope) = JSONObject().put("version", 1).put("scope", owner)
        .put("expiresAt", Instant.now().plusSeconds(3600).toString())

    @Test fun accountChangeClearsPersistedActionsAndAttachments() {
        val records = Records()
        val root = Files.createTempDirectory("bot-scope").toFile()
        try {
            val store = BotNativeStore(root, records)
            store.publish(snapshot())
            root.resolve("shared.txt").writeText("private")
            store.enqueue(JSONObject().put("kind", "ask_bot"), scope)
            store.publish(snapshot(scope.replace("alice", "bob")))
            assertEquals(0, store.pending().length())
            assertFalse(root.resolve("shared.txt").exists())
        } finally { root.deleteRecursively() }
    }

    @Test fun actionsSurviveRestartAndAcknowledgeOnlyTheirOwnScope() {
        val records = Records()
        val root = Files.createTempDirectory("bot-restart").toFile()
        try {
            val store = BotNativeStore(root, records)
            store.publish(snapshot())
            val id = UUID.randomUUID().toString()
            store.enqueue(JSONObject().put("kind", "ask_bot"), scope, id)
            val restarted = BotNativeStore(root, records)
            restarted.enqueue(JSONObject().put("kind", "ask_bot"), scope, id)
            assertEquals(1, restarted.pending().length())
            restarted.acknowledge(id, "wrong-scope")
            assertEquals(1, restarted.pending().length())
            restarted.acknowledge(id, scope)
            assertEquals(0, restarted.pending().length())
        } finally { root.deleteRecursively() }
    }

    @Test fun expiredNotificationAndOldAccountCannotQueueApproval() {
        val records = Records()
        val root = Files.createTempDirectory("bot-approval").toFile()
        try {
            val store = BotNativeStore(root, records)
            store.publish(snapshot())
            val id = store.rememberAction(JSONObject().put("kind", "respond_bot_approval"), scope, Instant.now().minusSeconds(1).toString())
            assertNull(store.resolveAction(id))
            store.publish(snapshot(scope.replace("alice", "bob")))
            assertThrows(IllegalArgumentException::class.java) { store.enqueue(JSONObject().put("kind", "respond_bot_approval"), scope) }
        } finally { root.deleteRecursively() }
    }

    @Test fun acknowledgementNeverDeletesFilesOutsideShareRoot() {
        val records = Records()
        val root = Files.createTempDirectory("bot-files").toFile()
        val outside = Files.createTempFile("bot-outside", ".txt").toFile()
        try {
            val store = BotNativeStore(root, records)
            store.publish(snapshot())
            val action = store.enqueue(JSONObject().put("kind", "ask_bot").put("files", org.json.JSONArray(listOf(outside.path))), scope)
            store.acknowledge(action.getString("id"), scope)
            assertTrue(outside.exists())
        } finally { root.deleteRecursively(); outside.delete() }
    }

    @Test fun expiredDeliveryBecomesForegroundFailureUntilScopedAcknowledgement() {
        val root = Files.createTempDirectory("bot-expired").toFile()
        try {
            val store = BotNativeStore(root, Records())
            store.publish(snapshot())
            val id = UUID.randomUUID().toString()
            store.enqueue(JSONObject().put("kind", "respond_bot_approval"), scope, id,
                backgroundDelivery = true, deadline = Instant.now().minusSeconds(1).toString())
            assertEquals(0, store.pending().length())
            val failed = store.foregroundActions().getJSONObject(0)
            assertEquals(id, failed.getString("id"))
            assertTrue(failed.getString("backgroundError").contains("expired"))
            store.acknowledge(id, "another-scope")
            assertEquals(1, store.foregroundActions().length())
            store.acknowledge(id, scope)
            assertEquals(0, store.foregroundActions().length())
        } finally { root.deleteRecursively() }
    }
}
