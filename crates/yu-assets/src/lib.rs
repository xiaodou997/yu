#![forbid(unsafe_code)]

//! Source-independent image resource preparation.
//!
//! `yu-assets` is deliberately a small handoff contract rather than an image
//! decoder. The editor supplies a Revision-bound destination range through an
//! [`ImageRequest`]; a platform worker can decode it off-thread and publish
//! owned RGBA bytes back through [`ImageCache`]. GPU texture handles never
//! enter this crate.

use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use yu_core::{Revision, TextRange};

/// Stable resource identity for one destination string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey(Arc<str>);

impl ImageKey {
    pub fn new(destination: impl Into<Arc<str>>) -> Result<Self, ImageKeyError> {
        let destination = destination.into();
        if destination.is_empty() {
            return Err(ImageKeyError::EmptyDestination);
        }
        Ok(Self(destination))
    }

    #[must_use]
    pub fn destination(&self) -> &str {
        &self.0
    }

    /// Returns a process-independent fingerprint for diagnostics and native
    /// cache keys. The destination itself remains the collision-free key.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut hash = 1_469_598_103_934_665_603_u64;
        for byte in self.0.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(1_099_511_628_211_u64);
        }
        hash
    }
}

/// A filesystem location resolved from a Markdown image destination.
///
/// Resolution is deliberately kept outside the parser and editor model. The
/// destination remains the source-backed [`ImageKey`], while a platform host
/// supplies the document path used to turn relative destinations into an
/// absolute filesystem location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageLocation(PathBuf);

impl ImageLocation {
    /// Resolves a local image destination against the directory containing the
    /// Markdown document. Remote URLs and data URLs are rejected explicitly;
    /// they need a separate, policy-controlled loader and must not silently
    /// become filesystem paths.
    pub fn resolve(
        document_path: impl AsRef<Path>,
        destination: &str,
    ) -> Result<Self, ImageLocationError> {
        let destination = destination.trim();
        if destination.is_empty() {
            return Err(ImageLocationError::EmptyDestination);
        }
        if destination.contains("://") || destination.starts_with("data:") {
            return Err(ImageLocationError::UnsupportedScheme);
        }
        let path = Path::new(destination);
        if path.is_absolute() {
            return Ok(Self(path.to_path_buf()));
        }
        let document_path = document_path.as_ref();
        let parent = document_path
            .parent()
            .ok_or(ImageLocationError::MissingDocumentParent)?;
        Ok(Self(parent.join(path)))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// Errors raised while resolving an image destination for a local decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageLocationError {
    EmptyDestination,
    UnsupportedScheme,
    MissingDocumentParent,
}

impl fmt::Display for ImageLocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDestination => formatter.write_str("image destination must not be empty"),
            Self::UnsupportedScheme => {
                formatter.write_str("remote and data image destinations are unsupported")
            }
            Self::MissingDocumentParent => {
                formatter.write_str("document path has no parent directory")
            }
        }
    }
}

impl Error for ImageLocationError {}

/// Errors raised while creating an image resource key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKeyError {
    EmptyDestination,
}

impl fmt::Display for ImageKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDestination => formatter.write_str("image destination must not be empty"),
        }
    }
}

impl Error for ImageKeyError {}

/// A source-backed request handed to an asynchronous decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRequest {
    revision: Revision,
    source: TextRange,
    key: ImageKey,
}

