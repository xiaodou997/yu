use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=native/metal_bridge.m");
    println!("cargo:rerun-if-changed=native/image_bridge.m");
    println!("cargo:rerun-if-changed=native/yu_shaders.metal");

    let target = env::var("TARGET").expect("Cargo must provide TARGET");
    if !target.ends_with("-apple-darwin") {
        return;
    }
    let architecture = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        other => panic!("unsupported macOS architecture: {other:?}"),
    };

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"));
    let air = out_dir.join("yu_shaders.air");
    let library = out_dir.join("yu_shaders.metallib");
    run(
        Command::new("xcrun")
            .args([
                "--sdk",
                "macosx",
                "metal",
                "-c",
                "-mmacosx-version-min=26.0",
                "native/yu_shaders.metal",
                "-o",
            ])
            .arg(&air),
        "compile Metal shader IR",
    );
    run(
        Command::new("xcrun")
            .args(["--sdk", "macosx", "metallib"])
            .arg(&air)
            .arg("-o")
            .arg(&library),
        "link Metal library",
    );
    let metal_object = out_dir.join("yu_metal_bridge.o");
    let image_object = out_dir.join("yu_image_bridge.o");
    let archive = out_dir.join("libyu_metal_bridge.a");
    let metal_source = PathBuf::from("native/metal_bridge.m");
    let image_source = PathBuf::from("native/image_bridge.m");

    run(
        Command::new("clang")
            .args([
                "-arch",
                architecture,
                "-fno-objc-arc",
                "-fblocks",
                "-fmodules",
                "-mmacosx-version-min=26.0",
                "-c",
            ])
            .arg(format!(
                "-fmodules-cache-path={}",
                out_dir.join("module-cache").display()
            ))
            .arg(&metal_source)
            .arg("-o")
            .arg(&metal_object),
        "compile the macOS Metal bridge",
    );
    run(
        Command::new("clang")
            .args([
                "-arch",
                architecture,
                "-fno-objc-arc",
                "-fmodules",
                "-mmacosx-version-min=26.0",
                "-c",
            ])
            .arg(format!(
                "-fmodules-cache-path={}",
                out_dir.join("module-cache").display()
            ))
            .arg(&image_source)
            .arg("-o")
            .arg(&image_object),
        "compile the macOS ImageIO bridge",
    );
    run(
        Command::new("ar")
            .args(["crus"])
            .arg(&archive)
            .arg(&metal_object)
            .arg(&image_object),
        "archive the macOS Metal bridge",
    );
    run(
        Command::new("ranlib").arg(&archive),
        "index the macOS Metal bridge",
    );

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=yu_metal_bridge");
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=ImageIO");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
}

fn run(command: &mut Command, action: &str) {
    let status = command.status().unwrap_or_else(|error| {
        panic!("failed to {action}: {error}");
    });
    assert!(status.success(), "failed to {action}: {status}");
}
