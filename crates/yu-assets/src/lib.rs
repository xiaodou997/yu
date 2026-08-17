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

/// Scheduling priority for an image candidate selected by a viewport query.
/// Visible candidates are always ordered before overscan candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImageRequestPriority {
    Visible,
    Overscan,
}

/// One source-backed image occurrence discovered in a viewport block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRequestCandidate {
    request: ImageRequest,
    block_index: usize,
    priority: ImageRequestPriority,
}

impl ImageRequestCandidate {
    #[must_use]
    pub const fn new(
        request: ImageRequest,
        block_index: usize,
        priority: ImageRequestPriority,
    ) -> Self {
        Self {
            request,
            block_index,
            priority,
        }
    }

    #[must_use]
    pub const fn request(&self) -> &ImageRequest {
        &self.request
    }

    #[must_use]
    pub const fn block_index(&self) -> usize {
        self.block_index
    }

    #[must_use]
    pub const fn priority(&self) -> ImageRequestPriority {
        self.priority
    }
}

/// Counts collected while converting image occurrences into unique work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageScheduleStats {
    candidate_count: usize,
    unique_count: usize,
    duplicate_count: usize,
    visible_candidate_count: usize,
    overscan_candidate_count: usize,
}

impl ImageScheduleStats {
    #[must_use]
    pub const fn candidate_count(self) -> usize {
        self.candidate_count
    }

    #[must_use]
    pub const fn unique_count(self) -> usize {
        self.unique_count
    }

    #[must_use]
    pub const fn duplicate_count(self) -> usize {
        self.duplicate_count
    }

    #[must_use]
    pub const fn visible_candidate_count(self) -> usize {
        self.visible_candidate_count
    }

    #[must_use]
    pub const fn overscan_candidate_count(self) -> usize {
        self.overscan_candidate_count
    }
}

/// Deterministic, deduplicated image work selected for one viewport batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRequestPlan {
    requests: Vec<ImageRequest>,
    stats: ImageScheduleStats,
}

impl ImageRequestPlan {
    /// Deduplicates by destination, keeps the most urgent occurrence, and
    /// orders visible work before overscan work. A lower block index wins when
    /// two occurrences have the same priority, making queue order stable.
    #[must_use]
    pub fn from_candidates(candidates: impl IntoIterator<Item = ImageRequestCandidate>) -> Self {
        let mut stats = ImageScheduleStats::default();
        let mut unique = HashMap::<ImageKey, ImageRequestCandidate>::new();
        for candidate in candidates {
            stats.candidate_count = stats.candidate_count.saturating_add(1);
            match candidate.priority() {
                ImageRequestPriority::Visible => {
                    stats.visible_candidate_count = stats.visible_candidate_count.saturating_add(1);
                }
                ImageRequestPriority::Overscan => {
                    stats.overscan_candidate_count =
                        stats.overscan_candidate_count.saturating_add(1);
                }
            }
            let key = candidate.request().key().clone();
            if let Some(previous) = unique.get_mut(&key) {
                stats.duplicate_count = stats.duplicate_count.saturating_add(1);
                if candidate.priority() < previous.priority()
                    || (candidate.priority() == previous.priority()
                        && candidate.block_index() < previous.block_index())
                {
                    *previous = candidate;
                }
            } else {
                unique.insert(key, candidate);
            }
        }

        let mut selected = unique.into_values().collect::<Vec<_>>();
        selected.sort_by_key(|candidate| {
            (
                candidate.priority(),
                candidate.block_index(),
                candidate.request().key().fingerprint(),
            )
        });
        stats.unique_count = selected.len();
        let requests = selected
            .into_iter()
            .map(|candidate| candidate.request)
            .collect();
        Self { requests, stats }
    }

    #[must_use]
    pub fn requests(&self) -> &[ImageRequest] {
        &self.requests
    }

    #[must_use]
    pub fn stats(&self) -> ImageScheduleStats {
        self.stats
    }

    #[must_use]
    pub fn into_requests(self) -> Vec<ImageRequest> {
        self.requests
    }
}

/// Owned decoded RGBA8 image data ready for a platform texture uploader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

