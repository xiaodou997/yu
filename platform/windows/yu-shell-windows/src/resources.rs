//! Bounded, revision-bound background resources for the native editor surface.
use crate::ShellError;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use yu_assets::*;
use yu_core::{Revision, TextRange};
use yu_editor::{BlockWidget, ImageIntrinsicSize, LayoutContext};
use yu_embedded_client::{NativeRendererClient, RenderControl, RenderFailure};
use yu_font_windows::DirectWriteShaper;
use yu_render_windows::{D3DRenderError, D3DRenderer, ResourceRasterizer};
use yu_workspace::{Appearance, ViewportFrameBuilder};

const MAX_IN_FLIGHT: usize = 4;
const MAX_DECODE_DIMENSION: u32 = 2048;
const EMBEDDED_PIXEL_BUDGET: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum JobKey {
    Image(ImageKey),
    Embedded(EmbeddedResourceKey),
}
enum Job {
    Image(ImageRequest, PathBuf),
    Embedded(EmbeddedRenderRequest, Option<EmbeddedRenderPayload>),
}
enum Completed {
    Image(ImageRequest, Result<DecodedImage, String>),
    Embedded(
        EmbeddedRenderRequest,
        Result<(EmbeddedRenderPayload, DecodedImage), RenderFailure>,
    ),
}

pub struct ResourceHost {
    pub contrast: Option<yu_scene::ContrastPalette>,
    jobs: Sender<Job>,
    results: Receiver<Completed>,
    control: RenderControl,
    in_flight: HashMap<JobKey, Revision>,
    images: ImageCache,
    embedded: EmbeddedResourceCache,
    embedded_pixels: HashMap<EmbeddedResourceKey, (u64, DecodedImage)>,
    ready_embedded: Vec<EmbeddedRenderPublication>,
    image_occurrences: Vec<ImageRequest>,
    pub identity: u64,
    pub path: PathBuf,
    pub appearance: Appearance,
    revision: Option<Revision>,
    failures: u64,
    stale_results: u64,
    failed_ranges: Vec<TextRange>,
}

impl Drop for ResourceHost {
    fn drop(&mut self) {
        self.control.close();
    }
}

impl ResourceHost {
    pub fn new(
        identity: u64,
        path: PathBuf,
        appearance: Appearance,
        raster_scale: f32,
    ) -> Result<Self, ShellError> {
        let helper = std::env::current_exe()
            .map_err(|error| ShellError::Platform(error.to_string()))?
            .with_file_name("yu-document-renderer.exe");
        Self::with_helper(identity, path, appearance, raster_scale, helper)
    }

