//! Resource residency must follow the current plan, not the edit history.
use yu_assets::{
    EmbeddedRenderPayload, EmbeddedRenderRequest, EmbeddedResourceCache, EmbeddedResourceKind,
};
use yu_core::{ByteOffset, Revision, TextRange};
use yu_font::{GlyphAtlas, GlyphAtlasConfig};
use yu_render::RenderPlanBuilder;
use yu_scene::{EmbeddedSvgPrimitive, Rect, Rgba8, Scene, SceneBuilder};

fn scene(id: u64) -> (Scene, yu_assets::EmbeddedRenderPublication) {
    let revision = Revision::new(id);
    let source = TextRange::new(ByteOffset::ZERO, ByteOffset::new(4)).expect("source");
    let request = EmbeddedRenderRequest::new(
        revision,
        source,
        EmbeddedResourceKind::Math,
        format!("x_{id}"),
    )
    .expect("request");
    let publication = EmbeddedResourceCache::new().publish(request, revision,
        EmbeddedRenderPayload::svg(16, 16, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"><path d=\"M0 0h16v16H0Z\"/></svg>").expect("SVG")).expect("publication");
    let viewport = Rect::new(0.0, 0.0, 100.0, 100.0).expect("viewport");
    let mut scene = SceneBuilder::new(revision, viewport).expect("scene");
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
    (scene.finish(), publication)
}

#[test]
fn changing_formulas_does_not_retain_historical_upload_identities() {
    let atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("atlas"));
    let mut builder = RenderPlanBuilder::new();
    for id in 0..128 {
        let (scene, publication) = scene(id);
        builder
            .build_with_embedded(&scene, &atlas, &[publication])
            .expect("plan");
        assert_eq!(
            builder.uploaded_embedded_count(),
            1,
            "old formula identities retained at edit {id}"
        );
    }
    let empty = SceneBuilder::new(
        Revision::new(129),
        Rect::new(0.0, 0.0, 100.0, 100.0).expect("viewport"),
    )
    .expect("plain scene")
    .finish();
    builder.build(&empty, &atlas).expect("plain plan");
    assert_eq!(builder.uploaded_embedded_count(), 0);
}

#[test]
fn failed_plan_does_not_discard_current_upload_identity() {
    let atlas = GlyphAtlas::new(GlyphAtlasConfig::new(32, 32, 1).expect("atlas"));
    let mut builder = RenderPlanBuilder::new();
    let (first, publication) = scene(1);
    builder
        .build_with_embedded(&first, &atlas, std::slice::from_ref(&publication))
        .expect("first");
    let (invalid, _) = scene(2);
    assert!(builder.build_with_embedded(&invalid, &atlas, &[]).is_err());
    assert_eq!(builder.uploaded_embedded_count(), 1);
    assert!(
        builder
            .build_with_embedded(&first, &atlas, &[publication])
            .expect("retry")
            .embedded_uploads()
            .is_empty()
    );
}
