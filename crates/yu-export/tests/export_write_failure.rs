//! Force a real write(2) failure without filling a disk or changing the user's
//! resource limits. Only the isolated child inherits a tiny file-size limit.
#![cfg(unix)]
use std::{fs, process::Command};
use yu_export::portable::Destination;
const MARKER: &[u8] = b"OLD-OUTPUT-KEEP";

#[test]
fn real_write_failure_preserves_old_output_and_cleans_temp() {
    let root = tempfile::Builder::new()
        .prefix("yu-export-write-bound-")
        .tempdir()
        .expect("isolated directory");
    fs::write(root.path().join("output.html"), MARKER).expect("old output");
    let result = Command::new("/bin/sh")
        .args([
            "-c",
            "ulimit -f 1 || exit 80; trap '' XFSZ || exit 81; exec \"$@\"",
            "yu-export-write-bound",
        ])
        .arg(std::env::current_exe().expect("test binary"))
        .args(["--exact", "bounded_write_child", "--ignored", "--nocapture"])
        .env("YU_EXPORT_WRITE_BOUND_ROOT", root.path())
        .output()
        .expect("bounded child");
    assert!(
        result.status.success(),
        "child failure: {} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("REAL-WRITE-REFUSAL-PASS"));
    assert_eq!(
        fs::read(root.path().join("output.html")).expect("retained output"),
        MARKER
    );
    assert_eq!(
        fs::read_dir(root.path()).expect("directory").count(),
        1,
        "temporary file leaked"
    );
}

#[test]
#[ignore = "Only launched by the parent with a process-local file-size limit"]
fn bounded_write_child() {
    let root = std::path::PathBuf::from(
        std::env::var_os("YU_EXPORT_WRITE_BOUND_ROOT").expect("parent-owned fixture"),
    );
    assert!(
        root.file_name()
            .expect("name")
            .to_string_lossy()
            .starts_with("yu-export-write-bound-")
    );
    let target = root.join("output.html");
    assert_eq!(fs::read(&target).expect("parent sentinel"), MARKER);
    let destination = Destination::capture(&target, true).expect("capture");
    let error = destination
        .publish(&vec![b'x'; 2 * 1024 * 1024], &[], || {
            // Checkpoints precede chunks; the first write can be short and then
            // fail with EFBIG inside write_all, so observation is after return.
            Ok(())
        })
        .expect_err("kernel must reject writes beyond child's file-size limit");
    // The production diagnostic identifies write_all, not create/sync/rename.
    assert!(
        error.contains("写入导出临时文件失败"),
        "unexpected failure: {error}"
    );
    assert_eq!(fs::read(&target).expect("old output"), MARKER);
    assert_eq!(fs::read_dir(root).expect("cleanup").count(), 1);
    println!("REAL-WRITE-REFUSAL-PASS");
}