/// Intrinsic dimensions retained independently from decoded pixel ownership.
///
/// A platform may evict CPU pixels or GPU textures while the editor still
/// needs the image's aspect ratio to keep its HeightIndex stable. Keeping this
/// small value separate from [`DecodedImage`] lets those resource policies
/// remain bounded without making layout jump back to the placeholder size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageDimensions {
    width: u32,
    height: u32,
}

impl ImageDimensions {
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            None
        } else {
            Some(Self { width, height })
        }
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

impl DecodedImage {
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        // DecodedImage validates dimensions during construction, so this is
        // infallible and keeps the metadata handoff allocation-free.
        ImageDimensions {
            width: self.width,
            height: self.height,
        }
    }
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

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.image.dimensions()
    }

    #[must_use]
    pub fn intrinsic_publication(&self) -> ImageIntrinsicPublication {
        ImageIntrinsicPublication {
            revision: self.revision,
            key: self.key.clone(),
            dimensions: self.dimensions(),
        }
    }
}

/// A revision-bound intrinsic-size publication without decoded pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageIntrinsicPublication {
    revision: Revision,
    key: ImageKey,
    dimensions: ImageDimensions,
}

impl ImageIntrinsicPublication {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn key(&self) -> &ImageKey {
        &self.key
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
}

/// Stable reason class recorded when a platform decoder cannot publish an
/// image. Platform-specific errors remain outside this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFailureKind {
    Decode,
    Unsupported,
    Io,
    Worker,
}

/// Revision-bound failure metadata for one image destination.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageFailure {
    revision: Revision,
    key: ImageKey,
    kind: ImageFailureKind,
    attempts: u32,
    next_retry_tick: u64,
}

impl ImageFailure {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn key(&self) -> &ImageKey {
        &self.key
    }

    #[must_use]
    pub const fn kind(&self) -> ImageFailureKind {
        self.kind
    }

    #[must_use]
    pub const fn attempts(&self) -> u32 {
        self.attempts
    }

    #[must_use]
    pub const fn next_retry_tick(&self) -> u64 {
        self.next_retry_tick
    }

    #[must_use]
    pub const fn is_exhausted(&self, policy: ImageRetryPolicy) -> bool {
        self.attempts >= policy.max_attempts()
    }
}

/// Bounded retry policy for transient image decode failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageRetryPolicy {
    max_attempts: u32,
    base_delay_ticks: u64,
    max_delay_ticks: u64,
}

impl ImageRetryPolicy {
    #[must_use]
    pub const fn new(max_attempts: u32, base_delay_ticks: u64, max_delay_ticks: u64) -> Self {
        Self {
            max_attempts: if max_attempts == 0 { 1 } else { max_attempts },
            base_delay_ticks,
            max_delay_ticks: if max_delay_ticks < base_delay_ticks {
                base_delay_ticks
            } else {
                max_delay_ticks
            },
        }
    }

    #[must_use]
    pub const fn max_attempts(self) -> u32 {
        self.max_attempts
    }

    #[must_use]
    pub const fn base_delay_ticks(self) -> u64 {
        self.base_delay_ticks
    }

    #[must_use]
    pub const fn max_delay_ticks(self) -> u64 {
        self.max_delay_ticks
    }

    fn delay_for_attempt(self, attempts: u32) -> u64 {
        let shift = attempts.saturating_sub(1).min(63);
        self.base_delay_ticks
            .saturating_mul(1_u64 << shift)
            .min(self.max_delay_ticks)
    }
}

impl Default for ImageRetryPolicy {
    fn default() -> Self {
        Self::new(3, 2, 60)
    }
}

/// Result of requesting a resource from the cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageRequestResult {
    Ready(ImagePublication),
    Pending,
    Failed(ImageFailure),
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
    InvalidCapacity,
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
            Self::InvalidCapacity => formatter.write_str("image cache capacity must be positive"),
        }
    }
}

impl Error for ImageCacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::StaleRevision { .. } | Self::GenerationOverflow | Self::InvalidCapacity => None,
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
    last_used: u64,
}

#[derive(Clone, Copy, Debug)]
struct MetadataEntry {
    dimensions: ImageDimensions,
    last_used: u64,
}

