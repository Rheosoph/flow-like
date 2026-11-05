package com.flow_like.app

import android.content.Context
import org.json.JSONObject

internal class NativeSurfaceStore(private val records: BotRecordStore) {
    constructor(context: Context) : this(BotSecureStore(context))
    companion object { private val lock = Any(); private const val KEY = "native-widget-configurations" }
    private fun read(): JSONObject = records.read(KEY)?.let { JSONObject(it.toString(Charsets.UTF_8)) } ?: JSONObject()
    fun get(id: Int): JSONObject? = synchronized(lock) { read().optJSONObject(id.toString()) }
    fun put(id: Int, scope: String, choice: NativeSurfaceChoice, path: String? = null) = synchronized(lock) {
        require(id > 0 && scope.length <= 8192 && (path == null || NativeSurfacePolicy.validPath(path)))
        val configs = read()
        require(configs.length() < 64 || configs.has(id.toString()))
        configs.put(id.toString(), JSONObject().put("scope", scope).put("kind", choice.kind).put("id", choice.id)
            .put("title", choice.title).put("path", path))
        records.write(KEY, configs.toString().toByteArray())
    }
    fun delete(ids: IntArray) = synchronized(lock) {
        val configs = read(); ids.forEach { configs.remove(it.toString()) }
        records.write(KEY, configs.toString().toByteArray())
    }
    fun clear() = synchronized(lock) { records.delete(KEY) }
}
