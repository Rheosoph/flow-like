import javax.inject.Inject
import org.gradle.api.DefaultTask
import org.gradle.api.file.DirectoryProperty
import org.gradle.api.file.FileSystemOperations
import org.gradle.api.tasks.InputDirectory
import org.gradle.api.tasks.OutputDirectory
import org.gradle.api.tasks.PathSensitive
import org.gradle.api.tasks.PathSensitivity
import org.gradle.api.tasks.TaskAction

abstract class PrepareAndroidResources @Inject constructor(
    private val files: FileSystemOperations,
) : DefaultTask() {
    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val inputDirectory: DirectoryProperty

    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty

    @TaskAction
    fun prepare() {
        files.sync {
            from(inputDirectory)
            into(outputDirectory)
            // Tauri icon generation can leave PNGs alongside Android Studio's WebP icons.
            exclude {
                val file = it.file
                file.parentFile.name.startsWith("mipmap-") && file.name.startsWith("ic_launcher") &&
                    file.extension == "png" && file.resolveSibling("${file.nameWithoutExtension}.webp").isFile
            }
        }
    }
}
