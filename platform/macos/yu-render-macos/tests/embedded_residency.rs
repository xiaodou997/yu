//! Runs real Metal allocation and AppKit rasterization without a document window.
#![cfg(target_os = "macos")]
use yu_assets::{
    EmbeddedRenderPayload, EmbeddedRenderRequest, EmbeddedResourceCache, EmbeddedResourceKind,
};
use yu_core::{ByteOffset, Revision, TextRange};
use yu_font::{GlyphAtlas, GlyphAtlasConfig};
use yu_render::{RenderPlan, RenderPlanBuilder};
use yu_render_macos::{MetalDevice, MetalImageAtlas, MetalUploader};
use yu_scene::{EmbeddedSvgPrimitive, Rect, Rgba8, SceneBuilder};

fn plans(id: u64) -> (RenderPlan, RenderPlan) {
    plans_with_markup(
        id,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"><path d=\"M0 0h16v16H0Z\"/></svg>",
    )
}

fn plans_with_markup(id: u64, markup: &str) -> (RenderPlan, RenderPlan) {
    let revision = Revision::new(id);
    let source = TextRange::new(ByteOffset::ZERO, ByteOffset::new(4)).expect("source");
    let request = EmbeddedRenderRequest::new(
        revision,
        source,
        EmbeddedResourceKind::Math,
        format!("x_{id}"),
    )
    .expect("request");
    let publication = EmbeddedResourceCache::new()
        .publish(
            request,
            revision,
            EmbeddedRenderPayload::svg(16, 16, markup).expect("SVG"),
        )
        .expect("publication");
    let mut scene = SceneBuilder::new(
        revision,
        Rect::new(0.0, 0.0, 100.0, 100.0).expect("viewport"),
    )
    .expect("scene");
    scene
        .embedded_svg(EmbeddedSvgPrimitive::new(
            publication.key().fingerprint(),
            publication.generation(),
            publication.kind().tag(),
            source,
            Rect::new(0.0, 0.0, 16.0, 16.0).expect("bounds"),
            16,
            16,
            Rgba8::black(),
        ))
        .expect("primitive");
    let scene = scene.finish();
    let atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("atlas"));
    let mut builder = RenderPlanBuilder::new();
    let first = builder
        .build_with_embedded(&scene, &atlas, std::slice::from_ref(&publication))
        .expect("first");
    let repeated = builder
        .build_with_embedded(&scene, &atlas, &[publication])
        .expect("repeat");
    assert!(repeated.embedded_uploads().is_empty());
    (first, repeated)
}

fn empty_plan() -> RenderPlan {
    let scene = SceneBuilder::new(
        Revision::new(999),
        Rect::new(0.0, 0.0, 100.0, 100.0).expect("viewport"),
    )
    .expect("scene")
    .finish();
    RenderPlanBuilder::new()
        .build(
            &scene,
            &GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("atlas")),
        )
        .expect("plain")
}

#[test]
fn embedded_plan_releases_absent_textures() {
    let mut uploader = MetalUploader::new(MetalDevice::system_default().expect("Metal device"));
    let mut images = MetalImageAtlas::new();
    for id in 0..48 {
        let (plan, _) = plans(id);
        assert_eq!(
            images
                .sync_embedded_plan(&mut uploader, &plan)
                .expect("upload"),
            1
        );
        assert_eq!(
            images.embedded_resource_count(),
            1,
            "GPU retains historical edit {id}"
        );
    }
    images
        .sync_embedded_plan(&mut uploader, &empty_plan())
        .expect("plain frame");
    assert_eq!(images.embedded_resource_count(), 0);
}

#[test]
fn invalid_rasterization_preserves_existing_texture_residency() {
    let mut uploader = MetalUploader::new(MetalDevice::system_default().expect("Metal device"));
    let mut images = MetalImageAtlas::new();
    let (valid, cached) = plans(1);
    images
        .sync_embedded_plan(&mut uploader, &valid)
        .expect("valid");
    let (invalid, _) = plans_with_markup(2, "not an SVG document");
    assert!(images.sync_embedded_plan(&mut uploader, &invalid).is_err());
    assert_eq!(images.embedded_resource_count(), 1);
    assert_eq!(images.embedded_texture_bytes(), 16 * 16 * 4);
    assert_eq!(
        images
            .sync_embedded_plan(&mut uploader, &cached)
            .expect("previous plan preserved"),
        0
    );
}

#[test]
fn unsubmitted_intermediate_plan_does_not_hide_a_missing_texture() {
    let (first, cached) = plans(7);
    assert_eq!(first.embedded_uploads().len(), 1);
    let mut uploader = MetalUploader::new(MetalDevice::system_default().expect("Metal device"));
    let mut images = MetalImageAtlas::new();
    // Simulate the first prepared frame being dropped before any upload.
    assert_eq!(
        images
            .sync_embedded_plan(&mut uploader, &cached)
            .expect("latest plan"),
        1
    );
    assert_eq!(images.embedded_resource_count(), 1);
}

#[test]
fn retained_plan_recovers_after_gpu_eviction_even_without_delta_uploads() {
    let mut uploader = MetalUploader::new(MetalDevice::system_default().expect("Metal device"));
    let mut images = MetalImageAtlas::new();
    let (first, repeated) = plans(1);
    assert_eq!(
        images
            .sync_embedded_plan(&mut uploader, &first)
            .expect("first"),
        1
    );
    assert_eq!(
        images
            .sync_embedded_plan(&mut uploader, &repeated)
            .expect("cached"),
        0
    );
    images.retain_embedded_resources(&[]);
    assert_eq!(images.embedded_resource_count(), 0);
    assert_eq!(
        images
            .sync_embedded_plan(&mut uploader, &repeated)
            .expect("recover"),
        1
    );
    assert_eq!(images.embedded_resource_count(), 1);
}
