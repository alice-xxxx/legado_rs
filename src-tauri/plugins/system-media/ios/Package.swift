// swift-tools-version:5.5
import PackageDescription

let package = Package(
    name: "tauri-plugin-system-media",
    platforms: [.iOS(.v15)],
    products: [.library(name: "tauri-plugin-system-media", type: .static, targets: ["SystemMedia"])],
    dependencies: [.package(name: "Tauri", path: "../.tauri/tauri-api")],
    targets: [.target(name: "SystemMedia", dependencies: [.byName(name: "Tauri")], path: "Sources")]
)
