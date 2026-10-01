use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn resource_compiler() -> PathBuf {
    for key in ["RC", "LLVM_RC"] {
        if let Some(path) = env::var_os(key) {
            return path.into();
        }
    }
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            for name in ["rc.exe", "llvm-rc", "llvm-rc.exe"] {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return candidate;
                }
            }
        }
    }
    if let Some(program_files) = env::var_os("ProgramFiles(x86)") {
        let directory = PathBuf::from(program_files).join("Windows Kits/10/bin");
        let mut versions: Vec<_> = std::fs::read_dir(directory)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.join("x64/rc.exe").is_file())
            .collect();
        versions.sort_by_key(|path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .split('.')
                .map(|value| value.parse::<u32>().unwrap_or(0))
                .collect::<Vec<_>>()
        });
        if let Some(version) = versions.pop() {
            return version.join("x64/rc.exe");
        }
    }
    panic!("Windows resources require the Windows SDK rc.exe or LLVM llvm-rc; set RC to its path");
}

fn main() {
    for key in ["RC", "LLVM_RC", "ProgramFiles(x86)"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    for file in ["Yu.rc", "Yu.ico", "Yu.manifest"] {
        println!("cargo:rerun-if-changed=AppBundle/{file}");
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ENV").as_deref(),
        Ok("msvc"),
        "Yu Windows release resources currently support MSVC targets"
    );
    assert_eq!(
        env::var("CARGO_CFG_TARGET_ARCH").as_deref(),
        Ok("x86_64"),
        "Yu Windows release resources currently support x64"
    );
    let directory = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let output = directory.join("Yu.res");
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo package version");
    let mut parts: Vec<_> = version.split('.').collect();
    assert_eq!(
        parts.len(),
        3,
        "Windows resource version requires major.minor.patch"
    );
    parts.push("0");
    let bundle = Path::new(&env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"))
        .join("AppBundle");
    for file in ["Yu.rc", "Yu.ico"] {
        std::fs::copy(bundle.join(file), directory.join(file))
            .expect("stage Windows resource input");
    }
    let manifest = std::fs::read_to_string(bundle.join("Yu.manifest"))
        .expect("read Windows manifest")
        .replace(
            "version=\"0.1.0.0\"",
            &format!("version=\"{}\"", parts.join(".")),
        );
    std::fs::write(directory.join("Yu.manifest"), manifest)
        .expect("stage versioned Windows manifest");
    let status = Command::new(resource_compiler())
        .current_dir(&directory)
        .args(["/nologo", "/d", &format!("YU_VERSION={}", parts.join(","))])
        .args(["/d", &format!("YU_VERSION_STRING=\"{version}\"")])
        .arg("/fo")
        .arg(&output)
        .arg("Yu.rc")
        .status()
        .expect("run Windows resource compiler");
    assert!(status.success(), "Windows resource compilation failed");
    println!("cargo:rustc-link-arg={}", output.display());
}
