import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins { id("com.android.library"); id("org.jetbrains.kotlin.android") }

extensions.configure<com.android.build.api.dsl.LibraryExtension> {
    namespace = "com.xyz.foliver.updates"
    compileSdk = 37
    defaultConfig { minSdk = 26 }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
}
kotlin { compilerOptions.jvmTarget.set(JvmTarget.JVM_11) }

dependencies {
    implementation(project(":tauri-android"))
    implementation("androidx.core:core-ktx:1.18.0")
}
