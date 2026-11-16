plugins {
    id("com.android.dynamic-feature")
}

android {
    namespace = "com.flow_like.app.runtime"
    compileSdk = if (providers.gradleProperty("enableBotAppFunctions").orNull == "true") 37 else 36
    ndkVersion = "29.0.14206865"
    defaultConfig {
        minSdk = if (providers.gradleProperty("enableBotLocalAi").orNull == "true") 26 else 24
    }
    flavorDimensions += "abi"
    productFlavors {
        listOf("universal", "arm64", "arm", "x86", "x86_64").forEach { architecture ->
            create(architecture) { dimension = "abi" }
        }
    }
    sourceSets.getByName("release") {
        jniLibs.srcDir(project(":app").file("src/main/jniLibs"))
        jniLibs.srcDir(rootProject.file("../../thirdparty/onnxruntime/android"))
    }
}

dependencies {
    implementation(project(":app"))
}

// Tauri stages libapp and libc++ before this module packages native libraries.
tasks.configureEach {
    if (name.startsWith("merge") && name.endsWith("ReleaseJniLibFolders")) {
        val variant = name.removePrefix("merge").removeSuffix("JniLibFolders")
        dependsOn(":app:rustBuild$variant")
    }
}
