use super::*;
fn png() -> Vec<u8> {
    b"\x89PNG\r\n\x1a\nTEST".to_vec()
}
#[test]
fn png_directory_publishes_whole_numbered_set() {
    let root = tempfile::tempdir().expect("root");
    let path = root.path().join("中文 images");
    let d = Destination::capture(&path, false).expect("target");
    d.publish_png_directory(
        2,
        &[],
        |_, _| Ok(png()),
        || Ok(()),
        || Ok(()),
        |a, b| fs::rename(a, b).map_err(|e| e.to_string()),
    )
    .expect("publish");
    assert_eq!(fs::read_dir(&path).expect("entries").count(), 2);
    assert_eq!(fs::read(path.join("part-001.png")).expect("part"), png());
    assert_eq!(fs::read(path.join("part-002.png")).expect("part"), png());
    assert!(Destination::capture(&path, false).is_err());
}
#[test]
fn png_partial_encoding_failure_cleans_only_owned_staging() {
    let root = tempfile::tempdir().expect("root");
    let keep = root.path().join("keep");
    fs::write(&keep, "KEEP").expect("keep");
    let path = root.path().join("images");
    let d = Destination::capture(&path, false).expect("target");
    let result = d.publish_png_directory(
        3,
        &[],
        |n, _| {
            if n == 1 {
                Err("cancelled encoder".into())
            } else {
                Ok(png())
            }
        },
        || Ok(()),
        || Ok(()),
        |_, _| panic!("must not commit"),
    );
    assert!(result.is_err());
    assert!(!path.exists());
    assert_eq!(fs::read_dir(root.path()).expect("entries").count(), 1);
    assert_eq!(fs::read(&keep).expect("keep"), b"KEEP");
}
#[test]
fn png_directory_external_appearance_and_commit_cancel_are_safe() {
    let root = tempfile::tempdir().expect("root");
    let path = root.path().join("images");
    let d = Destination::capture(&path, false).expect("target");
    let result = d.publish_png_directory(
        2,
        &[],
        |n, _| {
            if n == 1 {
                fs::create_dir(&path).expect("external directory");
                fs::write(path.join("external"), "KEEP").expect("external");
            }
            Ok(png())
        },
        || Ok(()),
        || Ok(()),
        |_, _| panic!("must not commit"),
    );
    assert!(result.is_err());
    assert_eq!(fs::read(path.join("external")).expect("external"), b"KEEP");
    let cancelled = root.path().join("cancelled");
    let d = Destination::capture(&cancelled, false).expect("target");
    let result = d.publish_png_directory(
        2,
        &[],
        |_, _| Ok(png()),
        || Ok(()),
        || Err::<(), _>("cancel at commit".into()),
        |_, _| panic!("must not commit"),
    );
    assert!(result.is_err());
    assert!(!cancelled.exists());
    assert_eq!(fs::read_dir(root.path()).expect("entries").count(), 1);
}
