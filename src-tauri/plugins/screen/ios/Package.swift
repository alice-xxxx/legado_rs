// swift-tools-version:5.5
import PackageDescription

let package = Package(
    name: "tauri-plugin-screen",
    platforms: [.iOS(.v15)],
    products: [.library(name: "tauri-plugin-screen", type: .static, targets: ["Screen"])],
    dependencies: [.package(name: "Tauri", path: "../.tauri/tauri-api")],
    targets: [.target(name: "Screen", dependencies: [.byName(name: "Tauri")], path: "Sources")]
)
