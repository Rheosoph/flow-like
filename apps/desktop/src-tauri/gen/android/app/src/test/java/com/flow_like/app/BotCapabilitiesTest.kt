package com.flow_like.app

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class BotCapabilitiesTest {
    @Test fun disabledModelBuildStillExposesNativeCaptureContract() {
        val capabilities = BotCapabilities.create(true, false,
            JSONObject().put("available", false).put("reason", "not_in_build"), false)
        assertTrue(capabilities.getBoolean("voice"))
        assertTrue(capabilities.getBoolean("document"))
        assertTrue(capabilities.getBoolean("photo"))
        assertEquals("picker", capabilities.getString("documentMode"))
        assertFalse(capabilities.getBoolean("localAiAvailable"))
        assertEquals("not_in_build", capabilities.getString("localAiReason"))
    }

    @Test fun downloadableSummaryAndIndependentExtractionRemainReachable() {
        val downloadable = BotCapabilities.create(false, true,
            JSONObject().put("downloadable", true).put("extraction", true), true)
        assertTrue(downloadable.getBoolean("localSummarizationAvailable"))
        assertTrue(downloadable.getBoolean("localExtractionAvailable"))
        val extractionOnly = BotCapabilities.create(false, true,
            JSONObject().put("available", false).put("extraction", true), true)
        assertTrue(extractionOnly.getBoolean("localAiAvailable"))
        assertFalse(extractionOnly.getBoolean("localSummarizationAvailable"))
        assertTrue(extractionOnly.getBoolean("localExtractionAvailable"))
    }
}
