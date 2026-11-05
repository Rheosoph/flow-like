package com.flow_like.app

import android.content.Context
import org.json.JSONObject

internal object BotLocalAi {
    fun availability(context: Context): JSONObject = JSONObject().put("available", false).put("reason", "not_in_build")
    fun process(context: Context, options: JSONObject): JSONObject = availability(context)
}
