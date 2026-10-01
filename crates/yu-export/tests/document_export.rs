use std::fs;
use yu_export::document::*;
use yu_export::portable::*;
use yu_markdown::EmbeddedSpan;
use yu_text::TextBuffer;

#[derive(Default)]
struct Resources {
    inputs: Vec<String>,
    cancelled: bool,
}
impl HtmlResources for Resources {
    fn checkpoint(&mut self) -> Result<(), String> {
        if self.cancelled {
            Err("cancelled".into())
        } else {
            Ok(())
        }
    }
    fn image(&mut self, destination: &str) -> Result<ExportImage, ResourceError> {
        if destination.starts_with("https:") || destination == "missing.png" {
            return Err(ResourceError::Warning("图片不可用".into()));
        }
        Ok(data_image(b"test-only-image", "image/png", 1, 1))
    }
    fn embedded(
        &mut self,
        _: EmbeddedSpan,
        source: &str,
        _: &HtmlOptions,
    ) -> Result<ExportImage, ResourceError> {
        self.inputs.push(source.to_owned());
        Ok(data_image(b"test-only-vector", "image/svg+xml", 10, 10))
    }
}
fn render(source: &str) -> (HtmlDocument, Resources) {
    let snapshot = TextBuffer::new(source).snapshot();
    let document = yu_markdown::parse(&snapshot);
    let mut resources = Resources::default();
    let result = export_html_document(&document, &HtmlOptions::default(), &mut resources)
        .expect("document export");
    assert_eq!(document.source().as_str(), source);
    assert_eq!(document.revision(), snapshot.revision());
    (result, resources)
}
#[test]
fn html_whole_semantics_and_clipboard_contract_remain_separate() {
    let source = "# 中文 🙂\n\n**粗体** *斜体* ~~删除~~ ==高亮== H~2~O x^2^\n\n- [x] 完成\n\n```rust\n<&>\n```\n\n| A | B |\n| - | - |\n| one | two |\n";
    let (result, _) = render(source);
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    for expected in [
        "<!doctype html>",
        "<strong>粗体</strong>",
        "<em>斜体</em>",
        "<del>删除</del>",
        "<mark>高亮</mark>",
        "<sub>2</sub>",
        "<sup>2</sup>",
        "<table>",
        "&lt;&amp;&gt;",
    ] {
        assert!(result.html.contains(expected), "missing {expected}");
    }
    assert_eq!(
        yu_export::export_html_fragment("<script>x</script>"),
        "<script>x</script>\n"
    );
}
#[test]
fn html_active_content_has_readable_fallback_without_execution() {
    let (result, _) = render(
        "<script>alert('x')</script>\n\n<img src=\"a.png\" onerror=\"alert(1)\">\n\n[x](javascript:alert%281%29)\n\n<style>@import 'https://example.test/a.css';</style>\n",
    );
    assert!(!result.warnings.is_empty());
    assert!(!result.html.contains("<script>"));
    assert!(!result.html.contains("src=\"https:"));
    assert!(!result.html.contains("href=\"javascript:"));
    assert!(result.html.contains("&lt;script&gt;"));
    assert!(result.html.contains("alert"));
}
#[test]
fn html_details_frontmatter_and_duplicate_headings() {
    let (result, _) = render(
        "---\ntitle: 分享标题\nsecret: private-key-do-not-export\n---\n\n[TOC]\n\n# 重复\n\n## 重复\n\n<details><summary>展开</summary><p>屏外正文</p></details>\n",
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert!(result.html.contains("<title>分享标题</title>"));
    assert!(!result.html.contains("private-key-do-not-export"));
    for expected in [
        "href=\"#yu-heading-0\"",
        "id=\"yu-heading-0\"",
        "href=\"#yu-heading-1\"",
        "id=\"yu-heading-1\"",
        "<details open>",
        "屏外正文",
    ] {
        assert!(result.html.contains(expected), "missing {expected}");
    }
}
#[test]
fn html_equation_numbering_and_static_resources_come_from_yu() {
    let (result, resources) = render(
        "引用 \\eqref{energy}。\n\n$$\nE=mc^2\\label{energy}\n$$\n\n```mermaid\nflowchart LR\nA-->B\n```\n",
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(result.embedded_count, 3);
    assert_eq!(resources.inputs.len(), 3);
    assert!(
        resources
            .inputs
            .iter()
            .any(|source| source.contains("mc^2") && !source.contains("\\label"))
    );
    assert!(result.html.contains("data:image/svg+xml;base64,"));
    assert!(!result.html.contains("MathJax"));
    assert!(!result.html.contains("mermaid.js"));
}
#[test]
fn html_table_math_and_mixed_repeated_footnotes() {
    let (result, resources) = render(
        "首个[^a]。\n\n<table><tr><td><span data-math-style=\"inline\">x^2</span><span data-yu-footnote=\"reference\">[^b]</span></td></tr></table>\n\n重复[^a]。\n\n[^a]: 第一条 **定义**\n\n[^b]: 第二条定义\n",
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(resources.inputs, ["x^2"]);
    assert_eq!(result.html.matches("id=\"yu-note-1\"").count(), 1);
    assert_eq!(result.html.matches("id=\"yu-note-2\"").count(), 1);
    assert_eq!(result.html.matches("href=\"#yu-note-1\"").count(), 2);
    assert!(result.html.contains("<strong>定义</strong>"));
    for i in 0..3 {
        assert!(result.html.contains(&format!("href=\"#yu-ref-{i}\"")));
    }
}
#[test]
fn html_local_images_are_data_and_remote_images_warn() {
    let (result, _) = render(
        "![本地](assets/a.png)\n\n![远程](https://example.test/a.png)\n\n![缺失](missing.png)\n",
    );
    assert!(result.html.contains("data:image/png;base64,"));
    assert!(!result.html.contains("src=\"https:"));
    assert!(!result.html.contains("assets/a.png"));
    assert!(!result.warnings.is_empty());
    assert!(result.html.contains("远程"));
    assert!(result.html.contains("缺失"));
}
#[test]
fn html_lf_crlf_utf8_identity_and_cancel() {
    for ending in ["\n", "\r\n"] {
        let source = format!("# 中文{ending}{ending}正文🙂 $x$。{ending}");
        let (result, _) = render(&source);
        assert!(result.warnings.is_empty());
        assert!(result.html.contains("正文🙂"));
    }
    let snapshot = TextBuffer::new("# 未保存正文").snapshot();
    let document = yu_markdown::parse(&snapshot);
    let mut resources = Resources {
        cancelled: true,
        ..Resources::default()
    };
    assert_eq!(
        export_html_document(&document, &HtmlOptions::default(), &mut resources)
            .expect_err("cancelled resource provider must stop export"),
        "cancelled"
    );
    assert_eq!(snapshot.as_str(), "# 未保存正文");
}
#[test]
fn destination_cancel_and_failure_keep_old_output() {
    let root = tempfile::tempdir().expect("tempdir");
    let target = root.path().join("中文 output.html");
    fs::write(&target, "old").expect("seed");
    let destination = Destination::capture(&target, true).expect("capture");
    assert!(
        destination
            .publish(b"new", &[], || Err("cancel".into()))
            .is_err()
    );
    assert_eq!(fs::read_to_string(&target).expect("read"), "old");
    assert_eq!(fs::read_dir(root.path()).expect("dir").count(), 1);
    let destination = Destination::capture(&target, true).expect("capture");
    fs::write(&target, "external").expect("write");
    assert!(destination.publish(b"new", &[], || Ok(())).is_err());
    assert_eq!(fs::read_to_string(&target).expect("read"), "external");
}
#[test]
fn destination_protects_source_resource_and_aliases() {
    let root = tempfile::tempdir().expect("tempdir");
    let source = root.path().join("source.md");
    fs::write(&source, "original").expect("seed");
    let protected = [ProtectedFile::capture(&source)];
    assert!(
        Destination::capture(&source, true)
            .expect("capture")
            .publish(b"export", &protected, || Ok(()))
            .is_err()
    );
    let hard = root.path().join("hard.html");
    fs::hard_link(&source, &hard).expect("hard link");
    assert!(
        Destination::capture(&hard, true)
            .expect("capture")
            .publish(b"export", &protected, || Ok(()))
            .is_err()
    );
    #[cfg(unix)]
    {
        let symlink = root.path().join("symbolic.html");
        std::os::unix::fs::symlink(&source, &symlink).expect("symlink");
        assert!(Destination::capture(&symlink, true).is_err());
    }
    assert_eq!(fs::read_to_string(&source).expect("read"), "original");
}
#[test]
fn image_snapshot_is_fixed_and_change_is_detected() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = root.path().join("中文 图片.png");
    fs::write(&path, b"initial").expect("seed");
    let mut images = FrozenImages::new(Some(root.path().join("source.md")), None);
    images.capture("中文%20图片.png").expect("capture");
    fs::write(&path, b"changed").expect("edit");
    assert_eq!(images.bytes("中文%20图片.png").expect("bytes"), b"initial");
    assert!(images.verify().is_err());
}
#[test]
fn image_snapshot_detects_same_length_rewrite_with_preserved_modified_time() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = root.path().join("image.png");
    fs::write(&path, b"initial").expect("seed");
    let modified = fs::metadata(&path)
        .expect("metadata")
        .modified()
        .expect("modified");
    let mut images = FrozenImages::new(Some(root.path().join("source.md")), None);
    images.capture("image.png").expect("capture");
    assert!(images.verify().is_ok());
    fs::write(&path, b"changed").expect("same length rewrite");
    fs::File::options()
        .write(true)
        .open(&path)
        .expect("file")
        .set_times(fs::FileTimes::new().set_modified(modified))
        .expect("restore modified time");
    assert!(images.verify().is_err());
    assert_eq!(images.bytes("image.png").expect("frozen"), b"initial");
}
#[test]
fn svg_security_rejects_active_and_external_content() {
    for markup in [
        "<svg><script>x</script></svg>",
        "<svg><image href=\"https://example.test/a\"/></svg>",
        "<svg><path onclick=\"x\"/></svg>",
        "<svg><style>@import 'x';</style></svg>",
        "<!DOCTYPE svg [<!ENTITY a 'x'>]><svg>&a;</svg>",
    ] {
        assert!(validate_svg(markup).is_err(), "accepted {markup}");
    }
    assert!(validate_svg("<?xml-stylesheet href=\"https://example.test/a.css\"?><svg/>").is_err());
    validate_svg("<svg xmlns=\"http://www.w3.org/2000/svg\"><defs><path id=\"a\" d=\"M0 0L1 1\"/></defs><use href=\"#a\"/></svg>").expect("static SVG");
}

#[test]
fn destination_commit_gate_preserves_old_file_on_rejection() {
    let root = tempfile::tempdir().expect("tempdir");
    let target = root.path().join("output.html");
    fs::write(&target, "old").expect("seed");
    let destination = Destination::capture(&target, true).expect("capture");
    assert!(
        destination
            .publish_guarded(
                b"new",
                &[],
                || Ok(()),
                || Err::<(), String>("cancel won commit gate".into())
            )
            .is_err()
    );
    assert_eq!(fs::read_to_string(&target).expect("read"), "old");
    assert_eq!(fs::read_dir(root.path()).expect("dir").count(), 1);
    Destination::capture(&target, true)
        .expect("capture")
        .publish_guarded(b"new", &[], || Ok(()), || Ok(()))
        .expect("commit");
    assert_eq!(fs::read_to_string(&target).expect("read"), "new");
    assert_eq!(fs::read_dir(root.path()).expect("dir").count(), 1);
}
