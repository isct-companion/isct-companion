fn main() {
    tauri_plugin::Builder::new(&["sign_in", "sign_out"])
        .ios_path("ios")
        .android_path("android")
        .build();
}
