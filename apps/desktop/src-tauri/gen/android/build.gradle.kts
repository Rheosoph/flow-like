buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.google.gms:google-services:4.4.4")
        classpath("com.android.tools.build:gradle:8.11.0")
        val botAppFunctions = project.findProperty("enableBotAppFunctions") == "true"
        val botLocalAi = project.findProperty("enableBotLocalAi") == "true"
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:${if (botAppFunctions || botLocalAi) "2.2.20" else "1.9.25"}")
        if (botAppFunctions) classpath("com.google.devtools.ksp:symbol-processing-gradle-plugin:2.2.20-2.0.4")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

subprojects {
    if (name == "tauri-plugin-remote-push") {
        pluginManager.withPlugin("org.jetbrains.kotlin.android") {
            // The current plugin checkout includes the same package under both source paths.
            // Keep its canonical app.tauri source without modifying Cargo's checkout.
            if (file("src/main/java/app/tauri/remotepush/PushNotificationPlugin.kt").isFile &&
                file("src/main/java/com/tauri/remotepush/PushNotificationPlugin.kt").isFile) {
                tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile>().configureEach {
                    exclude("**/com/tauri/remotepush/**")
                }
            }
        }
    }
}

tasks.register("clean").configure {
    delete("build")
}

plugins {
  // ...

  // Add the dependency for the Google services Gradle plugin
  // id("com.google.gms.google-services") version "4.4.4" apply false

}