impl ImageRequest {
    pub fn new(
        revision: Revision,
        source: TextRange,
        destination: impl Into<Arc<str>>,
    ) -> Result<Self, ImageKeyError> {
        Ok(Self {
            revision,
            source,
            key: ImageKey::new(destination)?,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub fn key(&self) -> &ImageKey {
        &self.key
    }
}

/// Owned decoded RGBA8 image data ready for a platform texture uploader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl DecodedImage {
    pub fn new(
        width: u32,
        height: u32,
        pixels: impl Into<Arc<[u8]>>,
    ) -> Result<Self, ImageDecodeError> {
        if width == 0 || height == 0 {
            return Err(ImageDecodeError::InvalidDimensions);
        }
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ImageDecodeError::PixelBufferOverflow)?;
        let pixels = pixels.into();
        if pixels.len() != expected {
            return Err(ImageDecodeError::PixelLength {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Errors raised while validating decoded image bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageDecodeError {
    InvalidDimensions,
    PixelBufferOverflow,
    PixelLength { expected: usize, actual: usize },
}

impl fmt::Display for ImageDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => formatter.write_str("image dimensions must be positive"),
            Self::PixelBufferOverflow => formatter.write_str("image pixel buffer size overflowed"),
            Self::PixelLength { expected, actual } => {
                write!(
                    formatter,
                    "image pixel buffer has {actual} bytes, expected {expected}"
                )
            }
        }
    }
}

impl Error for ImageDecodeError {}

/// A decoded image publication bound to the request's source Revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagePublication {
    revision: Revision,
    generation: u64,
    source: TextRange,
    key: ImageKey,
    image: DecodedImage,
}

impl ImagePublication {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn source(&self) -> TextRange {
        self.source
    }

    #[must_use]
    pub fn key(&self) -> &ImageKey {
        &self.key
    }

    #[must_use]
    pub const fn image(&self) -> &DecodedImage {
        &self.image
    }
}

/// Result of requesting a resource from the cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageRequestResult {
    Ready(ImagePublication),
    Pending,
}

/// Errors raised while publishing decoded image data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageCacheError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    Decode(ImageDecodeError),
    GenerationOverflow,
}

impl fmt::Display for ImageCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => {
                write!(
                    formatter,
                    "image publication {actual:?} is stale for {expected:?}"
                )
            }
            Self::Decode(error) => error.fmt(formatter),
            Self::GenerationOverflow => formatter.write_str("image generation overflowed"),
        }
    }
}

impl Error for ImageCacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::StaleRevision { .. } | Self::GenerationOverflow => None,
        }
    }
}

impl From<ImageDecodeError> for ImageCacheError {
    fn from(error: ImageDecodeError) -> Self {
        Self::Decode(error)
    }
}

#[derive(Clone, Debug)]
struct CacheEntry {
    generation: u64,
    image: DecodedImage,
}

/// Revision-aware decoded image cache with a pollable asynchronous work queue.
///
/// The cache never starts threads or touches a filesystem. A platform worker
/// calls [`Self::pending`] off the editor thread, decodes the request, and
/// calls [`Self::publish_decoded`] on the owner thread. A cached image can be
/// rebound to a newer Revision without decoding again, while a publication
/// for an old Revision is rejected before it can reach a texture uploader.
#[derive(Clone, Debug, Default)]
pub struct ImageCache {
    entries: HashMap<ImageKey, CacheEntry>,
    pending: VecDeque<ImageRequest>,
    pending_keys: HashSet<ImageKey>,
    next_generation: u64,
}

