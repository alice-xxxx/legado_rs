buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:9.4.1")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.3.20")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

tasks.register("clean").configure {
    delete("build")
}







// Foliver Android library compile SDK compatibility.
subprojects {
    pluginManager.withPlugin("com.android.library") {
        extensions.getByType<com.android.build.api.variant.LibraryAndroidComponentsExtension>()
            .finalizeDsl { library ->
                library.compileSdk = 37
                library.buildToolsVersion = "36.0.0"
                library.ndkVersion = "30.0.14904198"
                library.defaultConfig.minSdk = maxOf(library.defaultConfig.minSdk ?: 0, 26)
                library.compileOptions {
                    sourceCompatibility = JavaVersion.toVersion("11")
                    targetCompatibility = JavaVersion.toVersion("11")
                }
                tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinJvmCompile>().configureEach {
                    compilerOptions.jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.fromTarget("11"))
                }
            }
    }
}

// Foliver Android system certificate verifier.
allprojects {
    repositories {
        maven {
            url = uri("https://raw.githubusercontent.com/rustls/rustls-platform-verifier/maven-archive/android-release-support/maven/")
            content { includeGroup("org.rustls") }
        }
    }
}
