//! Fixed 5A boundary/failure contracts. Synthetic image payloads here are not
//! browser/decoder evidence; real native resources are checked separately.
use std::fs;
use yu_export::document::{
    ExportImage, HtmlOptions, HtmlResources, MAX_RESOURCES, ResourceError, export_html_document,
};
use yu_export::portable::{
    Destination, FrozenImages, MAX_RESOURCE_BYTES, ProtectedFile, data_image,
};
use yu_markdown::EmbeddedSpan;
use yu_text::TextBuffer;

struct NoResources;
impl HtmlResources for NoResources {
    fn checkpoint(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn image(&mut self, _: &str) -> Result<ExportImage, ResourceError> {
        panic!("must reject before loading resources")
    }
    fn embedded(
        &mut self,
        _: EmbeddedSpan,
        _: &str,
        _: &HtmlOptions,
    ) -> Result<ExportImage, ResourceError> {
        panic!("no embedded resource expected")
    }
}

#[test]
fn frozen_resource_count_is_bounded_before_loading() {
    let root = tempfile::tempdir().expect("test directory");
    let mut images = FrozenImages::new(Some(root.path().join("source.md")), None);
    for i in 0..MAX_RESOURCES {
        images
            .capture(&format!("missing-{i}.png"))
            .expect("bounded missing image");
    }
    images
        .capture("missing-0.png")
        .expect("cached destination costs no new slot");
    assert!(
        matches!(images.capture("over-limit.png"), Err(ResourceError::Fatal(message)) if message.contains("2048"))
    );
}

#[test]
fn frozen_resource_single_byte_budget_rejects_sparse_oversize() {
    let root = tempfile::tempdir().expect("test directory");
    let image = root.path().join("large.png");
    fs::File::create(&image)
        .expect("sparse file")
        .set_len(MAX_RESOURCE_BYTES as u64 + 1)
        .expect("sparse length");
    let mut images = FrozenImages::new(Some(root.path().join("source.md")), None);
    assert!(
        matches!(images.capture("large.png"), Err(ResourceError::Fatal(message)) if message.contains("32 MiB"))
    );
    assert_eq!(
        fs::metadata(image).expect("unchanged file").len(),
        MAX_RESOURCE_BYTES as u64 + 1
    );
}

#[test]
fn untitled_relative_base_is_required_and_same_names_stay_distinct() {
    let mut absent = FrozenImages::new(None, None);
    assert!(
        matches!(absent.capture("a.png"), Err(ResourceError::Fatal(message)) if message.contains("基准"))
    );
    let root = tempfile::tempdir().expect("test directory");
    for (name, content) in [("one", b"first".as_slice()), ("two", b"second".as_slice())] {
        fs::create_dir(root.path().join(name)).expect("subdirectory");
        fs::write(root.path().join(name).join("same.png"), content).expect("image bytes");
    }
    let mut images = FrozenImages::new(Some(root.path().join("source.md")), None);
    for path in ["one/same.png", "two/same.png"] {
        images.capture(path).expect("capture");
    }
    assert_eq!(images.bytes("one/same.png").expect("first"), b"first");
    assert_eq!(images.bytes("two/same.png").expect("second"), b"second");
    images.verify().expect("fixed resources");
}

#[test]
fn final_checkpoint_cancel_cleans_temp_and_keeps_old_file() {
    let root = tempfile::tempdir().expect("test directory");
    let path = root.path().join("old.html");
    fs::write(&path, b"old").expect("old output");
    let target = Destination::capture(&path, true).expect("capture target");
    let mut observed_temporary = false;
    let error = target
        .publish(b"new", &[], || {
            if fs::read_dir(root.path()).expect("list").count() > 1 {
                observed_temporary = true;
                return Err("cancel after temporary creation".into());
            }
            Ok(())
        })
        .expect_err("must cancel");
    assert!(error.contains("cancel") && observed_temporary);
    assert_eq!(fs::read(&path).expect("old remains"), b"old");
    assert_eq!(fs::read_dir(root.path()).expect("no temp").count(), 1);
}

#[test]
fn target_appearance_parent_swap_and_resource_alias_are_rejected() {
    let root = tempfile::tempdir().expect("test directory");
    let parent = root.path().join("target");
    fs::create_dir(&parent).expect("target directory");
    let path = parent.join("output.html");
    let absent = Destination::capture(&path, false).expect("capture absent");
    fs::write(&path, b"external").expect("external output");
    assert!(absent.publish(b"new", &[], || Ok(())).is_err());
    let moved = Destination::capture(&path, true).expect("capture existing");
    fs::rename(&parent, root.path().join("original")).expect("move test directory");
    fs::create_dir(&parent).expect("replacement directory");
    assert!(moved.publish(b"new", &[], || Ok(())).is_err());
    assert_eq!(
        fs::read(root.path().join("original/output.html")).expect("old remains"),
        b"external"
    );
    let image = root.path().join("image.png");
    fs::write(&image, b"image").expect("image");
    fs::hard_link(&image, &path).expect("hard alias");
    let alias = Destination::capture(&path, true).expect("target alias");
    assert!(
        alias
            .publish(b"new", &[ProtectedFile::capture(&image)], || Ok(()))
            .is_err()
    );
    assert_eq!(fs::read(image).expect("image preserved"), b"image");
}

#[test]
fn invalid_style_is_rejected_before_resource_resolution() {
    let document = yu_markdown::parse(&TextBuffer::new("![image](a.png)").snapshot());
    for options in [
        HtmlOptions {
            width: 359,
            ..HtmlOptions::default()
        },
        HtmlOptions {
            font_size: f32::NAN,
            ..HtmlOptions::default()
        },
    ] {
        assert!(export_html_document(&document, &options, &mut NoResources).is_err());
    }
    assert!(
        data_image(b"fixture", "image/png", 1, 1)
            .data_uri
            .starts_with("data:image/png;base64,")
    );
}
