use super::*;
#[test]
fn png_native_encode_cancel_budget_retry_and_dimensions() {
    let packet=br#"{"width":600,"height":1200,"margin":18,"pageNumbers":false,"maxPages":64,"minFigureScale":0.25,"images":[],"blocks":[{"kind":"paragraph","style":"body","align":"left","indent":0,"runs":[{"text":"PNG native test"}]}],"png":{"scale":1,"pixelWidth":800,"foreground":606679039,"background":4294967295,"link":157932287,"fontSize":16}}"#;
    let mut plan = prepare_png(packet, || true).expect("plan");
    assert_eq!(plan.sizes.len(), 1);
    assert_eq!(plan.sizes[0][0], 800);
    assert!(plan.encode(0, 256 * 1024 * 1024, || false).is_err());
    assert!(plan.encode(0, 1, || true).is_err());
    let bytes = plan.encode(0, 256 * 1024 * 1024, || true).expect("retry");
    assert_eq!(
        u32::from_be_bytes(bytes[16..20].try_into().expect("width")),
        plan.sizes[0][0]
    );
    assert_eq!(
        u32::from_be_bytes(bytes[20..24].try_into().expect("height")),
        plan.sizes[0][1]
    );
    assert!(plan.encode(1, 256 * 1024 * 1024, || true).is_err());
}
#[test]
fn png_native_directory_move_never_overwrites_even_empty_target() {
    let root = std::env::temp_dir().join(format!(
        "yu-png-move-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir(&root).expect("root");
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    let from = root.join("from");
    let to = root.join("to");
    std::fs::create_dir(&from).expect("from");
    std::fs::create_dir(&to).expect("to");
    std::fs::write(from.join("part-001.png"), b"KEEP").expect("part");
    assert!(move_directory_exclusive(&from, &to).is_err());
    assert!(from.join("part-001.png").is_file());
    assert_eq!(std::fs::read_dir(&to).expect("empty").count(), 0);
    let fresh = root.join("fresh");
    move_directory_exclusive(&from, &fresh).expect("publish");
    assert_eq!(
        std::fs::read(fresh.join("part-001.png")).expect("part"),
        b"KEEP"
    );
    assert!(!from.exists());
}
