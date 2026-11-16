package com.flow_like.app

import org.junit.Assert.*
import org.junit.Test

class NativeInstallPolicyTest {
    @Test fun setupRequiresBothFeatures() {
        assertEquals(setOf("runtime", "editors"), NativeInstallPolicy.missingModules(true, emptySet(), emptySet()))
        assertEquals(setOf("editors"), NativeInstallPolicy.missingModules(true, setOf("runtime"), emptySet()))
        assertTrue(NativeInstallPolicy.missingModules(true, setOf("runtime", "editors"), emptySet()).isEmpty())
    }

    @Test fun fusedApkMarkersAndInstalledSplitsCanEachSatisfySetup() {
        assertTrue(NativeInstallPolicy.missingModules(true, emptySet(), setOf("runtime", "editors")).isEmpty())
        assertTrue(NativeInstallPolicy.missingModules(true, setOf("runtime"), setOf("editors")).isEmpty())
        assertEquals(setOf("runtime"), NativeInstallPolicy.missingModules(true, setOf("unrelated"), setOf("editors")))
    }

    @Test fun developerBuildDoesNotNeedPlayOrFeatureMarkers() {
        assertTrue(NativeInstallPolicy.missingModules(false, emptySet(), emptySet()).isEmpty())
    }

    @Test fun resumesAnActiveRelevantSessionAfterActivityRecreation() {
        val sessions = listOf(
            NativeInstallSession(3, setOf("runtime"), NativeInstallPhase.DOWNLOADING),
            NativeInstallSession(7, setOf("other"), NativeInstallPhase.DOWNLOADING),
            NativeInstallSession(8, setOf("editors"), NativeInstallPhase.FAILED),
        )
        assertEquals(3, NativeInstallPolicy.selectSession(sessions, 0)?.id)
        assertEquals(8, NativeInstallPolicy.selectSession(sessions, 8)?.id)
    }

    @Test fun oldFailedAndCanceledSessionsDoNotBlockANewAttempt() {
        val sessions = listOf(
            NativeInstallSession(1, setOf("runtime", "editors"), NativeInstallPhase.FAILED),
            NativeInstallSession(2, setOf("runtime"), NativeInstallPhase.CANCELED),
        )
        assertNull(NativeInstallPolicy.selectSession(sessions, 0))
        assertEquals(2, NativeInstallPolicy.selectSession(sessions, 2)?.id)
    }

    @Test fun progressHandlesUnknownTotalsAndBoundsWithoutIntegerOverflow() {
        assertNull(NativeInstallPolicy.progress(0, 0))
        assertEquals(50, NativeInstallPolicy.progress(500_000_000, 1_000_000_000))
        assertEquals(100, NativeInstallPolicy.progress(Long.MAX_VALUE, Long.MAX_VALUE))
        assertEquals(100, NativeInstallPolicy.progress(20, 10))
        assertEquals(0, NativeInstallPolicy.progress(-1, 10))
    }
}
