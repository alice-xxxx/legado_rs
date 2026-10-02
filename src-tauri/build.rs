fn main() {
    println!("cargo:rustc-check-cfg=cfg(mobile)");
    // Tauri 配置生成只服务桌面应用；无 desktop feature 的本地解析测试不链接 Tauri，
    // 因此也不调用 tauri-build，避免测试构建缺少 Tauri 依赖元数据时误判为应用配置错误。
    if std::env::var_os("CARGO_FEATURE_DESKTOP").is_some() {
        tauri_build::build()
    }
}
