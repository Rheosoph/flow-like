import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.RegularFileProperty
import org.gradle.api.tasks.CacheableTask
import org.gradle.api.tasks.InputFile
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction

internal fun brandedAndroidWebDialogs(source: String): String {
    val builder = "AlertDialog.Builder(view.context)"
    val input = "EditText(view.context)"
    require(source.split(builder).size - 1 == 3 && source.split(input).size - 1 == 1) {
        "Generated WebView dialogs changed; update the native dialog integration"
    }
    return source.replace(builder, "NativeUi.dialog(view.context)")
        .replace(input, "NativeUi.input(view.context, \"Response\", defaultValue).apply { inputType = android.text.InputType.TYPE_CLASS_TEXT }")
}

@CacheableTask
abstract class PrepareAndroidWebDialogs : DefaultTask() {
    @get:InputFile
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val originalSource: RegularFileProperty

    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty

    @TaskAction
    fun prepare() {
        val prepared = brandedAndroidWebDialogs(originalSource.get().asFile.readText())
        val output = outputDirectory.file("com/flow_like/app/RustWebChromeClient.kt").get().asFile
        // Compile a themed copy while leaving Wry's watched source intact.
        if (!output.isFile || output.readText() != prepared) {
            output.parentFile.mkdirs()
            output.writeText(prepared)
        }
    }
}
