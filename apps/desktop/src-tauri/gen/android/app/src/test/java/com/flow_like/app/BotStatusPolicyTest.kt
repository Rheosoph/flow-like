package com.flow_like.app

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.time.Instant

class BotStatusPolicyTest {
    @Test fun statusRefreshReplacesLiveRowsAndPreservesOtherFreshness() {
        val snapshot = JSONObject("""{"scope":"[\"https://hub.example\",\"profile\",\"alice\"]",
          "generatedAt":"2026-11-01T00:00:00Z","expiresAt":"2026-12-01T00:00:00Z",
          "bots":[{"id":"bot","title":"My bot"},{"id":"local","title":"Local","local":true}],
          "sections":[{"kind":"bot_tasks","state":"ready","items":[
            {"id":"finished","live":true,"action":{"botId":"bot"}},
            {"id":"local-run","live":true,"action":{"botId":"local"}},
            {"id":"schedule","status":"scheduled"}]},
            {"kind":"bot_approvals","updatedAt":"2026-11-01T00:00:00Z","items":[]}]}""")
        val turns = JSONArray("""[
          {"id":"run","bot_id":"bot","conversation_id":"chat","owner_id":"alice","status":"running"},
          {"id":"foreign","bot_id":"bot","conversation_id":"chat","owner_id":"bob","status":"running"},
          {"id":"finished","bot_id":"bot","conversation_id":"chat","owner_id":"alice","status":"done"}]""")
        val merged = BotStatusPolicy.merge(snapshot, turns, Instant.parse("2026-11-04T00:00:00Z"))
        assertEquals(snapshot.getString("generatedAt"), merged.getString("generatedAt"))
        assertEquals(snapshot.getString("expiresAt"), merged.getString("expiresAt"))
        val sections = merged.getJSONArray("sections")
        assertEquals("2026-11-01T00:00:00Z", sections.getJSONObject(1).getString("updatedAt"))
        val tasks = sections.getJSONObject(0)
        assertEquals("2026-11-04T00:30:00Z", tasks.getString("staleAt"))
        val items = tasks.getJSONArray("items")
        assertEquals(listOf("run"), (0 until items.length()).map { items.getJSONObject(it).getString("id") })
        assertFalse(items.getJSONObject(0).has("progress"))
        val completed = BotStatusPolicy.merge(merged, JSONArray(), Instant.parse("2026-11-04T00:15:00Z"))
        assertEquals(0, completed.getJSONArray("sections").getJSONObject(0).getJSONArray("items").length())
        snapshot.put("expiresAt", "2026-11-04T00:10:00Z")
        val nearExpiry = BotStatusPolicy.merge(snapshot, turns, Instant.parse("2026-11-04T00:00:00Z"))
            .getJSONArray("sections").getJSONObject(0)
        assertEquals("2026-11-04T00:10:00Z", nearExpiry.getString("expiresAt"))
        assertEquals("2026-11-04T00:10:00Z", nearExpiry.getString("staleAt"))
    }
}
