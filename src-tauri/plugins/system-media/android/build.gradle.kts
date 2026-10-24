plugins { id("com.android.library") }

extensions.configure<com.android.build.api.dsl.LibraryExtension> {
    namespace = "com.xyz.foliver.media"
    compileSdk = 37
    defaultConfig { minSdk = 26 }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
}

dependencies {
    implementation(project(":tauri-android"))
    implementation("androidx.appcompat:appcompat:1.6.0")
    implementation("com.fasterxml.jackson.core:jackson-databind:2.15.3")
}
