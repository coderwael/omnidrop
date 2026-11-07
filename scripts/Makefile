# Task 6: Production Build Automation for OmniDrop Ecosystem
# Compiles `librust_omnidrop` for Android ABIs via `cargo-ndk` and Desktop targets,
# copies shared libraries into Flutter JNI/Desktop runner directories, and builds
# release APKs and Windows/Linux binaries.

SHELL := /bin/bash
RUST_DIR := ../core_rust
MOBILE_DIR := ../mobile_app
DESKTOP_DIR := ../desktop_app
ANDROID_JNI_DIR := $(MOBILE_DIR)/android/app/src/main/jniLibs

.PHONY: all android-rust desktop-rustwindows desktop-rust-linux apk windows-release linux-release test clean

all: android-rust apk windows-release

# 1. Cross-compile Rust core for Android ABIs (arm64-v8a, armeabi-v7a, x86_64)
android-rust:
	@echo "==> Compiling librust_omnidrop.so for Android ABIs via cargo-ndk..."
	cd $(RUST_DIR) && cargo ndk \
		-t arm64-v8a \
		-t armeabi-v7a \
		-t x86_64 \
		-o $(ANDROID_JNI_DIR) \
		build --release

# 2. Compile Rust core for Windows (x86_64-pc-windows-msvc)
desktop-rust-windows:
	@echo "==> Compiling rust_omnidrop.dll for Windows x86_64-pc-windows-msvc..."
	cd $(RUST_DIR) && cargo build --release --target x86_64-pc-windows-msvc
	mkdir -p $(DESKTOP_DIR)/build/windows/x64/runner/Release
	cp $(RUST_DIR)/target/x86_64-pc-windows-msvc/release/rust_omnidrop.dll \
		$(DESKTOP_DIR)/build/windows/x64/runner/Release/

# 3. Compile Rust core for Linux (x86_64-unknown-linux-gnu)
desktop-rust-linux:
	@echo "==> Compiling librust_omnidrop.so for Linux x86_64-unknown-linux-gnu..."
	cd $(RUST_DIR) && cargo build --release --target x86_64-unknown-linux-gnu
	mkdir -p $(DESKTOP_DIR)/build/linux/x64/release/bundle/lib
	cp $(RUST_DIR)/target/x86_64-unknown-linux-gnu/release/librust_omnidrop.so \
		$(DESKTOP_DIR)/build/linux/x64/release/bundle/lib/

# 4. Build Split-per-ABI Release Android APKs
apk: android-rust
	@echo "==> Building Release Android APKs (--split-per-abi)..."
	cd $(MOBILE_DIR) && flutter pub get && flutter build apk --release --split-per-abi

# 5. Build Windows Desktop Release Binary
windows-release: desktop-rust-windows
	@echo "==> Building Flutter Desktop Windows Release..."
	cd $(DESKTOP_DIR) && flutter pub get && flutter build windows --release
	cp $(RUST_DIR)/target/x86_64-pc-windows-msvc/release/rust_omnidrop.dll \
		$(DESKTOP_DIR)/build/windows/x64/runner/Release/

# 6. Build Linux Desktop Release Bundle
linux-release: desktop-rust-linux
	@echo "==> Building Flutter Desktop Linux Release..."
	cd $(DESKTOP_DIR) && flutter pub get && flutter build linux --release
	cp $(RUST_DIR)/target/x86_64-unknown-linux-gnu/release/librust_omnidrop.so \
		$(DESKTOP_DIR)/build/linux/x64/release/bundle/lib/

test:
	cd $(RUST_DIR) && cargo test --release

clean:
	cd $(RUST_DIR) && cargo clean
	cd $(MOBILE_DIR) && flutter clean
	cd $(DESKTOP_DIR) && flutter clean
