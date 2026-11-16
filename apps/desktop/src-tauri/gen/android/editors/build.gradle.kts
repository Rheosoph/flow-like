plugins {
    id("com.android.dynamic-feature")
}

android {
    namespace = "com.flow_like.app.editors"
    compileSdk = if (providers.gradleProperty("enableBotAppFunctions").orNull == "true") 37 else 36
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
        assets.srcDir(layout.buildDirectory.dir("generated/frontendAssets"))
    }
}

dependencies {
    implementation(project(":app"))
}

// The Rust build removes these payloads from EmbeddedAssets and stages them here.
tasks.configureEach {
    if (name.startsWith("merge") && name.endsWith("ReleaseAssets")) {
        val variant = name.removePrefix("merge").removeSuffix("Assets")
        dependsOn(":app:rustBuild$variant")
        doFirst {
            check(layout.buildDirectory.file("generated/frontendAssets/editors/install.marker").get().asFile.isFile) {
                "Editor payloads are missing. Rebuild the Android Rust library before packaging the release."
            }
        }
    }
}
