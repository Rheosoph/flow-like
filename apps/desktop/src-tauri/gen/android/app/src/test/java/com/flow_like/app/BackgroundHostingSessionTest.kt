package com.flow_like.app

import org.junit.Assert.*
import org.junit.Test

class BackgroundHostingSessionTest {
    @Test fun staleStartCannotEnableAStoppedSession() {
        val session = BackgroundHostingSession()
        assertEquals(Pair(false, false), session.state())
        val token = session.request()
        assertEquals(Pair(true, false), session.state())
        session.stop()
        assertFalse(session.started(token))
        assertEquals(Pair(false, false), session.state())
    }

    @Test fun oldServiceDestructionCannotStopANewerSession() {
        val session = BackgroundHostingSession()
        val old = session.request()
        assertTrue(session.started(old))
        session.stop()
        val current = session.request()
        assertNotEquals(old, current)
        assertTrue(session.started(current))
        assertFalse(session.stop(old))
        assertEquals(Pair(true, true), session.state())
        assertTrue(session.stop(current))
        assertEquals(Pair(false, false), session.state())
    }
}
