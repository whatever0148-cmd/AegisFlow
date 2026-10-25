//! Compiles swift/AegisANEBridge.swift into a static library and links it with CoreML.
//!
//! The Swift objects carry autolink entries for the Swift runtime (libswiftCore, Foundation
//! overlay, Concurrency); the linker resolves them through the SDK's usr/lib/swift stubs, and at
//! run time they load from the OS (/usr/lib/swift).

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn xcrun(args: &[&str]) -> String {
    let out = Command::new("xcrun").args(args).output().expect("xcrun must be available (Xcode or Command Line Tools)");
    assert!(out.status.success(), "xcrun {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("xcrun output is UTF-8").trim().to_string()
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let source = manifest.join("swift/AegisANEBridge.swift");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");

    let target = env::var("TARGET").unwrap_or_default();
    assert_eq!(target, "aarch64-apple-darwin", "aegis-ane-scorer needs Apple Silicon macOS (CoreML + Neural Engine)");
    let deployment = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "15.0".to_string());

    let library = out.join("libaegis_ane_bridge.a");
    let status = Command::new("xcrun")
        .args(["swiftc", "-O", "-wmo", "-parse-as-library", "-emit-library", "-static"])
        .args(["-module-name", "AegisANEBridge", "-swift-version", "5"])
        .arg("-target")
        .arg(format!("arm64-apple-macos{deployment}"))
        .arg("-module-cache-path")
        .arg(out.join("swift-module-cache"))
        .arg("-o")
        .arg(&library)
        .arg(&source)
        .status()
        .expect("failed to launch xcrun swiftc");
    assert!(status.success(), "swiftc failed to compile {}", source.display());

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=aegis_ane_bridge");
    println!("cargo:rustc-link-lib=framework=CoreML");
    println!("cargo:rustc-link-lib=framework=Foundation");

    let sdk = xcrun(&["--sdk", "macosx", "--show-sdk-path"]);
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    let swiftc = PathBuf::from(xcrun(&["--find", "swiftc"]));
    if let Some(toolchain) = swiftc.parent().and_then(|bin| bin.parent()) {
        println!("cargo:rustc-link-search=native={}", toolchain.join("lib/swift/macosx").display());
    }
}
