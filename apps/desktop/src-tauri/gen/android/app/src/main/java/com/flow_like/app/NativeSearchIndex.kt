package com.flow_like.app

import android.app.appsearch.AppSearchManager
import android.app.appsearch.AppSearchSchema
import android.app.appsearch.AppSearchSession
import android.app.appsearch.BatchResultCallback
import android.app.appsearch.AppSearchBatchResult
import android.app.appsearch.GenericDocument
import android.app.appsearch.PutDocumentsRequest
import android.app.appsearch.SearchSpec
import android.app.appsearch.SetSchemaRequest
import android.content.Context
import android.os.Build
import androidx.annotation.RequiresApi
import org.json.JSONObject
import java.time.Instant
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/** Index titles and scoped launch links; messages, instructions and secrets stay out of system search. */
internal object NativeSearchIndex {
    private val worker = Executors.newSingleThreadExecutor()
    private val callbacks = Executor { it.run() }
    fun refresh(context: Context) {
        if (Build.VERSION.SDK_INT < 31) return
        val application = context.applicationContext
        worker.execute {
            try { replace(application, BotNativeStore(application).snapshot()) } catch (_: Exception) { }
        }
    }
    fun clear(context: Context) {
        if (Build.VERSION.SDK_INT < 31) return
        val application = context.applicationContext
        worker.execute { try { replace(application, null) } catch (_: Exception) { } }
    }
    @RequiresApi(31)
    private fun replace(context: Context, snapshot: JSONObject?) {
        val manager = context.getSystemService(AppSearchManager::class.java) ?: return
        var session: AppSearchSession? = null
        val ready = CountDownLatch(1)
        manager.createSearchSession(AppSearchManager.SearchContext.Builder("flow-like-native").build(), callbacks) { result ->
            if (result.isSuccess) session = result.resultValue
            ready.countDown()
        }
        if (!ready.await(10, TimeUnit.SECONDS)) return
        val index = session ?: return
        try {
            val schema = AppSearchSchema.Builder("builtin:Thing")
                .addProperty(AppSearchSchema.StringPropertyConfig.Builder("name")
                    .setCardinality(AppSearchSchema.PropertyConfig.CARDINALITY_OPTIONAL)
                    .setIndexingType(AppSearchSchema.StringPropertyConfig.INDEXING_TYPE_PREFIXES)
                    .setTokenizerType(AppSearchSchema.StringPropertyConfig.TOKENIZER_TYPE_PLAIN).build())
                .addProperty(AppSearchSchema.StringPropertyConfig.Builder("description")
                    .setCardinality(AppSearchSchema.PropertyConfig.CARDINALITY_OPTIONAL)
                    .setIndexingType(AppSearchSchema.StringPropertyConfig.INDEXING_TYPE_PREFIXES)
                    .setTokenizerType(AppSearchSchema.StringPropertyConfig.TOKENIZER_TYPE_PLAIN).build())
                .addProperty(AppSearchSchema.StringPropertyConfig.Builder("url")
                    .setCardinality(AppSearchSchema.PropertyConfig.CARDINALITY_OPTIONAL).build()).build()
            val schemaReady = CountDownLatch(1); var schemaOk = false
            index.setSchema(SetSchemaRequest.Builder().addSchemas(schema).setForceOverride(true)
                .setSchemaTypeDisplayedBySystem("builtin:Thing", true).build(), callbacks, callbacks) { result ->
                schemaOk = result.isSuccess; schemaReady.countDown()
            }
            if (!schemaReady.await(10, TimeUnit.SECONDS) || !schemaOk) return
            val removed = CountDownLatch(1); var removeOk = false
            index.remove("", SearchSpec.Builder().addFilterNamespaces("flow-like").build(), callbacks) { result ->
                removeOk = result.isSuccess; removed.countDown()
            }
            if (!removed.await(10, TimeUnit.SECONDS) || !removeOk || snapshot == null) return
            val scope = snapshot.getString("scope")
            if (BotNativeStore(context).snapshot()?.optString("scope") != scope) return
            val ttl = (Instant.parse(snapshot.getString("expiresAt")).toEpochMilli() - System.currentTimeMillis()).coerceAtLeast(0)
            if (ttl == 0L) return
            val documents = NativeSurfacePolicy.choices(snapshot, true).take(600).map { choice ->
                val key = NativeSurfacePolicy.key(scope, choice.kind, choice.id)
                GenericDocument.Builder<GenericDocument.Builder<*>>("flow-like", key, "builtin:Thing")
                    .setPropertyString("name", choice.title.take(512))
                    .setPropertyString("description", "${choice.kind} in Flow-Like")
                    .setPropertyString("url", "flow-like://native-search?key=$key")
                    .setTtlMillis(ttl).build()
            }
            if (documents.isEmpty()) return
            val complete = CountDownLatch(1)
            index.put(PutDocumentsRequest.Builder().addGenericDocuments(documents).build(), callbacks,
                object : BatchResultCallback<String, Void> {
                    override fun onResult(result: AppSearchBatchResult<String, Void>) { complete.countDown() }
                    override fun onSystemError(error: Throwable?) { complete.countDown() }
                })
            complete.await(10, TimeUnit.SECONDS)
            // A logout while indexing must remove the completed batch before later operations.
            if (BotNativeStore(context).snapshot()?.optString("scope") != scope) {
                val cleared = CountDownLatch(1)
                index.remove("", SearchSpec.Builder().addFilterNamespaces("flow-like").build(), callbacks) { cleared.countDown() }
                cleared.await(10, TimeUnit.SECONDS)
            }
        } finally { index.close() }
    }
}
