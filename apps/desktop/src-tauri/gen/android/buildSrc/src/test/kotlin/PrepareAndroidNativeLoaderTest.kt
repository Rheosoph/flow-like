import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class PrepareAndroidNativeLoaderTest {
    private val original = "System.loadLibrary(\"app\")"
    private val guarded = "NativeRuntimeInstaller.ensureNativeLoaded(FlowLikeApplication.context())"

    @Test
    fun preservesGeneratedBindingsWhileGuardingTheLoader() {
        val source = "package com.flow_like.app\nobject Rust { init { $original }\nexternal fun onCreate() }\n"
        assertEquals(source.replace(original, guarded), guardedAndroidNativeLoader(source))
    }

    @Test
    fun acceptsThePreviouslyGuardedSourceDuringMigration() {
        val source = "object Rust { init { $guarded } }"
        assertEquals(source, guardedAndroidNativeLoader(source))
    }

    @Test
    fun refusesMissingDuplicateOrMixedLoaders() {
        for (source in listOf(
            "object Rust {}",
            "System.loadLibrary(\"other\")",
            "$original\n$original",
            "$guarded\n$guarded",
            "$original\n$guarded",
        )) {
            assertThrows(IllegalStateException::class.java) { guardedAndroidNativeLoader(source) }
        }
    }
}
