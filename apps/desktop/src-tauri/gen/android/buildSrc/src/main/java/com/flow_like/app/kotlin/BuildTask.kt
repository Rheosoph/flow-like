import java.io.File
import org.apache.tools.ant.taskdefs.condition.Os
import org.gradle.api.DefaultTask
import org.gradle.api.GradleException
import org.gradle.api.logging.LogLevel
import org.gradle.api.tasks.Input
import org.gradle.api.tasks.TaskAction

open class BuildTask : DefaultTask() {
    @Input
    var rootDirRel: String? = null
    @Input
    var target: String? = null
    @Input
    var release: Boolean? = null

    @TaskAction
    fun assemble() {
        val executable = """bunx""";
        try {
            runTauriCli(executable)
        } catch (e: Exception) {
            if (Os.isFamily(Os.FAMILY_WINDOWS)) {
                // Try different Windows-specific extensions
                val fallbacks = listOf(
                    "$executable.exe",
                    "$executable.cmd",
                    "$executable.bat",
                )

                var lastException: Exception = e
                for (fallback in fallbacks) {
                    try {
                        runTauriCli(fallback)
                        return
                    } catch (fallbackException: Exception) {
                        lastException = fallbackException
                    }
                }
                throw lastException
            } else {
                throw e;
            }
        }
    }

    fun runTauriCli(executable: String) {
        val rootDirRel = rootDirRel ?: throw GradleException("rootDirRel cannot be null")
        val target = target ?: throw GradleException("target cannot be null")
        val release = release ?: throw GradleException("release cannot be null")
        val args = listOf("tauri", "android", "android-studio-script");

        val tauriRoot = File(project.projectDir, rootDirRel).absoluteFile
        val ortAbiDir = when (target) {
            "aarch64" -> "arm64-v8a"
            "armv7" -> "armeabi-v7a"
            "i686" -> "x86"
            "x86_64" -> "x86_64"
            else -> null
        }
        val ortLibDir = if (ortAbiDir != null) {
            File(tauriRoot, "thirdparty/onnxruntime/android/$ortAbiDir")
        } else null
        val inheritedOrtLibLocation = project.providers.environmentVariable("ORT_LIB_LOCATION").orNull
        val ortLibLocation = if (inheritedOrtLibLocation != null) {
            val inheritedDir = File(inheritedOrtLibLocation)
            if (!inheritedDir.isAbsolute || !inheritedDir.isDirectory ||
                !File(inheritedDir, "libonnxruntime.so").isFile) {
                throw GradleException(
                    "ORT_LIB_LOCATION must be an absolute directory containing libonnxruntime.so. " +
                        "Correct it or unset it to use the bundled Android library."
                )
            }
            // ort-sys tracks the exact environment string, including any parent segments.
            inheritedOrtLibLocation
        } else {
            ortLibDir?.takeIf { it.isDirectory }?.canonicalPath
        }

        project.exec {
            workingDir(File(project.projectDir, rootDirRel))
            executable(executable)
            args(args)
            if (ortLibLocation != null) {
                environment("ORT_LIB_LOCATION", ortLibLocation)
                environment("ORT_PREFER_DYNAMIC_LINK", "1")
            }
            if (project.logger.isEnabled(LogLevel.DEBUG)) {
                args("-vv")
            } else if (project.logger.isEnabled(LogLevel.INFO)) {
                args("-v")
            }
            if (release) {
                args("--release")
            }
            args(listOf("--target", target))
        }.assertNormalExitValue()
    }
}
