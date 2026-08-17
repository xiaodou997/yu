#![forbid(unsafe_code)]

use std::cell::Cell;
use std::convert::Infallible;
use std::env;
use std::error::Error;
use std::time::Instant;

use yu_assets::{ImageRequest, ImageRequestCandidate, ImageRequestPlan, ImageRequestPriority};
use yu_core::{ByteOffset, TextRange};
use yu_editor::{EditorDocument, ImageIntrinsicSize, ViewportConfig, ViewportRect, VisualRunStyle};
use yu_layout::{
    FontFaceId, Glyph, GlyphId, GlyphRun, LayoutConfig, Script, ShapedText, ShapingProvider,
    TextDirection,
};

/// A deterministic shaper is enough for this benchmark: the workload measures
/// how many image candidates a viewport/overscan query visits, not CoreText.
struct BenchShaper;

impl ShapingProvider for BenchShaper {
    type Error = Infallible;

    fn shape(
        &self,
        text: &str,
        source: TextRange,
        style: VisualRunStyle,
    ) -> Result<ShapedText, Self::Error> {
        let mut glyphs = Vec::with_capacity(text.chars().count());
        for (start, character) in text.char_indices() {
            let end = start + character.len_utf8();
            let glyph_source = TextRange::new(
                ByteOffset::new(source.start().get() + start as u64),
                ByteOffset::new(source.start().get() + end as u64),
            )
            .expect("shaper source range");
            glyphs.push(Glyph::new(
                GlyphId::from_raw(1),
                glyph_source,
                1.0,
                0.0,
                0.0,
            ));
        }
        let run = GlyphRun::new(
            FontFaceId::from_raw(1),
            source,
            style,
            TextDirection::Ltr,
            Script::Common,
            glyphs,
        );
        Ok(ShapedText::new(source, vec![run]))
    }
}

#[derive(Clone, Copy, Debug)]
struct Configuration {
    blocks: usize,
    iterations: usize,
}

impl Configuration {
    fn from_arguments() -> Result<Self, Box<dyn Error>> {
        let mut arguments = env::args().skip(1);
        let blocks: usize = match arguments.next() {
            Some(value) => value.parse()?,
            None => 100_000,
        };
        let iterations: usize = match arguments.next() {
            Some(value) => value.parse()?,
            None => 200,
        };
        if blocks == 0 || iterations == 0 {
            return Err("blocks and iterations must be positive".into());
        }
        Ok(Self { blocks, iterations })
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let configuration = Configuration::from_arguments()?;
    let source = image_fixture(configuration.blocks);
    println!("Yu viewport image scheduling benchmark");
    println!("markdown image blocks: {}", configuration.blocks);
    println!("iterations: {}", configuration.iterations);
    println!("source bytes: {}", source.len());
    println!();

    for overscan in [0.0_f32, 160.0, 640.0] {
        run_case(&source, configuration, overscan)?;
    }
    Ok(())
}

fn run_case(
    source: &str,
    configuration: Configuration,
    overscan: f32,
) -> Result<(), Box<dyn Error>> {
    let mut document = EditorDocument::new(source);
    document.set_viewport_config(ViewportConfig::new(
        LayoutConfig::new(640.0, 20.0),
        20.0,
        overscan,
    ))?;
    let shaper = BenchShaper;
    let image_calls = Cell::new(0_usize);
    let measured_blocks = Cell::new(0_usize);
    let scheduled_requests = Cell::new(0_usize);
    let duplicate_candidates = Cell::new(0_usize);
    let start = Instant::now();
    let estimated_height = document.markdown().blocks().len() as f32 * 20.0;
    for iteration in 0..configuration.iterations {
        let scroll = (iteration as f32 * 137.0) % estimated_height.max(1.0);
        let snapshot = document.visible_blocks_with_shaper_and_image_resolver(
            ViewportRect::new(scroll, 480.0),
            &shaper,
            |_| {
                image_calls.set(image_calls.get().saturating_add(1));
                ImageIntrinsicSize::new(320, 180).ok()
            },
        )?;
        measured_blocks.set(
            measured_blocks
                .get()
                .saturating_add(snapshot.blocks().len()),
        );
        let visible_top = scroll;
        let visible_bottom = scroll + 480.0;
        let plan = ImageRequestPlan::from_candidates(snapshot.blocks().iter().map(|block| {
            let priority = if block.y() + block.height() > visible_top && block.y() < visible_bottom
            {
                ImageRequestPriority::Visible
            } else {
                ImageRequestPriority::Overscan
            };
            // Reuse a small destination set so the benchmark exercises the
            // same-batch deduplication contract instead of only measuring
            // unique-resource work.
            let destination = format!("image-{}.png", block.index() % 4);
            let request = ImageRequest::new(document.revision(), block.source(), destination)
                .expect("benchmark image request");
            ImageRequestCandidate::new(request, block.index(), priority)
        }));
        scheduled_requests.set(
            scheduled_requests
                .get()
                .saturating_add(plan.stats().unique_count()),
        );
        duplicate_candidates.set(
            duplicate_candidates
                .get()
                .saturating_add(plan.stats().duplicate_count()),
        );
        std::hint::black_box(plan);
        std::hint::black_box(snapshot);
    }
    let elapsed = start.elapsed();
    println!(
        "overscan={overscan:.0}px blocks={} image-resolver-calls={} measured-blocks={} calls/iteration={:.1} scheduled-requests={} duplicate-candidates={} elapsed={elapsed:?}",
        document.markdown().blocks().len(),
        image_calls.get(),
        measured_blocks.get(),
        image_calls.get() as f64 / configuration.iterations as f64,
        scheduled_requests.get(),
        duplicate_candidates.get(),
    );
    Ok(())
}

fn image_fixture(blocks: usize) -> String {
    let mut source = String::with_capacity(blocks.saturating_mul(32));
    for index in 0..blocks {
        source.push_str("![image](image-");
        source.push_str(&index.to_string());
        source.push_str(".png)\n\n");
    }
    source
}
