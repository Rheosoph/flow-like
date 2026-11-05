package com.flow_like.app

import android.content.Context
import com.google.android.gms.tasks.Tasks
import com.google.mlkit.genai.common.FeatureStatus
import com.google.mlkit.genai.summarization.Summarization
import com.google.mlkit.genai.summarization.SummarizationRequest
import com.google.mlkit.genai.summarization.SummarizerOptions
import com.google.mlkit.nl.entityextraction.EntityExtraction
import com.google.mlkit.nl.entityextraction.EntityExtractorOptions
import com.google.mlkit.nl.entityextraction.EntityExtractionRemoteModel
import com.google.mlkit.common.model.RemoteModelManager
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

internal object BotLocalAi {
    private fun summarizer(context: Context) = Summarization.getClient(SummarizerOptions.builder(context)
        .setInputType(SummarizerOptions.InputType.ARTICLE)
        .setOutputType(SummarizerOptions.OutputType.THREE_BULLETS)
        .setLanguage(SummarizerOptions.Language.ENGLISH).build())

    fun availability(context: Context): JSONObject {
        val model = try { summarizer(context) } catch (_: Exception) {
            return JSONObject().put("available", false).put("reason", "model_unavailable").put("extraction", true)
        }
        return try {
            val status = model.checkFeatureStatus().get(10, TimeUnit.SECONDS)
            JSONObject().put("available", status == FeatureStatus.AVAILABLE)
                .put("downloadable", status == FeatureStatus.DOWNLOADABLE).put("language", "en")
                .put("status", status).put("extraction", true)
        } catch (_: Exception) {
            JSONObject().put("available", false).put("reason", "model_unavailable").put("extraction", true)
        } finally { model.close() }
    }

    fun process(context: Context, options: JSONObject): JSONObject {
        val text = options.getString("text")
        require(text.isNotBlank() && text.length <= 16000) { "Use at most 16000 characters" }
        if (options.getString("operation") == "extract") {
            val extractor = EntityExtraction.getClient(EntityExtractorOptions.Builder(EntityExtractorOptions.ENGLISH).build())
            try {
                val model = EntityExtractionRemoteModel.Builder(EntityExtractorOptions.ENGLISH).build()
                val downloaded = Tasks.await(RemoteModelManager.getInstance().isModelDownloaded(model), 10, TimeUnit.SECONDS)
                if (!downloaded && !options.optBoolean("downloadModel")) return JSONObject().put("available", false).put("downloadable", true)
                if (!downloaded) Tasks.await(extractor.downloadModelIfNeeded(), 5, TimeUnit.MINUTES)
                val annotations = Tasks.await(extractor.annotate(text), 30, TimeUnit.SECONDS)
                val entities = JSONArray()
                annotations.forEach { annotation ->
                    entities.put(JSONObject().put("text", text.substring(annotation.start, annotation.end))
                        .put("start", annotation.start).put("end", annotation.end)
                        .put("types", JSONArray(annotation.entities.map { it.type })))
                }
                return JSONObject().put("available", true).put("text", entities.toString(2)).put("entities", entities)
            } finally { extractor.close() }
        }
        require(text.length >= 400) { "Article summaries require at least 400 characters" }
        val model = summarizer(context)
        try {
            val status = model.checkFeatureStatus().get(10, TimeUnit.SECONDS)
            if (status != FeatureStatus.AVAILABLE && !(status == FeatureStatus.DOWNLOADABLE && options.optBoolean("downloadModel"))) {
                return JSONObject().put("available", false).put("downloadable", status == FeatureStatus.DOWNLOADABLE)
            }
            val result = model.runInference(SummarizationRequest.builder(text).build()).get(5, TimeUnit.MINUTES)
            return JSONObject().put("available", true).put("text", result.summary)
        } finally { model.close() }
    }
}
