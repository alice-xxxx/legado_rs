#!/usr/bin/env bash
# 为独立 KMP 模块编译 iOS 所需的 QuickJS 与 mbedTLS 静态库。
# Rust 网络/存储仍由 `src-tauri` 的 Rust staticlib 实现；这里仅构建 Kotlin Native JS/crypto actual。
# 来源实现：原平台 scripts/build-ios-native.sh，路径和产物位置改为本提取模块内部。

set -euo pipefail

ENGINE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUICKJS_DIR="$ENGINE_DIR/../src/native/quickjs-ng"
MBEDTLS_DIR="$ENGINE_DIR/src/nativeInterop/cinterop/mbedtls"
OUT_ROOT="$ENGINE_DIR/build/iosNativeLibs"
IOS_MIN_VERSION="14.0"
TARGETS=("${@:-ios_arm64 ios_simulator_arm64}")

if [[ "${#TARGETS[@]}" -eq 1 && "${TARGETS[0]}" == *" "* ]]; then
    read -r -a TARGETS <<< "${TARGETS[0]}"
fi

if ! command -v xcrun >/dev/null 2>&1; then
    echo "[ios-native] Xcode command line tools are required (xcrun was not found)." >&2
    exit 1
fi

sdk_for_target() {
    case "$1" in
        *-simulator) echo "iphonesimulator" ;;
        *) echo "iphoneos" ;;
    esac
}

temporary_root=""
cleanup_temporary_root() {
    if [[ -n "$temporary_root" ]]; then
        case "$temporary_root" in
            "${TMPDIR:-/tmp}"/*) rm -rf -- "$temporary_root" ;;
            *) echo "[ios-native] refusing to remove unexpected path: $temporary_root" >&2; exit 1 ;;
        esac
    fi
}
trap cleanup_temporary_root EXIT

for target in "${TARGETS[@]}"; do
    case "$target" in
        ios_arm64)
            sdk="iphoneos"
            triple="arm64-apple-ios${IOS_MIN_VERSION}"
            konan="ios_arm64"
            ;;
        ios_simulator_arm64)
            sdk="iphonesimulator"
            triple="arm64-apple-ios${IOS_MIN_VERSION}-simulator"
            konan="ios_simulator_arm64"
            ;;
        *) echo "[ios-native] unsupported target: $target" >&2; exit 1 ;;
    esac
    compiler="$(xcrun --sdk "$sdk" --find clang)"
    sysroot="$(xcrun --sdk "$sdk" --show-sdk-path)"
    output_dir="$OUT_ROOT/$konan"

    quickjs_sources=(
        "$QUICKJS_DIR/quickjs.c"
        "$QUICKJS_DIR/dtoa.c"
        "$QUICKJS_DIR/libregexp.c"
        "$QUICKJS_DIR/libunicode.c"
    )
    quickjs_objects=()
    temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/legado-ios-quickjs.XXXXXX")"
    for index in "${!quickjs_sources[@]}"; do
        "$compiler" -target "$triple" -isysroot "$sysroot" -O2 -fPIC -std=gnu11 \
            -D_GNU_SOURCE -DQUICKJS_NG_BUILD -I"$QUICKJS_DIR" \
            -Wno-unused-parameter -Wno-sign-compare \
            -Wno-missing-field-initializers -Wno-implicit-fallthrough \
            -c "${quickjs_sources[$index]}" -o "$temporary_root/$index.o"
        quickjs_objects+=("$temporary_root/$index.o")
    done
    mkdir -p "$output_dir"
    libtool="$(xcrun --sdk "$sdk" --find libtool)"
    "$libtool" -static -no_warning_for_no_symbols \
        -o "$output_dir/libquickjs.a" "${quickjs_objects[@]}"
    rm -rf -- "$temporary_root"
    temporary_root=""

    mbedtls_sources=("$MBEDTLS_DIR"/library/*.c)
    mbedtls_objects=()
    temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/legado-ios-mbedtls.XXXXXX")"
    for index in "${!mbedtls_sources[@]}"; do
        "$compiler" -target "$triple" -isysroot "$sysroot" -O2 -fPIC -std=gnu99 \
            -DMBEDTLS_CONFIG_FILE=\"legado_mbedtls_config.h\" \
            -I"$MBEDTLS_DIR/include" -I"$MBEDTLS_DIR" \
            -c "${mbedtls_sources[$index]}" -o "$temporary_root/$index.o"
        mbedtls_objects+=("$temporary_root/$index.o")
    done
    "$libtool" -static -no_warning_for_no_symbols \
        -o "$output_dir/libmbedtls.a" "${mbedtls_objects[@]}"
    rm -rf -- "$temporary_root"
    temporary_root=""

    echo "[ios-native] built $output_dir/libquickjs.a and libmbedtls.a"
done
