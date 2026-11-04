package com.flow_like.app

import org.json.JSONObject

internal object BotCapabilities {
    fun create(voice: Boolean, scanner: Boolean, localAi: JSONObject, appFunctions: Boolean): JSONObject {
        val summary = localAi.optBoolean("available") || localAi.optBoolean("downloadable")
        val extraction = localAi.optBoolean("extraction")
        return JSONObject().put("voice", voice).put("document", true).put("photo", true)
            .put("documentMode", if (scanner) "scanner" else "picker")
            .put("localAiAvailable", summary || extraction).put("localSummarizationAvailable", summary)
            .put("localExtractionAvailable", extraction).put("localAiReason", localAi.optString("reason"))
            .put("secureStorage", true).put("sharing", true).put("widgets", true).put("notifications", true)
            .put("localAi", localAi).put("appFunctions", appFunctions)
            .put("backgroundExecution", "notification_reply_and_approval")
    }
}
