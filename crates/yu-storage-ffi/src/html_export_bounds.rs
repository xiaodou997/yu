//! Deterministic quota/lifecycle checks. The deadline boundary is injected as
//! an Instant into the production checkpoint, not a five-minute wall-clock run.
use super::*;
use yu_text::TextBuffer;

static EXPORT_TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn wait(job: &HtmlJob) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let status: Value = serde_json::from_str(&job.status_json()).expect("status");
        if matches!(status["phase"].as_str(), Some("ready" | "warnings")) {
            break;
        }
        assert_eq!(status["phase"], "preparing", "{status}");
        assert!(Instant::now() < deadline, "preparation did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn html_budget_lifecycle_two_jobs_deadline_and_source_refusal() {
    let _serial = EXPORT_TEST_SERIAL.lock().expect("export test serial");
    for format in ["html", "pdf", "png"] {
        check_format_budget(format);
    }
}
fn check_format_budget(format: &str) {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("yu-export-bounds-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).expect("isolated directory");
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let source = root.join("source.md");
    std::fs::write(&source, "# Boundaries\n").expect("source");
    let config = json!({"referenceDay":20724, "exportFormat":format});
    let snapshot = TextBuffer::new("# Boundaries\n").snapshot();
    let start =
        |name: &str| HtmlJob::start(snapshot.clone(), source.clone(), &root.join(name), &config);
    let first = start("one.html").expect("first");
    let second = start("two.html").expect("second");
    wait(&first);
    wait(&second);
    assert!(
        start("three.html").is_err(),
        "third task must not be admitted"
    );
    assert_eq!(TASK_TIMEOUT, Duration::from_secs(300));
    assert!(
        first
            .prepare_checkpoint_at(first.preparation_deadline - Duration::from_nanos(1))
            .is_ok()
    );
    let error = first
        .prepare_checkpoint_at(first.preparation_deadline)
        .expect_err("deadline reached");
    assert!(error.contains("300"));
    first.fail(error);
    let status: Value = serde_json::from_str(&first.status_json()).expect("status");
    assert_eq!(status["phase"], "failed");
    assert!(first.commit(false).is_err());
    assert!(!root.join("one.html").exists());
    // The worker and owner both release their handles before reclaiming a slot.
    drop(first);
    let deadline = Instant::now() + Duration::from_secs(5);
    while ACTIVE.load(Ordering::Acquire) > 1 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let third = start("three.html").expect("released slot reusable");
    wait(&third);
    third.cancel();
    assert!(third.commit(false).is_err());
    assert!(!root.join("three.html").exists());
    drop(third);
    second.cancel();
    drop(second);
    while ACTIVE.load(Ordering::Acquire) != 0 {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let oversized =
        TextBuffer::new("x".repeat(yu_export::document::MAX_SOURCE_BYTES + 1)).snapshot();
    let job = HtmlJob::start(oversized, source.clone(), &root.join("large.html"), &config)
        .expect("failed task status");
    let status: Value = serde_json::from_str(&job.status_json()).expect("status");
    assert_eq!(status["phase"], "failed");
    assert!(
        status["message"]
            .as_str()
            .expect("message")
            .contains("8 MiB")
    );
    assert!(!root.join("large.html").exists());
    assert_eq!(
        std::fs::read_to_string(source).expect("source"),
        "# Boundaries\n"
    );
}

#[test]
fn png_task_warning_and_file_failure_contracts() {
    let _serial = EXPORT_TEST_SERIAL.lock().expect("export test serial");
    use std::os::unix::fs::PermissionsExt;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("yu-png-safety-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).expect("isolated directory");
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Ok(entries) = std::fs::read_dir(&self.0) {
                for entry in entries.flatten() {
                    let _ = std::fs::set_permissions(
                        entry.path(),
                        std::fs::Permissions::from_mode(0o700),
                    );
                }
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    // The same owned 1x1 PNG fixture used by the native checks; no test-only dependency.
    let png = vec![
        137u8, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8,
        4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252, 255, 31, 0, 3,
        3, 2, 0, 239, 154, 229, 100, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let status = |job: &HtmlJob| serde_json::from_str::<Value>(&job.status_json()).expect("status");
    let settle = |job: &HtmlJob| {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let state = status(job);
            if !matches!(
                state["phase"].as_str(),
                Some("preparing" | "writing" | "committing")
            ) {
                break state;
            }
            assert!(Instant::now() < deadline, "{state}");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    for mode in [
        "warning-cancel",
        "warning-confirm",
        "write-denied",
        "target-changed",
        "frozen-resource",
        "untitled-no-base",
        "resource-budget",
        "split-warning",
    ] {
        let work = root.join(mode);
        std::fs::create_dir(&work).expect("case directory");
        let path = work.join("source.md");
        let target = work.join("output.png");
        let image = work.join("image.png");
        let mut source = "# PNG safety\n\nPNG-SAFETY-END\n".to_owned();
        if mode.starts_with("warning") || mode == "split-warning" {
            source.push_str("\n![missing](missing.png)\n");
        }
        if matches!(
            mode,
            "frozen-resource" | "untitled-no-base" | "resource-budget"
        ) {
            std::fs::write(&image, &png).expect("image");
            source.push_str("\n![frozen](image.png)\n");
        }
        if mode == "resource-budget" {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&image)
                .expect("image handle")
                .set_len(32 * 1024 * 1024 + 1)
                .expect("own sparse fixture");
        }
        if mode == "split-warning" {
            source.push_str(&"\nA bounded line for segmentation.\n".repeat(1800));
        }
        std::fs::write(&path, &source).expect("source");
        std::fs::write(&target, b"OLD-PNG-KEEP").expect("old output");
        let config = json!({"exportFormat":"png","width":800,"scale":1,"referenceDay":20724,"replaceExisting":true,"untitled":mode=="untitled-no-base"});
        let job = HtmlJob::start(
            TextBuffer::new(source.clone()).snapshot(),
            path.clone(),
            &target,
            &config,
        )
        .expect("PNG task");
        let prepared = settle(&job);
        if matches!(mode, "untitled-no-base" | "resource-budget") {
            assert_eq!(prepared["phase"], "failed", "{prepared}");
            assert!(job.commit(true).is_err());
        } else if matches!(mode, "warning-cancel" | "split-warning") {
            assert_eq!(
                prepared["phase"],
                if mode == "split-warning" {
                    "split"
                } else {
                    "warnings"
                }
            );
            assert!(
                job.commit(false).is_err(),
                "Resource warnings must require consent even when segmented"
            );
            assert_eq!(std::fs::read(&target).expect("old target"), b"OLD-PNG-KEEP");
            job.cancel();
            assert_eq!(status(&job)["phase"], "cancelled");
        } else {
            let expected_png = if mode == "frozen-resource" {
                let mut state = job.state.lock().expect("state");
                Some(
                    state
                        .ready
                        .as_mut()
                        .expect("ready")
                        .png
                        .as_mut()
                        .expect("plan")
                        .encode(0, yu_export::document::MAX_OUTPUT_BYTES, || true)
                        .expect("frozen reference encoding"),
                )
            } else {
                None
            };
            if mode == "frozen-resource" {
                let mut changed = png.clone();
                changed.push(0);
                std::fs::write(&image, changed).expect("external resource change");
            }
            if mode == "write-denied" {
                std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o500))
                    .expect("restrict own directory");
            }
            if mode == "target-changed" {
                std::fs::write(&target, b"EXTERNAL-PNG-KEEP").expect("external target");
            }
            job.commit(mode == "warning-confirm")
                .expect("commit request");
            let final_state = settle(&job);
            std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o700))
                .expect("restore own directory");
            if matches!(mode, "write-denied" | "target-changed") {
                assert_eq!(final_state["phase"], "failed", "{final_state}");
            } else {
                assert_eq!(
                    final_state["phase"],
                    if mode == "warning-confirm" {
                        "completed_with_warnings"
                    } else {
                        "completed"
                    },
                    "{final_state}"
                );
                let bytes = std::fs::read(&target).expect("PNG output");
                assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
                if let Some(expected) = expected_png {
                    assert_eq!(
                        bytes, expected,
                        "Resource changed after preparation must not change encoded pixels"
                    );
                }
            }
        }
        if !matches!(mode, "warning-confirm" | "frozen-resource") {
            assert_eq!(
                std::fs::read(&target).expect("retained target"),
                if mode == "target-changed" {
                    b"EXTERNAL-PNG-KEEP".as_slice()
                } else {
                    b"OLD-PNG-KEEP".as_slice()
                }
            );
        }
        assert_eq!(std::fs::read_to_string(&path).expect("source"), source);
        assert!(!work.join("output-images").exists());
        assert!(
            !std::fs::read_dir(&work)
                .expect("entries")
                .flatten()
                .any(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".yu-export-"))
        );
        drop(job);
        let deadline = Instant::now() + Duration::from_secs(5);
        while ACTIVE.load(Ordering::Acquire) != 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
