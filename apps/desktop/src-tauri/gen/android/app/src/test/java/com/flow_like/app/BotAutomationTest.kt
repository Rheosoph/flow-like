package com.flow_like.app

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.time.Instant

class BotAutomationTest {
    private class Records : BotRecordStore {
        val values = mutableMapOf<String,ByteArray>()
        override fun read(key:String)=values[key]?.copyOf()
        override fun write(key:String,bytes:ByteArray) { values[key]=bytes.copyOf() }
        override fun delete(key:String) { values.remove(key) }
    }
    private val scope="[\"https://hub.example\",\"profile\",\"alice\"]"
    private fun snapshot()=JSONObject().put("version",1).put("scope",scope).put("expiresAt",Instant.now().plusSeconds(3600).toString())
        .put("bots",JSONArray().put(JSONObject().put("id","bot_1").put("enabled",true)))
        .put("botTasks",JSONArray().put(JSONObject().put("id","task_1").put("botId","bot_1").put("enabled",true)))
        .put("events",JSONArray().put(JSONObject().put("id","event_1").put("appId","app_1").put("eventId","event_1")
            .put("eventType","simple_chat").put("surfaces",JSONArray(listOf("shortcuts"))).put("action",JSONObject().put("kind","open_event"))))
    private fun action()=JSONObject().put("kind","ask_bot").put("botId","bot_1").put("prompt","What changed?")

    @Test fun realResponseSurvivesDeliveryAcknowledgementAndIsConsumedOnce() {
        val root=Files.createTempDirectory("automation-result").toFile()
        try {
            val store=BotNativeStore(root,Records()); store.publish(snapshot())
            val request=store.enqueueAutomation(action(),scope,"text"); val id=request.getString("id")
            assertEquals("text",request.getString("responseMode"))
            assertTrue(request.getBoolean("automation"))
            assertTrue(Instant.parse(request.getString("responseDeadline")).isBefore(Instant.now().plusSeconds(91)))
            store.acknowledge(id,scope)
            assertTrue(store.automationCurrent(id,scope))
            assertNull(store.automationResult(id,scope))
            store.complete(JSONObject().put("id",id).put("scope",scope).put("status","success").put("text","Actual answer"))
            assertFalse(store.automationCurrent(id,scope))
            assertEquals("Actual answer",store.automationResult(id,scope,true)!!.getString("text"))
            assertNull(store.automationResult(id,scope))
        } finally { root.deleteRecursively() }
    }

    @Test fun cancellationStopsPendingDeliveryAndRejectsLateSuccess() {
        val root=Files.createTempDirectory("automation-cancel").toFile()
        try {
            val store=BotNativeStore(root,Records()); store.publish(snapshot())
            val id=store.enqueueAutomation(action(),scope,"text").getString("id")
            var signal=""; BotAutomationSignals.cancelled={received,owner -> signal="$received:$owner"}
            store.cancelAutomation(id,scope)
            assertEquals("$id:$scope",signal)
            assertEquals(0,store.pending().length())
            store.complete(JSONObject().put("id",id).put("scope",scope).put("status","success").put("text","Late"))
            assertEquals("error",store.automationResult(id,scope)!!.getString("status"))
        } finally { BotAutomationSignals.cancelled=null; root.deleteRecursively() }
    }

    @Test fun profileSwitchClearsResponsesAndRejectsForeignCompletion() {
        val root=Files.createTempDirectory("automation-scope").toFile()
        try {
            val store=BotNativeStore(root,Records()); store.publish(snapshot())
            val id=store.enqueueAutomation(action(),scope,"text").getString("id")
            store.publish(snapshot().put("scope",scope.replace("alice","bob")))
            store.complete(JSONObject().put("id",id).put("scope",scope).put("status","success").put("text","Private"))
            assertNull(store.automationResult(id,scope))
            assertFalse(store.automationCurrent(id,scope))
        } finally { root.deleteRecursively() }
    }

    @Test fun automationRequiresExposedEntitiesAndDoesNotEscalateActions() {
        val snapshot=snapshot()
        BotAutomationPolicy.validate(snapshot,action(),"text")
        BotAutomationPolicy.validate(snapshot,JSONObject().put("kind","run_bot_task").put("botId","bot_1").put("taskId","task_1"),"text")
        BotAutomationPolicy.validate(snapshot,JSONObject().put("kind","open_event").put("appId","app_1").put("eventId","event_1").put("text","Question"),"text")
        for (bad in listOf(action().put("botId","foreign"),action().put("conversationId","foreign"),action().put("files",JSONArray(listOf("private.txt"))),action().put("kind","respond_bot_approval")))
            assertThrows(Exception::class.java) { BotAutomationPolicy.validate(snapshot,bad,"text") }
        snapshot.getJSONArray("events").getJSONObject(0).put("surfaces",JSONArray(listOf("widget")))
        assertThrows(Exception::class.java) { BotAutomationPolicy.validate(snapshot,JSONObject().put("kind","open_event").put("appId","app_1").put("eventId","event_1").put("text","Question"),"text") }
    }

    @Test fun expiredAutomationReturnsErrorAndCannotExecuteLater() {
        val root=Files.createTempDirectory("automation-expiry").toFile(); val records=Records()
        try {
            val store=BotNativeStore(root,records); store.publish(snapshot())
            val id=store.enqueueAutomation(action(),scope,"text").getString("id")
            val saved=JSONObject(String(records.values.getValue("native-automation")))
            saved.getJSONObject(id).put("responseDeadline",Instant.now().minusSeconds(1).toString())
            records.write("native-automation",saved.toString().toByteArray())
            assertFalse(store.automationCurrent(id,scope))
            store.complete(JSONObject().put("id",id).put("scope",scope).put("status","success").put("text","Late"))
            assertNull(store.automationResult(id,scope))
            store.cancelAutomation(id,scope,"Expired")
            assertEquals("Expired",store.automationResult(id,scope)!!.getString("error"))
        } finally { root.deleteRecursively() }
    }
}
