fn main() {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    build_apple_intelligence_bridge();

    tauri_build::build()
}

/// 把 swift/apple_intelligence.swift 編成靜態庫連進 binary（Handy 實證模式）。
/// SDK 沒有 FoundationModels 時改編 stub，並靠 -weak_framework 讓 app 在
/// 沒有該 framework 的舊 macOS 也能啟動（runtime 由 @available 把關）。
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn build_apple_intelligence_bridge() {
    use std::env;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const REAL_SWIFT_FILE: &str = "swift/apple_intelligence.swift";
    const STUB_SWIFT_FILE: &str = "swift/apple_intelligence_stub.swift";

    println!("cargo:rerun-if-changed={REAL_SWIFT_FILE}");
    println!("cargo:rerun-if-changed={STUB_SWIFT_FILE}");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let object_path = out_dir.join("apple_intelligence.o");
    let static_lib_path = out_dir.join("libclaro_apple_ai.a");

    // SDKROOT/SWIFTC 環境變數可覆寫，支援非 Xcode 工具鏈
    let sdk_path = env::var("SDKROOT").unwrap_or_else(|_| {
        String::from_utf8(
            Command::new("xcrun")
                .args(["--sdk", "macosx", "--show-sdk-path"])
                .output()
                .expect("Failed to locate macOS SDK")
                .stdout,
        )
        .expect("SDK path is not valid UTF-8")
        .trim()
        .to_string()
    });

    let has_foundation_models = Path::new(&sdk_path)
        .join("System/Library/Frameworks/FoundationModels.framework")
        .exists();
    let source_file = if has_foundation_models {
        REAL_SWIFT_FILE
    } else {
        println!("cargo:warning=FoundationModels SDK not found — building Apple Intelligence stub.");
        STUB_SWIFT_FILE
    };

    let swiftc_path = env::var("SWIFTC").unwrap_or_else(|_| {
        String::from_utf8(
            Command::new("xcrun")
                .args(["--find", "swiftc"])
                .output()
                .expect("Failed to locate swiftc")
                .stdout,
        )
        .expect("swiftc path is not valid UTF-8")
        .trim()
        .to_string()
    });

    // -parse-as-library：避免 script mode 產生 _main 符號蓋掉 Rust 的 main
    let status = Command::new(&swiftc_path)
        .args([
            "-parse-as-library",
            "-target",
            "arm64-apple-macosx11.0",
            "-sdk",
            &sdk_path,
            "-O",
            "-c",
            source_file,
            "-o",
            object_path.to_str().expect("object path utf8"),
        ])
        .status()
        .expect("Failed to invoke swiftc");
    assert!(status.success(), "swiftc failed to compile {source_file}");

    // 走 xcrun 定位 Apple libtool——PATH 上可能有同名的 GNU libtool（conda/brew）
    let status = Command::new("xcrun")
        .args([
            "libtool",
            "-static",
            "-o",
            static_lib_path.to_str().expect("lib path utf8"),
            object_path.to_str().expect("object path utf8"),
        ])
        .status()
        .expect("Failed to run libtool");
    assert!(status.success(), "libtool failed");

    let toolchain_swift_lib = Path::new(&swiftc_path)
        .parent()
        .and_then(|p| p.parent())
        .map(|root| root.join("lib/swift/macosx"))
        .expect("Unable to determine Swift toolchain lib directory");
    let sdk_swift_lib = Path::new(&sdk_path).join("usr/lib/swift");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=claro_apple_ai");
    println!("cargo:rustc-link-search=native={}", toolchain_swift_lib.display());
    println!("cargo:rustc-link-search=native={}", sdk_swift_lib.display());
    println!("cargo:rustc-link-lib=framework=Foundation");
    if has_foundation_models {
        println!("cargo:rustc-link-arg=-weak_framework");
        println!("cargo:rustc-link-arg=FoundationModels");
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}
