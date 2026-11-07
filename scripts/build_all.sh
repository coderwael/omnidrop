#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="${ROOT_DIR}/core_rust"
MOBILE_DIR="${ROOT_DIR}/mobile_app"
DESKTOP_DIR="${ROOT_DIR}/desktop_app"
JNI_LIBS_DIR="${MOBILE_DIR}/android/app/src/main/jniLibs"

echo "[1/4] Building Rust Core Engine for Android ABIs (arm64-v8a, armeabi-v7a, x86_64)..."
mkdir -p "${JNI_LIBS_DIR}"
(
  cd "${RUST_DIR}"
  cargo ndk \
    -t arm64-v8a \
    -t armeabi-v7a \
    -t x86_64 \
    -o "${JNI_LIBS_DIR}" \
    build --release
)

echo "[2/4] Building Release Split-per-ABI Android APKs..."
(
  cd "${MOBILE_DIR}"
  flutter pub get
  flutter build apk --release --split-per-abi
)

echo "[3/4] Cross-Compiling Rust Core for Windows (x86_64-pc-windows-msvc)..."
(
  cd "${RUST_DIR}"
  cargo build --release --target x86_64-pc-windows-msvc
)

echo "[4/4] Building Windows Desktop Portable Release..."
(
  cd "${DESKTOP_DIR}"
  flutter pub get
  flutter build windows --release
  cp "${RUST_DIR}/target/x86_64-pc-windows-msvc/release/rust_omnidrop.dll" \
     "${DESKTOP_DIR}/build/windows/x64/runner/Release/"
)

echo "OmniDrop Release Artifacts Ready!"
