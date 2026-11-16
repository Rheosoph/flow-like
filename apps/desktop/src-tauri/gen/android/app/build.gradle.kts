import java.util.Properties
import java.nio.file.Files
import java.util.zip.ZipFile

plugins {
    id("com.android.application")
    id("com.google.gms.google-services")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

val keyProperties = Properties().apply {
    val configuredPath = providers.environmentVariable("ANDROID_KEY_PROPERTIES").orNull
    val propFile = rootProject.file(
        configuredPath ?: "key.properties"
    )
    require(configuredPath == null || propFile.isFile) {
        "ANDROID_KEY_PROPERTIES must point to an existing signing properties file"
    }
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}
val releaseSigningValues = mapOf(
    "storeFile" to "ANDROID_KEYSTORE_PATH",
    "storePassword" to "ANDROID_KEYSTORE_PASSWORD",
    "keyAlias" to "ANDROID_KEY_ALIAS",
    "keyPassword" to "ANDROID_KEY_PASSWORD",
).mapValues { (property, environment) ->
    providers.environmentVariable(environment).orNull ?: keyProperties.getProperty(property)
}
val hasReleaseSigning = releaseSigningValues.values.any { !it.isNullOrEmpty() }
require(hasReleaseSigning || providers.environmentVariable("ANDROID_REQUIRE_SIGNING").orNull != "1") {
    "Android release signing requires ANDROID_KEY_PROPERTIES, key.properties, or the ANDROID_KEYSTORE_PATH, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS and ANDROID_KEY_PASSWORD environment variables"
}
if (hasReleaseSigning) {
    val missing = releaseSigningValues.filterValues { it.isNullOrEmpty() }.keys
    require(missing.isEmpty()) { "Android release signing is missing: ${missing.joinToString()}" }
}

val enableBotLocalAi = providers.gradleProperty("enableBotLocalAi").orNull == "true"
val enableBotAppFunctions = providers.gradleProperty("enableBotAppFunctions").orNull == "true"
if (enableBotAppFunctions) apply(plugin = "com.google.devtools.ksp")

val prepareAndroidResources = tasks.register<PrepareAndroidResources>("prepareAndroidResources") {
    inputDirectory.set(layout.projectDirectory.dir("src/main/res"))
    outputDirectory.set(layout.buildDirectory.dir("generated/res/main"))
}

android {
    compileSdk = if (enableBotAppFunctions) 37 else 36
    ndkVersion = "29.0.14206865"
    namespace = "com.flow_like.app"
    dynamicFeatures += setOf(":runtime", ":editors")
    testBuildType = if (providers.gradleProperty("testReleaseDelivery").orNull == "true") "release" else "debug"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.flow_like.app"
        minSdk = if (enableBotLocalAi) 26 else 24
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        buildConfigField("boolean", "BOT_LOCAL_AI", enableBotLocalAi.toString())
        buildConfigField("boolean", "BOT_APP_FUNCTIONS", enableBotAppFunctions.toString())
    }
    sourceSets {
        getByName("main") {
            res.setSrcDirs(emptyList<String>())
            // Release libraries live in the downloadable runtime module.
            jniLibs.setSrcDirs(emptyList<String>())
            java.srcDir(if (enableBotLocalAi) "src/botLocalAi/java" else "src/botLocalAiDisabled/java")
            if (enableBotAppFunctions) {
                java.srcDir("src/botAppFunctions/java")
                res.srcDir("src/botAppFunctions/res")
            }
        }
        getByName("debug") {
            jniLibs.srcDir("src/main/jniLibs")
            jniLibs.srcDir(rootProject.file("../../thirdparty/onnxruntime/android"))
        }
        if (enableBotAppFunctions) {
            getByName("debug").manifest.srcFile("src/botAppFunctions/AndroidManifest.xml")
            getByName("release").manifest.srcFile("src/botAppFunctions/AndroidManifest.xml")
        }
    }
    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = rootProject.file(releaseSigningValues.getValue("storeFile")!!)
                storePassword = releaseSigningValues.getValue("storePassword")
                keyAlias = releaseSigningValues.getValue("keyAlias")
                keyPassword = releaseSigningValues.getValue("keyPassword")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            buildConfigField("boolean", "PLAY_FEATURE_DELIVERY", "false")
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
        }
        getByName("release") {
            buildConfigField("boolean", "PLAY_FEATURE_DELIVERY", "true")
            if (hasReleaseSigning) {
                signingConfig = signingConfigs.getByName("release")
            }
            isMinifyEnabled = true
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
        isCoreLibraryDesugaringEnabled = true
    }
    buildFeatures {
        buildConfig = true
    }
}

androidComponents.onVariants { variant ->
    variant.sources.res?.addGeneratedSourceDirectory(prepareAndroidResources, PrepareAndroidResources::outputDirectory)
    val variantName = variant.name.replaceFirstChar { it.uppercase() }
    val prepareNativeLoader = tasks.register<PrepareAndroidNativeLoader>("prepare${variantName}NativeLoader") {
        dependsOn("rustBuild$variantName")
        originalSource.set(layout.projectDirectory.file("src/main/java/com/flow_like/app/generated/Rust.kt"))
        outputDirectory.set(layout.buildDirectory.dir("generated/nativeLoader/${variant.name}"))
    }
    variant.sources.java?.addGeneratedSourceDirectory(prepareNativeLoader, PrepareAndroidNativeLoader::outputDirectory)
    val prepareWebDialogs = tasks.register<PrepareAndroidWebDialogs>("prepare${variantName}WebDialogs") {
        dependsOn("rustBuild$variantName")
        originalSource.set(layout.projectDirectory.file("src/main/java/com/flow_like/app/generated/RustWebChromeClient.kt"))
        outputDirectory.set(layout.buildDirectory.dir("generated/webDialogs/${variant.name}"))
    }
    variant.sources.java?.addGeneratedSourceDirectory(prepareWebDialogs, PrepareAndroidWebDialogs::outputDirectory)
}

tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
    // The prepared copy lives under com/flow_like/app/Rust.kt, outside this pattern.
    exclude("**/generated/Rust.kt")
    exclude("**/generated/RustWebChromeClient.kt")
}

androidComponents.onVariants(androidComponents.selector().withBuildType("debug")) { variant ->
    variant.packaging.jniLibs.apply {
        useLegacyPackaging.set(true)
        useLegacyPackagingFromBundle.set(true)
        keepDebugSymbols.addAll(
            listOf(
                "*/arm64-v8a/*.so",
                "*/armeabi-v7a/*.so",
                "*/x86/*.so",
                "*/x86_64/*.so",
            )
        )
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("com.google.android.play:feature-delivery:2.1.0")
    implementation("com.getkeepsafe.relinker:relinker:1.4.5")
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.5")
    implementation("androidx.work:work-runtime-ktx:2.9.1")
    implementation("androidx.sharetarget:sharetarget:1.2.0")
    implementation("com.google.android.gms:play-services-mlkit-document-scanner:16.0.0")
    implementation("com.google.android.gms:play-services-location:21.3.0")
    implementation("com.google.mlkit:text-recognition:16.0.1")
    if (enableBotLocalAi) {
        implementation("com.google.guava:guava:33.4.8-android")
        implementation("com.google.mlkit:genai-summarization:1.0.0-beta1")
        implementation("com.google.mlkit:entity-extraction:16.0.0-beta6")
    }
    if (enableBotAppFunctions) {
        implementation("androidx.appfunctions:appfunctions:1.0.0-beta01")
        add("ksp", "androidx.appfunctions:appfunctions-compiler:1.0.0-beta01")
    }
    implementation(platform("com.google.firebase:firebase-bom:34.10.0"))
    implementation("com.google.firebase:firebase-messaging")
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test:runner:1.5.2")
    androidTestImplementation("androidx.test:core:1.5.0")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")

val bundletool = configurations.create("bundletool")
dependencies.add(bundletool.name, "com.android.tools.build:bundletool:1.18.1")

// A normal assemble task emits the base APK only. Fuse both download modules for sideloading.
tasks.register("packageStandaloneUniversalRelease") {
    group = "build"
    description = "Build a signed APK containing the runtime and editors for installation without Play"
    dependsOn("bundleUniversalRelease")
    doLast {
        check(hasReleaseSigning) { "The standalone release APK requires the release signing key" }
        val bundle = layout.buildDirectory.file("outputs/bundle/universalRelease/app-universal-release.aab").get().asFile
        ZipFile(bundle).use { zip ->
            val names = zip.entries().asSequence().map { it.name }.toSet()
            check(names.any { it.startsWith("runtime/lib/") && it.endsWith("/libapp.so") }) {
                "The bundle is missing its downloadable native runtime"
            }
            check(names.none { it.startsWith("base/lib/") && it.endsWith("/libapp.so") }) {
                "The native runtime is still included in the base module"
            }
            check("editors/assets/editors/install.marker" in names && names.any { it.startsWith("editors/assets/frontend-wasm/") }) {
                "The bundle is missing its downloadable editor payloads"
            }
        }
        val output = layout.buildDirectory.dir("outputs/apk/standalone/release").get().asFile
        output.mkdirs()
        val archive = output.resolve("app-standalone-release.apks")
        val apk = output.resolve("app-standalone-release.apk")
        val aapt2Name = if (System.getProperty("os.name").startsWith("Windows")) "aapt2.exe" else "aapt2"
        val aapt2 = androidComponents.sdkComponents.sdkDirectory.get().asFile
            .resolve("build-tools/${android.buildToolsVersion}/$aapt2Name")
        val secrets = Files.createTempDirectory("flow-like-android-signing-")
        try {
            val storePassword = secrets.resolve("store-password")
            val keyPassword = secrets.resolve("key-password")
            Files.writeString(storePassword, releaseSigningValues.getValue("storePassword")!!)
            Files.writeString(keyPassword, releaseSigningValues.getValue("keyPassword")!!)
            project.javaexec {
                classpath = bundletool
                mainClass.set("com.android.tools.build.bundletool.BundleToolMain")
                maxHeapSize = "4g"
                args(
                    "build-apks", "--bundle=${bundle.absolutePath}", "--output=${archive.absolutePath}",
                    "--mode=universal", "--overwrite",
                    "--aapt2=${aapt2.absolutePath}",
                    "--ks=${rootProject.file(releaseSigningValues.getValue("storeFile")!!).absolutePath}",
                    "--ks-key-alias=${releaseSigningValues.getValue("keyAlias")}",
                    "--ks-pass=file:$storePassword", "--key-pass=file:$keyPassword",
                )
            }.assertNormalExitValue()
            ZipFile(archive).use { zip ->
                val entry = checkNotNull(zip.getEntry("universal.apk")) { "bundletool did not generate a standalone APK" }
                zip.getInputStream(entry).use { input -> apk.outputStream().use { input.copyTo(it) } }
            }
            archive.delete()
            logger.lifecycle("Standalone APK: ${apk.absolutePath}")
        } finally {
            secrets.toFile().deleteRecursively()
        }
    }
}
