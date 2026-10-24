plugins {
    kotlin("jvm")
}

// KSP processor is a JVM build plugin. Its generated Kotlin source is compiled into the
// Android/iOS target; the parser runtime itself stays Kotlin Multiplatform.
val toolchain = rootProject.extra["engineToolchain"] as java.util.Properties

dependencies {
    implementation("com.google.devtools.ksp:symbol-processing-api:${toolchain.getProperty("ksp.version")}")
}

kotlin {
    jvmToolchain(toolchain.getProperty("java.version").toInt())
}
