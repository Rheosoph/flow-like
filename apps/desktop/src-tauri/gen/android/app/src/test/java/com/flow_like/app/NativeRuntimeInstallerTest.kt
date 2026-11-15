package com.flow_like.app

import java.io.File
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class NativeRuntimeInstallerTest {
    @get:Rule val temporary = TemporaryFolder()

    @Test fun failedExtractionCleanupPreservesDependenciesAndOtherVersions() {
        val directory = temporary.newFolder("lib")
        val failed = File(directory, "libapp.so.42").apply { writeText("partial") }
        val preserved = listOf("libc++_shared.so.42", "libonnxruntime.so.42", "libapp.so.41", "libapp.so", "other")
            .map { File(directory, it).apply { writeText("keep $it") } }

        assertTrue(NativeRuntimeInstaller.removeFailedExtraction(directory, "app", "42"))

        assertFalse(failed.exists())
        preserved.forEach { assertEquals("keep ${it.name}", it.readText()) }
    }

    @Test fun cleanupRejectsUnknownLibrariesAndPathComponents() {
        val directory = temporary.newFolder("lib")
        val unknown = File(directory, "libunknown.so.42").apply { writeText("keep") }
        val outside = temporary.newFile("outside").apply { writeText("keep") }

        assertFalse(NativeRuntimeInstaller.removeFailedExtraction(directory, "unknown", "42"))
        assertFalse(NativeRuntimeInstaller.removeFailedExtraction(directory, "../app", "42"))
        assertFalse(NativeRuntimeInstaller.removeFailedExtraction(directory, "app", "42/../../outside"))
        assertFalse(NativeRuntimeInstaller.removeFailedExtraction(directory, "app", ""))

        assertEquals("keep", unknown.readText())
        assertEquals("keep", outside.readText())
    }

    @Test fun cleanupDoesNotDeleteDirectoriesOrRequireAnExtraction() {
        val directory = temporary.newFolder("lib")
        val nested = File(directory, "libapp.so.42").apply { mkdir() }
        val child = File(nested, "keep").apply { writeText("keep") }

        assertFalse(NativeRuntimeInstaller.removeFailedExtraction(directory, "app", "42"))
        assertFalse(NativeRuntimeInstaller.removeFailedExtraction(directory, "onnxruntime", "42"))
        assertTrue(nested.isDirectory)
        assertEquals("keep", child.readText())
    }
}
