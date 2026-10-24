fn main() {
    tauri_plugin::Builder::new(&["set_gestures"])
        .ios_path("ios")
        .build();
}
