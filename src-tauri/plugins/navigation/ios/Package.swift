// swift-tools-version:5.5
import PackageDescription

let package = Package(
    name: "tauri-plugin-navigation",
    platforms: [.iOS(.v15)],
    products: [.library(name: "tauri-plugin-navigation", type: .static, targets: ["Navigation"])],
    dependencies: [.package(name: "Tauri", path: "../.tauri/tauri-api")],
    targets: [.target(name: "Navigation", dependencies: [.byName(name: "Tauri")], path: "Sources")]
)
