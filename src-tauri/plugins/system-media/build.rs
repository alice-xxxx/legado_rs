fn main() {
    tauri_plugin::Builder::new(&["update", "clear"])
        .android_path("android")
        .ios_path("ios")
        .build();
}
