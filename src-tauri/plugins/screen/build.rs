fn main() {
    tauri_plugin::Builder::new(&["apply", "get_info", "reset"])
        .android_path("android")
        .ios_path("ios")
        .build();
}
