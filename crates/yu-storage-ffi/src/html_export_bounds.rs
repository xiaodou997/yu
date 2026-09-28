//! Deterministic quota/lifecycle checks. The deadline boundary is injected as
//! an Instant into the production checkpoint, not a five-minute wall-clock run.
use super::*;
use yu_text::TextBuffer;

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
    for format in ["html", "pdf"] {
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
