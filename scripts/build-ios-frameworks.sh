#!/usr/bin/env bash
set -euo pipefail

# Helper for the iOS framework cache/build steps. The iOS job should set
# IPHONEOS_DEPLOYMENT_TARGET once at job scope, then use this script's cache-key
# and build-frameworks modes so the cache hash and Gradle see the same target.
#
# When wiring the cache key, hash this recipe and its real inputs: the Kotlin
# Gradle files/wrapper, kmp-engine sources/scripts/schemas, QuickJS vendor source,
# the QuickJS processor, and src-tauri/include. Do not hash the whole workflow or
# the Swift plugin package/sources; they do not produce this XCFramework.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:-}"

if [[ -z "${IPHONEOS_DEPLOYMENT_TARGET:-}" ]]; then
  echo "Set IPHONEOS_DEPLOYMENT_TARGET in the iOS job environment." >&2
  exit 2
fi

case "$mode" in
  cache-key)
    {
      xcodebuild -version
      printf 'IPHONEOS_DEPLOYMENT_TARGET=%s\n' "$IPHONEOS_DEPLOYMENT_TARGET"
      xcrun --sdk iphoneos --show-sdk-version
      xcrun --sdk iphonesimulator --show-sdk-version
      printf 'targets=iosArm64,iosSimulatorArm64\n'
    } | shasum -a 256 | cut -c1-16
    ;;
  build-frameworks)
    (
      cd "$repo_root/kotlin"
      IPHONEOS_DEPLOYMENT_TARGET="$IPHONEOS_DEPLOYMENT_TARGET" \
        ./gradlew --no-daemon --console=plain \
          :kmp-engine:linkReleaseFrameworkIosArm64 \
          :kmp-engine:linkReleaseFrameworkIosSimulatorArm64
    )
    ;;
  package-xcframework)
    framework_root="$repo_root/src-tauri/plugins/source-engine/ios/Frameworks"
    output="$framework_root/LegadoSourceEngine.xcframework"
    mkdir -p "$framework_root"
    backup="$framework_root/.LegadoSourceEngine.backup.$$"
    if [[ -e "$backup" ]]; then
      echo "Refusing to overwrite existing XCFramework backup: $backup" >&2
      exit 1
    fi
    staging_root="$(mktemp -d "$framework_root/.LegadoSourceEngine.XXXXXX")"
    cleanup() {
      if [[ -e "$backup" ]]; then
        rm -rf "$output"
        mv "$backup" "$output" || true
      fi
      rm -rf "$staging_root"
    }
    trap cleanup EXIT
    staged_output="$staging_root/LegadoSourceEngine.xcframework"

    xcodebuild -create-xcframework \
      -framework "$repo_root/kotlin/kmp-engine/build/bin/iosArm64/releaseFramework/LegadoSourceEngine.framework" \
      -framework "$repo_root/kotlin/kmp-engine/build/bin/iosSimulatorArm64/releaseFramework/LegadoSourceEngine.framework" \
      -output "$staged_output"

    if [[ -L "$output" || ( -e "$output" && ! -d "$output" ) ]]; then
      echo "Refusing to replace non-directory XCFramework output: $output" >&2
      exit 1
    fi
    if [[ -e "$output" ]]; then
      mv "$output" "$backup"
    fi
    mv "$staged_output" "$output"
    rm -rf "$backup"
    ;;
  *)
    echo "Usage: $0 {cache-key|build-frameworks|package-xcframework}" >&2
    exit 2
    ;;
esac