impl ImageCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&mut self, request: ImageRequest) -> ImageRequestResult {
        if let Some(entry) = self.entries.get(request.key()) {
            return ImageRequestResult::Ready(ImagePublication {
                revision: request.revision,
                generation: entry.generation,
                source: request.source,
                key: request.key.clone(),
                image: entry.image.clone(),
            });
        }
        let key = request.key.clone();
        if self.pending_keys.contains(&key) {
            if let Some(queued) = self.pending.iter_mut().find(|queued| queued.key == key) {
                *queued = request;
            }
        } else {
            self.pending_keys.insert(key);
            self.pending.push_back(request);
        }
        ImageRequestResult::Pending
    }

    /// Pops one decode request. The caller may process it on a worker thread
    /// but must publish the result through the owning cache afterwards.
    pub fn pending(&mut self) -> Option<ImageRequest> {
        let request = self.pending.pop_front()?;
        self.pending_keys.remove(request.key());
        Some(request)
    }

    pub fn publish_decoded(
        &mut self,
        request: ImageRequest,
        current_revision: Revision,
        image: DecodedImage,
    ) -> Result<ImagePublication, ImageCacheError> {
        if request.revision != current_revision {
            return Err(ImageCacheError::StaleRevision {
                expected: current_revision,
                actual: request.revision,
            });
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(ImageCacheError::GenerationOverflow)?;
        let generation = self.next_generation;
        self.entries.insert(
            request.key.clone(),
            CacheEntry {
                generation,
                image: image.clone(),
            },
        );
        Ok(ImagePublication {
            revision: current_revision,
            generation,
            source: request.source,
            key: request.key,
            image,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.pending.clear();
        self.pending_keys.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_locations_resolve_from_document_parent() {
        let location = ImageLocation::resolve("/tmp/notes/readme.md", "../img/yu.png")
            .expect("local image location");
        assert_eq!(location.path(), Path::new("/tmp/notes/../img/yu.png"));
    }

    #[test]
    fn absolute_locations_are_preserved() {
        let location = ImageLocation::resolve("/tmp/notes/readme.md", "/tmp/yu.png")
            .expect("absolute image location");
        assert_eq!(location.path(), Path::new("/tmp/yu.png"));
    }

    #[test]
    fn remote_and_data_locations_are_rejected() {
        assert_eq!(
            ImageLocation::resolve("/tmp/readme.md", "https://example.com/yu.png")
                .expect_err("remote image"),
            ImageLocationError::UnsupportedScheme
        );
        assert_eq!(
            ImageLocation::resolve("/tmp/readme.md", "data:image/png;base64,AA==")
                .expect_err("data image"),
            ImageLocationError::UnsupportedScheme
        );
    }

    fn request(revision: u64, source: TextRange, destination: &str) -> ImageRequest {
        ImageRequest::new(Revision::new(revision), source, destination).expect("request")
    }

    fn decoded(byte: u8) -> DecodedImage {
        DecodedImage::new(2, 1, vec![byte; 8]).expect("image")
    }

    #[test]
    fn pending_requests_are_deduplicated_by_destination() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let mut cache = ImageCache::new();
        assert_eq!(
            cache.request(request(0, source, "img.png")),
            ImageRequestResult::Pending
        );
        assert_eq!(
            cache.request(request(0, source, "img.png")),
            ImageRequestResult::Pending
        );
        assert_eq!(
            cache.pending().expect("pending").key().destination(),
            "img.png"
        );
        assert!(cache.pending().is_none());
    }

    #[test]
    fn newer_revision_replaces_a_queued_request_before_decode() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let mut cache = ImageCache::new();
        assert_eq!(
            cache.request(request(1, source, "img.png")),
            ImageRequestResult::Pending
        );
        assert_eq!(
            cache.request(request(2, source, "img.png")),
            ImageRequestResult::Pending
        );
        assert_eq!(
            cache.pending().expect("pending").revision(),
            Revision::new(2)
        );
        assert!(cache.pending().is_none());
    }

    #[test]
    fn stale_decoded_publication_is_rejected_without_cache_mutation() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let mut cache = ImageCache::new();
        let request = request(1, source, "img.png");
        let error = cache
            .publish_decoded(request, Revision::new(2), decoded(1))
            .expect_err("old revision must be rejected");
        assert_eq!(
            error,
            ImageCacheError::StaleRevision {
                expected: Revision::new(2),
                actual: Revision::new(1),
            }
        );
        assert!(cache.is_empty());
    }

    #[test]
    fn decoded_data_is_reused_and_rebound_to_a_new_revision() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let mut cache = ImageCache::new();
        let first_request = request(1, source, "img.png");
        let publication = cache
            .publish_decoded(first_request, Revision::new(1), decoded(7))
            .expect("publication");
        assert_eq!(publication.generation(), 1);
        let rebound = cache.request(request(2, source, "img.png"));
        let ImageRequestResult::Ready(rebound) = rebound else {
            panic!("cached image should be ready");
        };
        assert_eq!(rebound.revision(), Revision::new(2));
        assert_eq!(rebound.generation(), publication.generation());
        assert_eq!(rebound.image().pixels(), publication.image().pixels());
        assert_eq!(cache.len(), 1);
    }
}
