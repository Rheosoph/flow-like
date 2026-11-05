package com.flow_like.app

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.time.Instant
import java.net.URLEncoder

class NativeSurfacePolicyTest {
    private fun snapshot() = JSONObject().put("scope", "workspace-a").put("webOrigin", "https://app.example")
        .put("apps", JSONArray().put(JSONObject().put("id", "app").put("title", "Orders").put("spotlightEligible", true)))
        .put("bots", JSONArray().put(JSONObject().put("id", "bot").put("title", "Assistant").put("surfaces", JSONArray().put("spotlight"))))

    @Test fun searchKeysCannotResolveAnotherAccountOrEntityKind() {
        assertNotEquals(NativeSurfacePolicy.key("alice", "bot", "same"), NativeSurfacePolicy.key("bob", "bot", "same"))
        assertNotEquals(NativeSurfacePolicy.key("alice", "app", "same"), NativeSurfacePolicy.key("alice", "bot", "same"))
        assertEquals(64, NativeSurfacePolicy.key("alice", "bot", "same").length)
    }

    @Test fun continuationRequiresTheSameHttpsOriginAndAnAppOrBotRoute() {
        val state = snapshot()
        assertTrue(NativeSurfacePolicy.continuation(state, "https://app.example/use/app/orders?status=open"))
        assertTrue(NativeSurfacePolicy.continuation(state, "https://app.example/use?id=app&eventId=chat"))
        assertTrue(NativeSurfacePolicy.continuation(state, "https://app.example/bots?botId=bot&conversationId=thread"))
        for (url in listOf("http://app.example/bots", "https://evil.example/bots", "https://app.example:8443/bots",
            "https://user@app.example/bots", "https://app.example/settings", "https://app.example/use/../settings",
            "https://app.example/use/%2e%2e/settings", "https://app.example/use/app%2f..%2fsettings", "https://app.example/bots#token")) {
            assertFalse(url, NativeSurfacePolicy.continuation(state, url))
        }
    }

    @Test fun appPathsStayInsideTheApp() {
        assertTrue(NativeSurfacePolicy.validPath("/orders?status=open"))
        for (path in listOf("https://evil.example", "//evil.example", "/../admin", "/%2e%2e/admin", "/orders#secret", "/orders\\admin", "/%5cadmin", "/orders%00"))
            assertFalse(path, NativeSurfacePolicy.validPath(path))
    }

    @Test fun continuationOuterLinksRejectDuplicateParametersAndAmbiguousAuthorities() {
        val state = snapshot()
        val target = "https://app.example/bots?botId=bot&conversationId=thread"
        val query = "scope=workspace-a&url=" + URLEncoder.encode(target, "UTF-8")
        assertEquals(target, NativeSurfacePolicy.continuationRequest(state, "flow-like://continue?$query"))
        assertEquals(target, NativeSurfacePolicy.continuationRequest(state, "flow-like://continue/?$query"))
        for (link in listOf("flow-like://continue?$query&scope=workspace-a", "flow-like://continue?$query&%75rl=other",
            "flow-like://continue?$query#fragment", "flow-like://user@continue?$query", "flow-like://continue:42?$query",
            "flow-like://continue/extra?$query", "flow-like://continue?$query&%73cope=other", "https://continue?$query")) {
            assertNull(link, NativeSurfacePolicy.continuationRequest(state, link))
        }
        state.put("scope", "other")
        assertNull(NativeSurfacePolicy.continuationRequest(state, "flow-like://continue?$query"))
    }

    @Test fun taskLaunchUsesTheCurrentExactConversationAndRejectsExpiredRows() {
        val state = snapshot()
        val section = JSONObject().put("kind", "bot_tasks").put("expiresAt", Instant.now().plusSeconds(60).toString())
            .put("items", JSONArray().put(JSONObject().put("id", "task").put("action", JSONObject()
                .put("kind", "open_conversation").put("botId", "bot").put("conversationId", "exact-thread"))))
        state.put("sections", JSONArray().put(section))
        val choice = NativeSurfaceChoice("bot_tasks", title = "Tasks")
        assertEquals("exact-thread", NativeSurfacePolicy.action(state, choice, "task")?.getString("conversationId"))
        assertNull(NativeSurfacePolicy.action(state, choice, "missing"))
        section.put("expiresAt", Instant.now().minusSeconds(1).toString())
        assertNull(NativeSurfacePolicy.action(state, choice, "task"))
    }

    @Test fun searchHonorsPublishedEligibilityAndWidgetsRequireCurrentApps() {
        val state = snapshot()
        state.getJSONArray("apps").put(JSONObject().put("id", "private").put("title", "Private"))
        state.put("events", JSONArray().put(JSONObject().put("id", "event").put("title", "Launch")
            .put("surfaces", JSONArray().put("widget"))))
        state.put("customWidgets", JSONArray().put(JSONObject().put("id", "chart").put("kind", "chart")
            .put("title", "Revenue").put("appId", "app").put("expiresAt", Instant.now().plusSeconds(60).toString())))
        val search = NativeSurfacePolicy.choices(state, true)
        assertEquals(setOf("app", "bot"), search.map { it.id }.toSet())
        assertTrue(NativeSurfacePolicy.choices(state).any { it.id == "chart" })
        assertTrue(NativeSurfacePolicy.choices(state).any { it.id == "event" })
        for (index in 2 until 200) state.getJSONArray("apps").put(JSONObject().put("id", "app-$index").put("title", "App $index"))
        assertTrue("Large app catalogs retain saved widgets", NativeSurfacePolicy.choices(state).any { it.id == "chart" })
        state.put("apps", JSONArray())
        assertNull(NativeSurfacePolicy.custom(state, "chart"))
        assertNull(NativeSurfacePolicy.action(state, NativeSurfaceChoice("app", "app", "Orders")))
    }

    @Test fun chartScalesPreserveNegativeValuesAndSignedStacks() {
        val points = listOf(4.0, 6.0, -3.0, -2.0).map { JSONObject().put("label", "January").put("value", it) }
        val grouped = NativeChartDomain.calculate(points, false)
        assertEquals(-3.0, grouped.minimum, 0.0)
        assertEquals(6.0, grouped.maximum, 0.0)
        val stacked = NativeChartDomain.calculate(points, true)
        assertEquals(-5.0, stacked.minimum, 0.0)
        assertEquals(10.0, stacked.maximum, 0.0)
        assertTrue(stacked.fraction(-3.0) < stacked.fraction(0.0))
        assertTrue(stacked.fraction(4.0) > stacked.fraction(0.0))
        assertTrue(NativeChartDomain.calculate(emptyList(), false).fraction(0.0).isFinite())
    }

    @Test fun widgetInstancesKeepSeparateEncryptedConfigurationsAndDeleteOnlyTheirOwnIds() {
        val records = object : BotRecordStore {
            val values = mutableMapOf<String, ByteArray>()
            override fun read(key: String) = values[key]?.copyOf()
            override fun write(key: String, bytes: ByteArray) { values[key] = bytes.copyOf() }
            override fun delete(key: String) { values.remove(key) }
        }
        val store = NativeSurfaceStore(records)
        store.put(1, "alice", NativeSurfaceChoice("app", "orders", "Orders"), "/open")
        store.put(2, "bob", NativeSurfaceChoice("bot", "assistant", "Assistant"))
        assertEquals("/open", store.get(1)?.getString("path"))
        assertEquals("bob", NativeSurfaceStore(records).get(2)?.getString("scope"))
        store.delete(intArrayOf(1))
        assertNull(store.get(1))
        assertNotNull(store.get(2))
        store.clear()
        assertNull(store.get(2))
    }
}