    pub(crate) fn with_helper(
        identity: u64,
        path: PathBuf,
        appearance: Appearance,
        raster_scale: f32,
        helper: PathBuf,
    ) -> Result<Self, ShellError> {
        let (jobs, input) = mpsc::channel();
        let (output, results) = mpsc::channel();
        let control = RenderControl::default();
        let worker_control = control.clone();
        std::thread::Builder::new()
            .name("yu-windows-resources".into())
            .spawn(move || {
                use windows::Win32::System::Com::{
                    COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
                };
                let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() };
                let mut rasterizer = com
                    .as_ref()
                    .map_err(|error| error.to_string())
                    .and_then(|()| ResourceRasterizer::new());
                let mut renderer = NativeRendererClient::new(helper, identity);
                while let Ok(job) = input.recv() {
                    let completed = match job {
                        Job::Image(request, path) => {
                            let result = if worker_control.is_current(request.revision().get()) {
                                match &mut rasterizer {
                                    Ok(decoder) => {
                                        decoder.decode_local(&path, request.max_pixel_dimension())
                                    }
                                    Err(error) => Err(error.clone()),
                                }
                            } else {
                                Err("Obsolete image request".into())
                            };
                            Completed::Image(request, result)
                        }
                        Job::Embedded(request, cached) => {
                            let result = if worker_control.is_current(request.revision().get()) {
                                cached
                                    .map(Ok)
                                    .unwrap_or_else(|| renderer.render(&request, &worker_control))
                                    .and_then(|payload| {
                                        let image = match &mut rasterizer {
                                            Ok(decoder) => decoder.raster_embedded(
                                                &payload,
                                                MAX_DECODE_DIMENSION,
                                                raster_scale,
                                            ),
                                            Err(error) => Err(error.clone()),
                                        }
                                        .map_err(RenderFailure::Worker)?;
                                        Ok((payload, image))
                                    })
                            } else {
                                Err(RenderFailure::Cancelled)
                            };
                            Completed::Embedded(request, result)
                        }
                    };
                    if output.send(completed).is_err() {
                        break;
                    }
                }
                drop(renderer);
                drop(rasterizer);
                if com.is_ok() {
                    unsafe {
                        CoUninitialize();
                    }
                }
            })
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        let mut images = ImageCache::with_capacity(32);
        images
            .set_byte_capacity(64 * 1024 * 1024)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        Ok(Self {
            contrast: None,
            jobs,
            results,
            control,
            in_flight: HashMap::new(),
            images,
            embedded: EmbeddedResourceCache::with_capacity(32),
            embedded_pixels: HashMap::new(),
            ready_embedded: Vec::new(),
            image_occurrences: Vec::new(),
            identity,
            path,
            appearance,
            revision: None,
            failures: 0,
            stale_results: 0,
            failed_ranges: Vec::new(),
        })
    }

    pub fn has_work(&self) -> bool {
        !self.in_flight.is_empty()
    }

    pub fn diagnostics(&self) -> (usize, usize, usize, u64, u64) {
        (
            self.images.len(),
            self.embedded_pixels.len(),
            self.in_flight.len(),
            self.failures,
            self.stale_results,
        )
    }

    pub fn advance(&mut self, revision: Revision) -> bool {
        let mut changed = false;
        self.control.set_revision(revision.get());
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.embedded.retain_revision(revision);
            self.ready_embedded.clear();
            self.image_occurrences.clear();
            self.failed_ranges.clear();
        }
        while let Ok(completed) = self.results.try_recv() {
            changed = true;
            let (key, owner) = match &completed {
                Completed::Image(request, _) => {
                    (JobKey::Image(request.key().clone()), request.revision())
                }
                Completed::Embedded(request, _) => {
                    (JobKey::Embedded(request.key().clone()), request.revision())
                }
            };
            if self.in_flight.get(&key) == Some(&owner) {
                self.in_flight.remove(&key);
            }
            if owner != revision {
                self.stale_results += 1;
                continue;
            }
            match completed {
                Completed::Image(request, result) => match result {
                    Ok(image) => {
                        let _ = self.images.publish_decoded(request, revision, image);
                    }
                    Err(error) => {
                        self.failures += 1;
                        eprintln!(
                            "yu-resource image={} error={error}",
                            request.key().destination()
                        );
                        let _ =
                            self.images
                                .record_failure(request, revision, ImageFailureKind::Decode);
                    }
                },
                Completed::Embedded(request, result) => match result {
                    Ok((payload, image)) => {
                        if let Ok(publication) = self.embedded.publish(request, revision, payload) {
                            self.trim_embedded_pixels(image.pixels().len());
                            self.embedded_pixels.insert(
                                publication.key().clone(),
                                (publication.generation(), image),
                            );
                            self.retain_embedded(publication);
                        }
                    }
                    Err(error) => {
                        self.failures += 1;
                        let kind = match error {
                            RenderFailure::InvalidSource(ref message) => {
                                eprintln!("yu-resource invalid={message}");
                                EmbeddedFailureKind::InvalidSource
                            }
                            RenderFailure::Worker(ref message) => {
                                eprintln!("yu-resource worker={message}");
                                EmbeddedFailureKind::Worker
                            }
                            RenderFailure::Cancelled => EmbeddedFailureKind::Worker,
                        };
                        let _ = self.embedded.record_failure(request, revision, kind);
                    }
                },
            }
        }
        changed
    }

    fn trim_embedded_pixels(&mut self, incoming: usize) {
        while self.embedded_pixels.len() >= 16
            || self
                .embedded_pixels
                .values()
                .map(|(_, image)| image.pixels().len())
                .sum::<usize>()
                + incoming
                > EMBEDDED_PIXEL_BUDGET
        {
            let Some(key) = self
                .embedded_pixels
                .iter()
                .min_by_key(|(_, (generation, _))| *generation)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.embedded_pixels.remove(&key);
        }
    }

    fn retain_embedded(&mut self, publication: EmbeddedRenderPublication) {
        self.ready_embedded
            .retain(|old| old.source_range() != publication.source_range());
        self.ready_embedded.push(publication);
        // Bound retained geometry/vector occurrences as well as raster bytes.
        while self.ready_embedded.len() > 64
            || self
                .ready_embedded
                .iter()
                .map(|p| p.payload().markup().map_or(0, str::len))
                .sum::<usize>()
                > 32 * 1024 * 1024
        {
            self.ready_embedded.remove(0);
        }
    }

    fn queue(&mut self, key: JobKey, revision: Revision, job: Job) -> bool {
        if self.in_flight.contains_key(&key) || self.in_flight.len() >= MAX_IN_FLIGHT {
            return false;
        }
        if self.jobs.send(job).is_ok() {
            self.in_flight.insert(key, revision);
            true
        } else {
            false
        }
    }

    pub fn prepare(
        &mut self,
        document: &mut LayoutContext,
        builder: &ViewportFrameBuilder<DirectWriteShaper>,
    ) -> Result<(Vec<ImagePublication>, Vec<ImageIntrinsicPublication>), ShellError> {
        let revision = document.revision();
        self.advance(revision);
        self.apply_geometry(document)?;
        let prior_intrinsics: Vec<_> = self
            .image_occurrences
            .iter()
            .filter_map(|request| self.images.intrinsic_publication(request))
            .collect();
        let blocks = builder
            .viewport_image_blocks_with_images_and_intrinsics(document, &[], &prior_intrinsics)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        let snapshot = document.snapshot();
        let definitions = document.markdown().reference_definitions().clone();
        let mut candidates = Vec::new();
        let mut requests = Vec::new();
        let mut failed = self.failed_ranges.clone();
        let theme = self.appearance.theme_id();
        for (index, priority) in blocks {
            let decorations = document
                .block_decorations(index)
                .map_err(|error| ShellError::Platform(error.to_string()))?
                .clone();
            let heading = decorations
                .line_styles()
                .iter()
                .find_map(|ornament| match ornament {
                    yu_editor::BlockOrnament::Heading { level } => Some(*level),
                    _ => None,
                });
            let quote = decorations
                .line_styles()
                .iter()
                .any(|o| matches!(o, yu_editor::BlockOrnament::QuoteBar { .. }));
            let (size, color) = if let Some(level) = heading {
                (
                    16.0 * theme.spec().heading_sizes[usize::from(level - 1)],
                    theme.heading_color(level),
                )
            } else if quote {
                (16.0, theme.quote_text_color())
            } else {
                (16.0, theme.spec().text)
            };
            let style = EmbeddedStyle::new(
                size,
                self.contrast.map_or(color, |c| c.foreground.packed()),
                matches!(self.appearance, Appearance::Dark | Appearance::YuDark),
            )
            .ok_or_else(|| ShellError::Platform("Invalid embedded style".into()))?;
            for widget in decorations.widgets() {
                match widget {
                    BlockWidget::Image(image) => {
                        if let Some(destination) =
                            yu_editor::image_destination_text(&snapshot, *image, &definitions)
                        {
                            let request = ImageRequest::new(revision, image.source(), destination)
                                .map_err(|error| ShellError::Platform(error.to_string()))?
                                .with_max_pixel_dimension(MAX_DECODE_DIMENSION);
                            candidates.push(ImageRequestCandidate::new(request, index, priority));
                        }
                    }
                    BlockWidget::Embedded(resource) => {
                        let kind = match resource.kind {
                            yu_markdown::EmbeddedKind::Math => EmbeddedResourceKind::Math,
                            yu_markdown::EmbeddedKind::Mermaid => EmbeddedResourceKind::Mermaid,
                        };
                        let content = if kind == EmbeddedResourceKind::Math {
                            document.markdown().equation_source(*resource)
                        } else {
                            document
                                .markdown()
                                .embedded_source(*resource)
                                .ok_or_else(|| "Missing embedded source".to_owned())
                        };
                        match content {
                            Ok(content) => {
                                let content = if content.is_empty() {
                                    "\n".into()
                                } else {
                                    content
                                };
                                requests.push(
                                    EmbeddedRenderRequest::new(
                                        revision,
                                        resource.source,
                                        kind,
                                        content,
                                    )
                                    .map_err(|error| ShellError::Platform(error.to_string()))?
                                    .with_style(style.with_display(resource.display)),
                                );
                            }
                            Err(_) => failed.push(resource.source),
                        }
                    }
                    BlockWidget::Checkbox(_) => {}
                }
            }
        }
        let mut images = Vec::new();
        for request in ImageRequestPlan::from_candidates(candidates).into_requests() {
            self.image_occurrences
                .retain(|old| old.source() != request.source());
            self.image_occurrences.push(request.clone());
            if self.image_occurrences.len() > 256 {
                self.image_occurrences.remove(0);
            }
            let key = JobKey::Image(request.key().clone());
            if self.in_flight.contains_key(&key) {
                continue;
            }
            match self.images.request(request.clone()) {
                ImageRequestResult::Ready(publication) => images.push(publication),
                ImageRequestResult::Failed(_) => failed.push(request.source()),
                ImageRequestResult::Pending => {}
            }
        }
        while self.in_flight.len() < MAX_IN_FLIGHT {
            let Some(request) = self.images.pending() else {
                break;
            };
            if request.revision() != revision {
                continue;
            }
            if self
                .in_flight
                .contains_key(&JobKey::Image(request.key().clone()))
            {
                continue;
            }
            match ImageLocation::resolve(&self.path, request.key().destination()) {
                Ok(location) => {
                    self.queue(
                        JobKey::Image(request.key().clone()),
                        revision,
                        Job::Image(request, location.path().into()),
                    );
                }
                Err(_) => {
                    let _ = self.images.record_failure(
                        request.clone(),
                        revision,
                        ImageFailureKind::Decode,
                    );
                    failed.push(request.source());
                }
            }
        }
        for request in requests {
            let key = JobKey::Embedded(request.key().clone());
            if self.in_flight.contains_key(&key) {
                continue;
            }
            match self.embedded.request(request.clone()) {
                EmbeddedRequestResult::Ready(publication) => {
                    if self
                        .embedded_pixels
                        .get(publication.key())
                        .is_some_and(|(generation, _)| *generation == publication.generation())
                    {
                        self.retain_embedded(publication);
                    } else {
                        self.queue(
                            key,
                            revision,
                            Job::Embedded(request, Some(publication.payload().clone())),
                        );
                    }
                }
                EmbeddedRequestResult::Failed(_) => failed.push(request.source_range()),
                EmbeddedRequestResult::Pending => {}
            }
        }
        while self.in_flight.len() < MAX_IN_FLIGHT {
            let Some(request) = self.embedded.pending() else {
                break;
            };
            self.queue(
                JobKey::Embedded(request.key().clone()),
                revision,
                Job::Embedded(request, None),
            );
        }
        self.apply_geometry(document)?;
        failed.sort_unstable_by_key(|range| (range.start(), range.end()));
        failed.dedup();
        self.failed_ranges = failed.clone();
        document
            .set_failed_resource_ranges(revision, failed)
            .map_err(|error| ShellError::Platform(error.to_string()))?;
        let intrinsics = self
            .image_occurrences
            .iter()
            .filter_map(|request| self.images.intrinsic_publication(request))
            .collect();
        Ok((images, intrinsics))
    }

    fn apply_geometry(&self, document: &mut LayoutContext) -> Result<(), ShellError> {
        let sizes = self
            .ready_embedded
            .iter()
            .filter_map(|publication| {
                let dimensions = publication.payload().dimensions();
                ImageIntrinsicSize::new(dimensions.width(), dimensions.height())
                    .and_then(|size| size.with_baseline(publication.payload().baseline_milli()))
                    .ok()
                    .map(|size| (publication.source_range(), size))
            })
            .collect();
        document
            .set_embedded_sizes(document.revision(), sizes)
            .map_err(|error| ShellError::Platform(error.to_string()))
    }

    pub fn embedded_publications(&self) -> Vec<EmbeddedRenderPublication> {
        self.ready_embedded.clone()
    }

    pub fn upload(
        &self,
        renderer: &mut D3DRenderer,
        images: &[ImagePublication],
    ) -> Result<(), D3DRenderError> {
        let mut keys = Vec::new();
        for publication in images {
            let resource = publication.key().fingerprint();
            renderer.upload_image(
                resource,
                yu_render::IMAGE_KIND_REGULAR,
                publication.image(),
                publication.generation(),
            )?;
            keys.push((resource, yu_render::IMAGE_KIND_REGULAR));
        }
        for publication in &self.ready_embedded {
            if let Some((generation, image)) = self.embedded_pixels.get(publication.key())
                && *generation == publication.generation()
            {
                let key = (
                    publication.key().fingerprint(),
                    u32::from(publication.kind().tag()) + 1,
                );
                renderer.upload_image(key.0, key.1, image, *generation)?;
                keys.push(key);
            }
        }
        renderer.retain_images(&keys);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use yu_editor::{EditorDocument, LayoutConfig, ViewportConfig, ViewportSpan};
    use yu_font::{FontRequest, GlyphAtlasConfig};
    use yu_scene::Rect;
    use yu_workspace::ViewportRenderConfig;

    fn builder() -> ViewportFrameBuilder<DirectWriteShaper> {
        let config = ViewportRenderConfig::new(
            ViewportSpan::new(0.0, 400.0),
            16.0,
            Rect::new(0.0, 0.0, 640.0, 400.0).expect("valid native resource test"),
            Appearance::Light.text(),
        );
        ViewportFrameBuilder::with_shaper(
            DirectWriteShaper::new(
                FontRequest::new("Segoe UI", 16.0).expect("valid native resource test"),
            )
            .expect("valid native resource test"),
            config,
            GlyphAtlasConfig::default(),
        )
        .expect("valid native resource test")
    }

    #[test]
    fn stale_worker_results_cannot_publish_into_a_new_revision() {
        let mut host = ResourceHost::with_helper(
            1,
            PathBuf::from("C:/missing/test.md"),
            Appearance::Light,
            1.0,
            PathBuf::from("missing-renderer.exe"),
        )
        .expect("valid native resource test");
        let old = Revision::INITIAL;
        let current = Revision::new(old.get() + 1);
        let range = TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(1))
            .expect("valid native resource test");
        let request =
            ImageRequest::new(old, range, "missing.png").expect("valid native resource test");
        host.advance(old);
        assert!(host.queue(
            JobKey::Image(request.key().clone()),
            old,
            Job::Image(request, PathBuf::from("C:/missing/does-not-exist.png"))
        ));
        host.advance(current);
        let deadline = Instant::now() + Duration::from_secs(3);
        while host.has_work() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
            host.advance(current);
        }
        assert_eq!(host.stale_results, 1);
        assert_eq!(host.images.len(), 0);
        assert_eq!(host.images.failure_count(), 0);
    }

    #[test]
    fn missing_helper_settles_to_source_without_repeated_jobs_or_source_edits() {
        let mut host = ResourceHost::with_helper(
            2,
            PathBuf::from("C:/missing/test.md"),
            Appearance::Light,
            1.0,
            PathBuf::from("C:/missing/yu-renderer-does-not-exist.exe"),
        )
        .expect("valid native resource test");
        let mut editor = EditorDocument::new("prefix $x^2$ suffix");
        editor
            .set_viewport_config(ViewportConfig::new(
                LayoutConfig::new(640.0, 25.6),
                25.6,
                0.0,
            ))
            .expect("valid native resource test");
        let builder = builder();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            host.prepare(&mut editor, &builder)
                .expect("valid native resource test");
            if !host.has_work() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(host.failures, 1);
        assert_eq!(host.failed_ranges.len(), 1);
        for _ in 0..10 {
            host.prepare(&mut editor, &builder)
                .expect("valid native resource test");
        }
        assert!(!host.has_work());
        assert_eq!(host.failures, 1);
        assert_eq!(host.failed_ranges.len(), 1);
        assert_eq!(editor.snapshot().as_str(), "prefix $x^2$ suffix");
        assert_eq!(editor.revision(), Revision::INITIAL);
    }
}
