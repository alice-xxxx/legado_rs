plugins {
    kotlin("jvm")
}

// KSP processor is a JVM build plugin. Its generated Kotlin source is compiled into the
// Android/iOS target; the parser runtime itself stays Kotlin Multiplatform.
dependencies {
    implementation("com.google.devtools.ksp:symbol-processing-api:2.3.11")
}

kotlin {
    jvmToolchain(17)
}
