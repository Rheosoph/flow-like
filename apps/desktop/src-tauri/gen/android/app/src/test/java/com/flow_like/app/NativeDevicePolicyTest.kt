package com.flow_like.app

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class NativeDevicePolicyTest {
    private fun region() = JSONObject().put("id", "a".repeat(64)).put("appId", "app").put("eventId", "event")
        .put("latitude", 52.5).put("longitude", 13.4).put("radiusMeters", 200).put("transition", "both").put("background", true)
    private fun config(value: JSONObject = region()) = JSONObject().put("scope", "hub:alice").put("registrations", JSONArray().put(value))

    @Test fun registrationRejectsCrossAccountInvalidGeometryAndDuplicateRegions() {
        NativeGeofencePolicy.validate(config(), "hub:alice")
        for (value in listOf(
            config().put("scope", "hub:bob"), config(region().put("latitude", 91)),
            config(region().put("longitude", -181)), config(region().put("radiusMeters", 99)),
            config(region().put("radiusMeters", 100001)), config(region().put("transition", "dwell")),
            config(region().put("background", "true")), config().put("registrations", JSONArray().put(region()).put(region())),
        )) assertThrows(IllegalArgumentException::class.java) { NativeGeofencePolicy.validate(value, "hub:alice") }
    }

    @Test fun crossingMatchesRustGeometryIdentityAndTimestamps() {
        val region = region()
        val event = NativeGeofencePolicy.crossing("hub:alice", region, "enter", 1234567)
        assertNotNull(java.util.UUID.fromString(event.getString("id")))
        assertEquals(region.getString("id"), event.getString("registrationId"))
        assertEquals("hub:alice", event.getString("scope"))
        assertEquals("app", event.getString("appId"))
        assertEquals("event", event.getString("eventId"))
        assertEquals(1234567, event.getLong("occurredAt"))
        assertEquals(13.4, event.getJSONObject("geometry").getJSONArray("coordinates").getDouble(0), 0.0)
        assertEquals(52.5, event.getJSONObject("geometry").getJSONArray("coordinates").getDouble(1), 0.0)
        assertEquals(200.0, event.getDouble("radiusMeters"), 0.0)
        assertEquals(840000L, NativeGeofencePolicy.REMOTE_RETENTION)
    }

    @Test fun hubTemporaryFailuresRetainGeofencesWhileRefusalsDisableThem() {
        for (status in listOf(402, 408, 409, 425, 429, 500, 503)) assertFalse(NativeGeofencePolicy.terminal(status, "PROJECT_INACTIVE"))
        for (status in listOf(400, 401, 403, 404, 410, 422)) assertTrue(NativeGeofencePolicy.terminal(status, null))
        assertTrue(NativeGeofencePolicy.terminal(409, "EVENT_PAUSED"))
        assertFalse(NativeGeofencePolicy.terminal(200, null))
    }

    @Test fun privateClipboardRequiresSameAccountAndUnexpiredDeadline() {
        val item = JSONObject().put("scope", "hub:alice").put("expiresAt", 1000)
        assertTrue(NativeClipboardPolicy.readable(item, "hub:alice", 999))
        assertFalse(NativeClipboardPolicy.readable(item, "hub:alice", 1000))
        assertFalse(NativeClipboardPolicy.readable(item, "hub:bob", 999))
        assertFalse(NativeClipboardPolicy.readable(item, null, 999))
    }

    @Test fun clipboardPrivacyOptionsAndDeadlinesAreStrictlyTyped() {
        val valid = JSONObject().put("format", "text").put("text", "private").put("appPrivate", true).put("localOnly", true).put("expiresAt", 2000)
        NativeClipboardPolicy.validate(valid, 1000)
        for (value in listOf(
            JSONObject(valid.toString()).put("appPrivate", "true"), JSONObject(valid.toString()).put("localOnly", 1),
            JSONObject(valid.toString()).put("expiresAt", "2000"), JSONObject(valid.toString()).put("expiresAt", 1000),
            JSONObject(valid.toString()).put("requestDeadline", 999), JSONObject(valid.toString()).put("format", "png"),
        )) assertThrows(IllegalArgumentException::class.java) { NativeClipboardPolicy.validate(value, 1000) }
    }
}
