plugins {
    kotlin("multiplatform") version "2.3.0"
    kotlin("plugin.serialization") version "2.3.0"
    id("com.vanniktech.maven.publish") version "0.36.0"
}

group = "com.flow-like"
version = "0.1.0"

repositories {
    mavenCentral()
}

kotlin {
    wasmWasi()

    sourceSets {
        commonMain {
            dependencies {
                implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.8.1")
            }
        }
    }
}

mavenPublishing {
    publishToMavenCentral(automaticRelease = true)
    signAllPublications()

    pom {
        name.set("Flow-Like WASM SDK")
        description.set("Kotlin/WASM SDK for building Flow-Like WASM nodes")
        url.set("https://github.com/TM9657/flow-like")
        licenses {
            license {
                name.set("MIT")
                url.set("https://opensource.org/licenses/MIT")
            }
        }
        developers {
            developer {
                id.set("TM9657")
                name.set("Flow-Like")
                url.set("https://flow-like.com")
            }
        }
        scm {
            connection.set("scm:git:git://github.com/TM9657/flow-like.git")
            developerConnection.set("scm:git:ssh://github.com/TM9657/flow-like.git")
            url.set("https://github.com/TM9657/flow-like")
        }
    }
}
