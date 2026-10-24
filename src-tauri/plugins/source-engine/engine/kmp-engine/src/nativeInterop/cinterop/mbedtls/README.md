# mbedTLS 加密源码

源码来自 [Mbed-TLS/mbedtls](https://github.com/Mbed-TLS/mbedtls)。随仓库头文件的版本为 3.6.7，见 `include/mbedtls/build_info.h`；许可见 [LICENSE](LICENSE)。

`legado_mbedtls_config.h` 选择书源使用的摘要、对称加密、RSA 和随机数模块，不启用 TLS/X.509 协议实现。Kotlin/Native 绑定由相邻的 `mbedtls.def` 生成。

iOS 接线已存在：

- `src-tauri/plugins/source-engine/engine/kmp-engine/scripts/build-ios-native.sh` 编译 `libmbedtls.a`。
- `src-tauri/plugins/source-engine/engine/kmp-engine/build.gradle.kts` 配置 cinterop 和 framework 链接。
- `src-tauri/plugins/source-engine/build.rs` 将该静态库接入 Tauri 最终链接。

升级时同步检查头文件、所选 library 源码、配置开关和绑定，并在对应 iOS 工具链上验证编译与链接。
