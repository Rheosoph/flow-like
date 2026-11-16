import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.tasks.CacheableTask
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction

private const val ORIGINAL_NATIVE_LOADER = "System.loadLibrary(\"app\")"
private const val GUARDED_NATIVE_LOADER =
    "NativeRuntimeInstaller.ensureNativeLoaded(FlowLikeApplication.context())"

internal fun guardedAndroidNativeLoader(source: String): String {
    val originalCount = source.split(ORIGINAL_NATIVE_LOADER).size - 1
    val guardedCount = source.split(GUARDED_NATIVE_LOADER).size - 1
    return when {
        originalCount == 1 && guardedCount == 0 ->
            source.replace(ORIGINAL_NATIVE_LOADER, GUARDED_NATIVE_LOADER)
        originalCount == 0 && guardedCount == 1 -> source
        else -> error(
            "Generated Rust.kt has an unexpected native loader; update the Android delivery integration"
        )
    }
}

@CacheableTask
abstract class PrepareAndroidNativeLoader : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val originalSource: RegularFileProperty

    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty

    @TaskAction
    fun prepare() {
        val prepared = guardedAndroidNativeLoader(originalSource.get().asFile.readText())
        val output = outputDirectory.file("com/flow_like/app/Rust.kt").get().asFile
        // Wry watches its generated source. Keep that file unchanged so Cargo can reuse it.
        if (!output.isFile || output.readText() != prepared) {
            output.parentFile.mkdirs()
            output.writeText(prepared)
        }
    }
}
