use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=native/metal_bridge.m");

    let target = env::var("TARGET").expect("Cargo must provide TARGET");
    if !target.ends_with("-apple-darwin") {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"));
    let object = out_dir.join("yu_metal_bridge.o");
    let archive = out_dir.join("libyu_metal_bridge.a");
    let source = PathBuf::from("native/metal_bridge.m");

    run(
        Command::new("clang")
            .args(["-fno-objc-arc", "-fmodules", "-c"])
            .arg(format!(
                "-fmodules-cache-path={}",
                out_dir.join("module-cache").display()
            ))
            .arg(&source)
            .arg("-o")
            .arg(&object),
        "compile the macOS Metal bridge",
    );
    run(
        Command::new("ar").args(["crus"]).arg(&archive).arg(&object),
        "archive the macOS Metal bridge",
    );
    run(
        Command::new("ranlib").arg(&archive),
        "index the macOS Metal bridge",
    );

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=yu_metal_bridge");
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-lib=framework=Foundation");
}

fn run(command: &mut Command, action: &str) {
    let status = command.status().unwrap_or_else(|error| {
        panic!("failed to {action}: {error}");
    });
    assert!(status.success(), "failed to {action}: {status}");
}
