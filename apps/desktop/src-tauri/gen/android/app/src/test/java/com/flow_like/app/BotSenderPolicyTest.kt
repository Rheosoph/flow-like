package com.flow_like.app

import org.junit.Assert.*
import org.junit.Test

class BotSenderPolicyTest {
    @Test fun titleThatRepeatsTheBotIsDropped() {
        assertEquals("The webhook fired", BotSenderPolicy.messageText("Joerg", "The webhook fired", "Joerg"))
        assertEquals("Joerg needs your approval\nSend the invoice", BotSenderPolicy.messageText("Joerg needs your approval", "Send the invoice", "Joerg"))
        assertEquals("Build finished", BotSenderPolicy.messageText("Build finished", "", "Joerg"))
        assertEquals("", BotSenderPolicy.messageText(null, null, "Joerg"))
    }

    @Test fun avatarsComeOnlyFromPublicHttpsLinks() {
        assertEquals("cdn.example.com", BotSenderPolicy.avatarUrl("https://cdn.example.com/bots/bot_1/look.png?X-Amz-Signature=abc")?.host)
        for (value in listOf(null, "", "http://cdn.example.com/a.png", "https://user:pass@cdn.example.com/a.png", "file:///sdcard/a.png", "not a url")) {
            assertNull(value, BotSenderPolicy.avatarUrl(value))
        }
    }

    @Test fun cacheNamesNeverLeaveTheAvatarDirectory() {
        assertEquals("bot_clx1-abc.png", BotSenderPolicy.cacheName("bot_clx1-abc"))
        for (id in listOf("", "../secret", "bot/1", "bot 1", "a".repeat(129))) assertNull(id, BotSenderPolicy.cacheName(id))
    }
}
