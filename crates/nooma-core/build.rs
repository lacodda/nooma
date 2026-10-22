//! What the build adds to linking: on macOS, the compiler's runtime library,
//! for the model runner.
//!
//! ONNX Runtime is linked statically, as `ort` downloads it. On macOS it is
//! built with its CoreML provider, whose code asks the version of the system
//! the way Objective-C does (`@available`) - a call into clang's runtime
//! library, `__isPlatformVersionAtLeast`. clang links that library into
//! whatever it builds; rustc links without it, and the symbol is missing
//! from every binary that runs the model. It is linked here, from the
//! toolchain's own copy, when the model runner is built for macOS - and
//! reaches every binary that links this crate.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let semantic = std::env::var_os("CARGO_FEATURE_SEMANTIC").is_some();
    let macos = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "macos");
    if !(semantic && macos) {
        return;
    }
    let resources = Command::new("xcrun").args(["clang", "-print-resource-dir"]).output();
    let dir = match resources {
        Ok(output) if output.status.success() => PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()).join("lib/darwin"),
        _ => {
            println!("cargo:warning=`xcrun clang -print-resource-dir` failed: ONNX Runtime will not link without clang's runtime library");
            return;
        }
    };
    if !dir.join("libclang_rt.osx.a").exists() {
        println!("cargo:warning=no libclang_rt.osx.a in {}: ONNX Runtime will not link without it", dir.display());
        return;
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