/// Revision-aware decoded image cache with a pollable asynchronous work queue.
///
/// The cache never starts threads or touches a filesystem. A platform worker
/// calls [`Self::pending`] off the editor thread, decodes the request, and
/// calls [`Self::publish_decoded`] on the owner thread. A cached image can be
/// rebound to a newer Revision without decoding again, while a publication
/// for an old Revision is rejected before it can reach a texture uploader.
#[derive(Clone, Debug)]
pub struct ImageCache {
    entries: HashMap<ImageKey, CacheEntry>,
    metadata: HashMap<ImageKey, MetadataEntry>,
    failures: HashMap<ImageKey, ImageFailure>,
    pending: VecDeque<ImageRequest>,
    pending_keys: HashSet<ImageKey>,
    next_generation: u64,
    next_access: u64,
    capacity: usize,
    evictions: u64,
    metadata_capacity: usize,
    metadata_evictions: u64,
    retry_policy: ImageRetryPolicy,
    retry_tick: u64,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }
}

impl ImageCache {
    pub const DEFAULT_CAPACITY: usize = 64;
    pub const DEFAULT_METADATA_CAPACITY: usize = 256;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            metadata: HashMap::new(),
            failures: HashMap::new(),
            pending: VecDeque::new(),
            pending_keys: HashSet::new(),
            next_generation: 0,
            next_access: 0,
            capacity: capacity.max(1),
            evictions: 0,
            metadata_capacity: capacity.max(Self::DEFAULT_METADATA_CAPACITY),
            metadata_evictions: 0,
            retry_policy: ImageRetryPolicy::default(),
            retry_tick: 0,
        }
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Changes the decoded-image capacity and returns the number of entries
    /// evicted while enforcing the new limit.
    pub fn set_capacity(&mut self, capacity: usize) -> Result<usize, ImageCacheError> {
        if capacity == 0 {
            return Err(ImageCacheError::InvalidCapacity);
        }
        self.capacity = capacity;
        Ok(self.trim_to_capacity())
    }

    #[must_use]
    pub fn metadata_capacity(&self) -> usize {
        self.metadata_capacity
    }

    /// Changes the bounded intrinsic metadata capacity independently of the
    /// decoded-pixel capacity.
    pub fn set_metadata_capacity(&mut self, capacity: usize) -> Result<usize, ImageCacheError> {
        if capacity == 0 {
            return Err(ImageCacheError::InvalidCapacity);
        }
        self.metadata_capacity = capacity;
        Ok(self.trim_metadata_to_capacity())
    }

    #[must_use]
    pub const fn retry_policy(&self) -> ImageRetryPolicy {
        self.retry_policy
    }

    pub fn set_retry_policy(&mut self, policy: ImageRetryPolicy) {
        self.retry_policy = policy;
    }

    /// Advances the logical frame clock used by the retry backoff policy.
    /// Hosts call this once before scheduling a new viewport batch.
    pub fn advance_retry_clock(&mut self) {
        self.retry_tick = self.retry_tick.saturating_add(1);
    }

    #[must_use]
    pub const fn retry_tick(&self) -> u64 {
        self.retry_tick
    }

    pub fn request(&mut self, request: ImageRequest) -> ImageRequestResult {
        if let Some(failure) = self.failures.get(request.key()) {
            if failure.revision == request.revision
                && (failure.is_exhausted(self.retry_policy)
                    || self.retry_tick < failure.next_retry_tick)
            {
                return ImageRequestResult::Failed(failure.clone());
            }
            if failure.revision != request.revision {
                self.failures.remove(request.key());
            }
        }
        let access_tick = self.next_access_tick();
        if let Some(entry) = self.entries.get_mut(request.key()) {
            entry.last_used = access_tick;
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
        let access_tick = self.next_access_tick();
        self.metadata.insert(
            request.key.clone(),
            MetadataEntry {
                dimensions: image.dimensions(),
                last_used: access_tick,
            },
        );
        self.trim_metadata_to_capacity();
        self.failures.remove(&request.key);
        self.entries.insert(
            request.key.clone(),
            CacheEntry {
                generation,
                image: image.clone(),
                last_used: access_tick,
            },
        );
        self.trim_to_capacity();
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
    pub fn metadata_len(&self) -> usize {
        self.metadata.len()
    }

    #[must_use]
    pub fn metadata_eviction_count(&self) -> u64 {
        self.metadata_evictions
    }

    /// Returns known intrinsic dimensions even if decoded pixels were evicted.
    /// The returned publication is rebound to the request's current Revision.
    pub fn intrinsic_publication(
        &mut self,
        request: &ImageRequest,
    ) -> Option<ImageIntrinsicPublication> {
        let access_tick = self.next_access_tick();
        let metadata = self.metadata.get_mut(request.key())?;
        metadata.last_used = access_tick;
        Some(ImageIntrinsicPublication {
            revision: request.revision,
            key: request.key.clone(),
            dimensions: metadata.dimensions,
        })
    }

    #[must_use]
    pub fn failure_count(&self) -> usize {
        self.failures.len()
    }

    #[must_use]
    pub fn failure(&self, key: &ImageKey) -> Option<&ImageFailure> {
        self.failures.get(key)
    }

    #[must_use]
    pub fn eviction_count(&self) -> u64 {
        self.evictions
    }

    /// Records a stable failure for the current Revision. A later Revision
    /// automatically clears the old failure when it requests the same key.
    pub fn record_failure(
        &mut self,
        request: ImageRequest,
        current_revision: Revision,
        kind: ImageFailureKind,
    ) -> Result<ImageFailure, ImageCacheError> {
        if request.revision != current_revision {
            return Err(ImageCacheError::StaleRevision {
                expected: current_revision,
                actual: request.revision,
            });
        }
        self.pending.retain(|pending| pending.key != request.key);
        self.pending_keys.remove(&request.key);
        let attempts = self
            .failures
            .get(&request.key)
            .filter(|failure| failure.revision == current_revision)
            .map_or(1, |failure| failure.attempts.saturating_add(1));
        let next_retry_tick = self
            .retry_tick
            .saturating_add(self.retry_policy.delay_for_attempt(attempts));
        let failure = ImageFailure {
            revision: current_revision,
            key: request.key.clone(),
            kind,
            attempts,
            next_retry_tick,
        };
        self.failures.insert(request.key, failure.clone());
        Ok(failure)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.metadata.clear();
        self.failures.clear();
        self.pending.clear();
        self.pending_keys.clear();
    }

    fn next_access_tick(&mut self) -> u64 {
        self.next_access = self.next_access.saturating_add(1);
        self.next_access
    }

    fn trim_to_capacity(&mut self) -> usize {
        let mut evicted = 0;
        while self.entries.len() > self.capacity {
            let Some(key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.entries.remove(&key);
            evicted += 1;
            self.evictions = self.evictions.saturating_add(1);
        }
        evicted
    }

    fn trim_metadata_to_capacity(&mut self) -> usize {
        let mut evicted = 0;
        while self.metadata.len() > self.metadata_capacity {
            let Some(key) = self
                .metadata
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.metadata.remove(&key);
            evicted += 1;
            self.metadata_evictions = self.metadata_evictions.saturating_add(1);
        }
        evicted
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

    #[test]
    fn image_request_plan_deduplicates_and_prioritizes_visible_work() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let plan = ImageRequestPlan::from_candidates([
            ImageRequestCandidate::new(
                request(0, source, "overscan.png"),
                20,
                ImageRequestPriority::Overscan,
            ),
            ImageRequestCandidate::new(
                request(0, source, "visible.png"),
                10,
                ImageRequestPriority::Visible,
            ),
            ImageRequestCandidate::new(
                request(0, source, "visible.png"),
                30,
                ImageRequestPriority::Overscan,
            ),
            ImageRequestCandidate::new(
                request(0, source, "overscan.png"),
                5,
                ImageRequestPriority::Visible,
            ),
        ]);

        assert_eq!(plan.stats().candidate_count(), 4);
        assert_eq!(plan.stats().unique_count(), 2);
        assert_eq!(plan.stats().duplicate_count(), 2);
        assert_eq!(plan.stats().visible_candidate_count(), 2);
        assert_eq!(plan.stats().overscan_candidate_count(), 2);
        assert_eq!(plan.requests().len(), 2);
        assert_eq!(plan.requests()[0].key().destination(), "overscan.png");
        assert_eq!(plan.requests()[1].key().destination(), "visible.png");
    }

    #[test]
    fn image_request_plan_keeps_lowest_block_for_equal_priority() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let plan = ImageRequestPlan::from_candidates([
            ImageRequestCandidate::new(
                request(0, source, "same.png"),
                9,
                ImageRequestPriority::Visible,
            ),
            ImageRequestCandidate::new(
                request(0, source, "same.png"),
                3,
                ImageRequestPriority::Visible,
            ),
        ]);
        assert_eq!(plan.requests()[0].source(), source);
        assert_eq!(plan.stats().duplicate_count(), 1);
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

    #[test]
    fn bounded_cache_evicts_the_least_recently_used_publication() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let mut cache = ImageCache::with_capacity(1);
        cache
            .publish_decoded(
                request(0, source, "first.png"),
                Revision::INITIAL,
                decoded(1),
            )
            .expect("first publication");
        cache
            .publish_decoded(
                request(0, source, "second.png"),
                Revision::INITIAL,
                decoded(2),
            )
            .expect("second publication");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.eviction_count(), 1);
        assert!(matches!(
            cache.request(request(0, source, "first.png")),
            ImageRequestResult::Pending
        ));
    }

    #[test]
    fn intrinsic_metadata_survives_pixel_eviction() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let mut cache = ImageCache::with_capacity(1);
        cache
            .publish_decoded(
                request(0, source, "first.png"),
                Revision::INITIAL,
                DecodedImage::new(320, 180, vec![1; 320 * 180 * 4]).expect("first image"),
            )
            .expect("first publication");
        cache
            .publish_decoded(
                request(0, source, "second.png"),
                Revision::INITIAL,
                DecodedImage::new(64, 32, vec![2; 64 * 32 * 4]).expect("second image"),
            )
            .expect("second publication");

        let first = request(4, source, "first.png");
        assert!(matches!(
            cache.request(first.clone()),
            ImageRequestResult::Pending
        ));
        let intrinsic = cache
            .intrinsic_publication(&first)
            .expect("metadata remains after pixel eviction");
        assert_eq!(intrinsic.revision(), Revision::new(4));
        assert_eq!(
            intrinsic.dimensions(),
            ImageDimensions::new(320, 180).expect("positive dimensions")
        );
        assert_eq!(cache.metadata_len(), 2);
    }

    #[test]
    fn failed_requests_retry_with_bounded_backoff() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let mut cache = ImageCache::new();
        cache.set_retry_policy(ImageRetryPolicy::new(2, 1, 1));
        let image_request = request(0, source, "transient.png");
        let first = cache
            .record_failure(
                image_request.clone(),
                Revision::INITIAL,
                ImageFailureKind::Io,
            )
            .expect("first failure");
        assert_eq!(first.attempts(), 1);
        assert_eq!(first.next_retry_tick(), 1);
        assert!(matches!(
            cache.request(image_request.clone()),
            ImageRequestResult::Failed(_)
        ));

        cache.advance_retry_clock();
        assert!(matches!(
            cache.request(image_request.clone()),
            ImageRequestResult::Pending
        ));
        let retry_request = cache.pending().expect("due retry");
        let second = cache
            .record_failure(retry_request, Revision::INITIAL, ImageFailureKind::Io)
            .expect("second failure");
        assert_eq!(second.attempts(), 2);
        assert!(second.is_exhausted(cache.retry_policy()));

        cache.advance_retry_clock();
        let ImageRequestResult::Failed(exhausted) = cache.request(image_request) else {
            panic!("exhausted retries must remain failed");
        };
        assert_eq!(exhausted.attempts(), 2);
    }

    #[test]
    fn failures_are_revision_bound_and_report_attempts() {
        let source =
            TextRange::new(yu_core::ByteOffset::ZERO, yu_core::ByteOffset::new(4)).expect("range");
        let mut cache = ImageCache::new();
        let failed = cache
            .record_failure(
                request(0, source, "broken.png"),
                Revision::INITIAL,
                ImageFailureKind::Decode,
            )
            .expect("failure");
        assert_eq!(failed.attempts(), 1);
        let ImageRequestResult::Failed(current) = cache.request(request(0, source, "broken.png"))
        else {
            panic!("same revision should expose the recorded failure");
        };
        assert_eq!(current.kind(), ImageFailureKind::Decode);
        assert_eq!(cache.failure_count(), 1);
        assert!(matches!(
            cache.request(request(1, source, "broken.png")),
            ImageRequestResult::Pending
        ));
        assert_eq!(cache.failure_count(), 0);
    }
}
