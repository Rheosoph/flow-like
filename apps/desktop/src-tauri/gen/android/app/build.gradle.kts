import java.util.Properties

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

val enableBotLocalAi = providers.gradleProperty("enableBotLocalAi").orNull == "true"
val enableBotAppFunctions = providers.gradleProperty("enableBotAppFunctions").orNull == "true"
if (enableBotAppFunctions) apply(plugin = "com.google.devtools.ksp")

val prepareAndroidResources = tasks.register<PrepareAndroidResources>("prepareAndroidResources") {
    inputDirectory.set(layout.projectDirectory.dir("src/main/res"))
    outputDirectory.set(layout.buildDirectory.dir("generated/res/main"))
}

android {
    compileSdk = if (enableBotAppFunctions) 37 else 36
    namespace = "com.flow_like.app"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.flow_like.app"
        minSdk = if (enableBotLocalAi) 26 else 24
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
        buildConfigField("boolean", "BOT_LOCAL_AI", enableBotLocalAi.toString())
        buildConfigField("boolean", "BOT_APP_FUNCTIONS", enableBotAppFunctions.toString())
    }
    sourceSets {
        getByName("main") {
            res.setSrcDirs(emptyList<String>())
            // Mobile builds link ORT dynamically, so package the tracked libraries explicitly.
            jniLibs.srcDir(rootProject.file("../../thirdparty/onnxruntime/android"))
            java.srcDir(if (enableBotLocalAi) "src/botLocalAi/java" else "src/botLocalAiDisabled/java")
            if (enableBotAppFunctions) {
                java.srcDir("src/botAppFunctions/java")
                res.srcDir("src/botAppFunctions/res")
            }
        }
        if (enableBotAppFunctions) {
            getByName("debug").manifest.srcFile("src/botAppFunctions/AndroidManifest.xml")
            getByName("release").manifest.srcFile("src/botAppFunctions/AndroidManifest.xml")
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.useLegacyPackaging = true
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
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
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    coreLibraryDesugaring("com.android.tools:desugar_jdk_libs:2.1.5")
    implementation("androidx.work:work-runtime-ktx:2.9.1")
    implementation("androidx.sharetarget:sharetarget:1.2.0")
    implementation("com.google.android.gms:play-services-mlkit-document-scanner:16.0.0")
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
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")
