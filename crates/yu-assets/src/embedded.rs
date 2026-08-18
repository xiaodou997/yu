//! Revision-bound resource publication for embedded Markdown blocks.
//!
//! This module intentionally does not parse Math/Mermaid or own a renderer.
//! It provides the handoff between the editor's source-backed block discovery
//! and a replaceable worker implementation. The queue is pollable so a host
//! can run work on any executor or native worker without making the document
//! thread wait for rendering.

use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_core::{Revision, TextRange};

/// Embedded block kinds that have a visual renderer extension point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmbeddedResourceKind {
    Math,
    Mermaid,
}

impl EmbeddedResourceKind {
    /// Stable wire tag shared with native diagnostics.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Math => 0,
            Self::Mermaid => 1,
        }
    }

    /// Canonical language name used when constructing renderer requests.
    #[must_use]
    pub const fn language(self) -> &'static str {
        match self {
            Self::Math => "math",
            Self::Mermaid => "mermaid",
        }
    }
}

/// Errors raised while creating an embedded resource identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedResourceKeyError {
    EmptySource,
}

impl fmt::Display for EmbeddedResourceKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySource => formatter.write_str("embedded resource source must not be empty"),
        }
    }
}

impl Error for EmbeddedResourceKeyError {}

/// Stable identity for one renderer input.
///
/// The source is kept because a renderer needs the actual content, while the
/// fingerprint is a cheap diagnostic/native handoff value. Kind is part of
/// the identity: the same source must not share Math and Mermaid output.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EmbeddedResourceKey {
    kind: EmbeddedResourceKind,
    source: Arc<str>,
    fingerprint: u64,
}

impl EmbeddedResourceKey {
    pub fn new(
        kind: EmbeddedResourceKind,
        source: impl Into<Arc<str>>,
    ) -> Result<Self, EmbeddedResourceKeyError> {
        let source = source.into();
        if source.is_empty() {
            return Err(EmbeddedResourceKeyError::EmptySource);
        }
        Ok(Self {
            kind,
            fingerprint: fingerprint(kind, &source),
            source,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> EmbeddedResourceKind {
        self.kind
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
}

fn fingerprint(kind: EmbeddedResourceKind, source: &str) -> u64 {
    let mut hash = 1_469_598_103_934_665_603_u64 ^ u64::from(kind.tag());
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211_u64);
    }
    hash
}

/// A source-backed request handed to an embedded renderer worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedRenderRequest {
    revision: Revision,
    source_range: TextRange,
    key: EmbeddedResourceKey,
}

impl EmbeddedRenderRequest {
    pub fn new(
        revision: Revision,
        source_range: TextRange,
        kind: EmbeddedResourceKind,
        source: impl Into<Arc<str>>,
    ) -> Result<Self, EmbeddedResourceKeyError> {
        Ok(Self {
            revision,
            source_range,
            key: EmbeddedResourceKey::new(kind, source)?,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn source_range(&self) -> TextRange {
        self.source_range
    }

    #[must_use]
    pub const fn kind(&self) -> EmbeddedResourceKind {
        self.key.kind()
    }

    #[must_use]
    pub fn source(&self) -> &str {
        self.key.source()
    }

    #[must_use]
    pub fn key(&self) -> &EmbeddedResourceKey {
        &self.key
    }
}

/// Intrinsic dimensions supplied with an embedded render output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedDimensions {
    width: u32,
    height: u32,
}

impl EmbeddedDimensions {
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

/// Output format owned by an embedded renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedRenderFormat {
    Rgba8,
    Svg,
}

/// Owned visual output ready for a later layout/scene or platform uploader.
///
/// The cache does not interpret SVG and does not create GPU handles. Keeping
/// both raster and vector forms here lets Math/Mermaid backends choose their
/// natural output without changing the request or Revision protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddedRenderPayload {
    Rgba8 {
        dimensions: EmbeddedDimensions,
        pixels: Arc<[u8]>,
    },
    Svg {
        dimensions: EmbeddedDimensions,
        markup: Arc<str>,
    },
}

impl EmbeddedRenderPayload {
    pub fn rgba8(
        width: u32,
        height: u32,
        pixels: impl Into<Arc<[u8]>>,
    ) -> Result<Self, EmbeddedPayloadError> {
        let dimensions = EmbeddedDimensions::new(width, height)
            .ok_or(EmbeddedPayloadError::InvalidDimensions)?;
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(EmbeddedPayloadError::PixelBufferOverflow)?;
        let pixels = pixels.into();
        if pixels.len() != expected {
            return Err(EmbeddedPayloadError::PixelLength {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self::Rgba8 { dimensions, pixels })
    }

    pub fn svg(
        width: u32,
        height: u32,
        markup: impl Into<Arc<str>>,
    ) -> Result<Self, EmbeddedPayloadError> {
        let dimensions = EmbeddedDimensions::new(width, height)
            .ok_or(EmbeddedPayloadError::InvalidDimensions)?;
        let markup = markup.into();
        if markup.trim().is_empty() {
            return Err(EmbeddedPayloadError::EmptySvg);
        }
        Ok(Self::Svg { dimensions, markup })
    }

    #[must_use]
    pub const fn format(&self) -> EmbeddedRenderFormat {
        match self {
            Self::Rgba8 { .. } => EmbeddedRenderFormat::Rgba8,
            Self::Svg { .. } => EmbeddedRenderFormat::Svg,
        }
    }

    #[must_use]
    pub const fn dimensions(&self) -> EmbeddedDimensions {
        match self {
            Self::Rgba8 { dimensions, .. } | Self::Svg { dimensions, .. } => *dimensions,
        }
    }

    #[must_use]
    pub fn pixels(&self) -> Option<&[u8]> {
        match self {
            Self::Rgba8 { pixels, .. } => Some(pixels),
            Self::Svg { .. } => None,
        }
    }

    #[must_use]
    pub fn markup(&self) -> Option<&str> {
        match self {
            Self::Rgba8 { .. } => None,
            Self::Svg { markup, .. } => Some(markup),
        }
    }
}

/// Errors raised while validating a renderer-owned payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddedPayloadError {
    InvalidDimensions,
    PixelBufferOverflow,
    PixelLength { expected: usize, actual: usize },
    EmptySvg,
}

impl fmt::Display for EmbeddedPayloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => formatter.write_str("embedded dimensions must be positive"),
            Self::PixelBufferOverflow => {
                formatter.write_str("embedded pixel buffer size overflowed")
            }
            Self::PixelLength { expected, actual } => write!(
                formatter,
                "embedded pixel buffer has {actual} bytes, expected {expected}"
            ),
            Self::EmptySvg => formatter.write_str("embedded SVG markup must not be empty"),
        }
    }
}

impl Error for EmbeddedPayloadError {}

/// A successful renderer result bound to the request's Revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedRenderPublication {
    revision: Revision,
    generation: u64,
    source_range: TextRange,
    key: EmbeddedResourceKey,
    payload: EmbeddedRenderPayload,
}

impl EmbeddedRenderPublication {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn source_range(&self) -> TextRange {
        self.source_range
    }

    #[must_use]
    pub const fn kind(&self) -> EmbeddedResourceKind {
        self.key.kind()
    }

    #[must_use]
    pub fn key(&self) -> &EmbeddedResourceKey {
        &self.key
    }

    #[must_use]
    pub const fn payload(&self) -> &EmbeddedRenderPayload {
        &self.payload
    }
}

/// Stable reason class recorded when a renderer cannot publish output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EmbeddedFailureKind {
    Unsupported,
    InvalidSource,
    Render,
    Worker,
}

impl EmbeddedFailureKind {
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        !matches!(self, Self::Unsupported | Self::InvalidSource)
    }
}

/// Error returned by a replaceable [`EmbeddedRenderer`] implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedRenderError {
    Unsupported,
    InvalidSource,
    Render,
    Worker,
}

impl EmbeddedRenderError {
    #[must_use]
    pub const fn failure_kind(self) -> EmbeddedFailureKind {
        match self {
            Self::Unsupported => EmbeddedFailureKind::Unsupported,
            Self::InvalidSource => EmbeddedFailureKind::InvalidSource,
            Self::Render => EmbeddedFailureKind::Render,
            Self::Worker => EmbeddedFailureKind::Worker,
        }
    }
}

/// Backend-independent renderer extension point.
pub trait EmbeddedRenderer {
    fn render(
        &self,
        request: &EmbeddedRenderRequest,
    ) -> Result<EmbeddedRenderPayload, EmbeddedRenderError>;
}

/// Explicit no-renderer implementation used while a host has not registered
/// a Math/Mermaid backend. Keeping this as a renderer value makes the
/// unsupported state use the same completion path as a real worker.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnsupportedEmbeddedRenderer;

impl EmbeddedRenderer for UnsupportedEmbeddedRenderer {
    fn render(
        &self,
        _request: &EmbeddedRenderRequest,
    ) -> Result<EmbeddedRenderPayload, EmbeddedRenderError> {
        Err(EmbeddedRenderError::Unsupported)
    }
}

/// Revision-bound failure metadata for one embedded resource key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddedFailure {
    revision: Revision,
    key: EmbeddedResourceKey,
    kind: EmbeddedFailureKind,
    attempts: u32,
    next_retry_tick: u64,
}

impl EmbeddedFailure {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn key(&self) -> &EmbeddedResourceKey {
        &self.key
    }

    #[must_use]
    pub const fn kind(&self) -> EmbeddedFailureKind {
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
    pub const fn is_exhausted(&self, policy: EmbeddedRetryPolicy) -> bool {
        self.attempts >= policy.max_attempts()
    }
}

/// Bounded retry policy for transient embedded renderer failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbeddedRetryPolicy {
    max_attempts: u32,
    base_delay_ticks: u64,
    max_delay_ticks: u64,
}

impl EmbeddedRetryPolicy {
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

impl Default for EmbeddedRetryPolicy {
    fn default() -> Self {
        Self::new(3, 2, 60)
    }
}

/// Result of requesting an embedded renderer output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddedRequestResult {
    Ready(EmbeddedRenderPublication),
    Pending,
    Failed(EmbeddedFailure),
}

/// Errors raised while publishing an embedded renderer result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddedCacheError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    GenerationOverflow,
    InvalidCapacity,
}

impl fmt::Display for EmbeddedCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "embedded publication {actual:?} is stale for {expected:?}"
            ),
            Self::GenerationOverflow => formatter.write_str("embedded generation overflowed"),
            Self::InvalidCapacity => {
                formatter.write_str("embedded cache capacity must be positive")
            }
        }
    }
}

impl Error for EmbeddedCacheError {}

#[derive(Clone, Debug)]
struct EmbeddedCacheEntry {
    generation: u64,
    payload: EmbeddedRenderPayload,
    last_used: u64,
}

/// Revision-aware cache and pollable worker queue for Math/Mermaid outputs.
///
/// The cache is intentionally executor-free. A host calls [`Self::pending`],
/// invokes an [`EmbeddedRenderer`] on a worker, then calls [`Self::complete`]
/// (or the lower-level `publish`/`record_failure`) on the owner thread.
/// Publications are rebound to the requesting source range and old Revision
/// results are rejected before they can replace a current entry.
#[derive(Clone, Debug)]
pub struct EmbeddedResourceCache {
    entries: HashMap<EmbeddedResourceKey, EmbeddedCacheEntry>,
    failures: HashMap<EmbeddedResourceKey, EmbeddedFailure>,
    pending: VecDeque<EmbeddedRenderRequest>,
    pending_keys: HashSet<EmbeddedResourceKey>,
    in_flight: HashMap<EmbeddedResourceKey, Revision>,
    next_generation: u64,
    next_access: u64,
    capacity: usize,
    evictions: u64,
    retry_policy: EmbeddedRetryPolicy,
    retry_tick: u64,
}

impl Default for EmbeddedResourceCache {
    fn default() -> Self {
        Self::with_capacity(Self::DEFAULT_CAPACITY)
    }
}

impl EmbeddedResourceCache {
    pub const DEFAULT_CAPACITY: usize = 32;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            failures: HashMap::new(),
            pending: VecDeque::new(),
            pending_keys: HashSet::new(),
            in_flight: HashMap::new(),
            next_generation: 0,
            next_access: 0,
            capacity: capacity.max(1),
            evictions: 0,
            retry_policy: EmbeddedRetryPolicy::default(),
            retry_tick: 0,
        }
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn set_capacity(&mut self, capacity: usize) -> Result<usize, EmbeddedCacheError> {
        if capacity == 0 {
            return Err(EmbeddedCacheError::InvalidCapacity);
        }
        self.capacity = capacity;
        Ok(self.trim_to_capacity())
    }

    #[must_use]
    pub const fn retry_policy(&self) -> EmbeddedRetryPolicy {
        self.retry_policy
    }

    pub fn set_retry_policy(&mut self, policy: EmbeddedRetryPolicy) {
        self.retry_policy = policy;
    }

    pub fn advance_retry_clock(&mut self) {
        self.retry_tick = self.retry_tick.saturating_add(1);
    }

    #[must_use]
    pub const fn retry_tick(&self) -> u64 {
        self.retry_tick
    }

    pub fn request(&mut self, request: EmbeddedRenderRequest) -> EmbeddedRequestResult {
        if let Some(failure) = self.failures.get(request.key()).cloned() {
            if failure.revision == request.revision {
                if failure.kind == EmbeddedFailureKind::Unsupported
                    || failure.kind == EmbeddedFailureKind::InvalidSource
                    || failure.is_exhausted(self.retry_policy)
                    || self.retry_tick < failure.next_retry_tick
                {
                    return EmbeddedRequestResult::Failed(failure);
                }
                self.failures.remove(request.key());
            } else {
                self.failures.remove(request.key());
            }
        }

        let access_tick = self.next_access_tick();
        if let Some(entry) = self.entries.get_mut(request.key()) {
            entry.last_used = access_tick;
            return EmbeddedRequestResult::Ready(EmbeddedRenderPublication {
                revision: request.revision,
                generation: entry.generation,
                source_range: request.source_range,
                key: request.key.clone(),
                payload: entry.payload.clone(),
            });
        }

        let key = request.key.clone();
        if self.in_flight.contains_key(&key) {
            return EmbeddedRequestResult::Pending;
        }
        if self.pending_keys.contains(&key) {
            if let Some(queued) = self.pending.iter_mut().find(|queued| queued.key == key) {
                *queued = request;
            }
        } else {
            self.pending_keys.insert(key);
            self.pending.push_back(request);
        }
        EmbeddedRequestResult::Pending
    }

    /// Pops one request for a worker. The key remains in `in_flight` until a
    /// matching success or failure is completed.
    pub fn pending(&mut self) -> Option<EmbeddedRenderRequest> {
        let request = self.pending.pop_front()?;
        self.pending_keys.remove(request.key());
        self.in_flight
            .insert(request.key().clone(), request.revision());
        Some(request)
    }

    /// Runs one pending request through a replaceable renderer. Hosts that
    /// need a real background thread can use `pending` and `complete` instead.
    pub fn render_pending<R: EmbeddedRenderer + ?Sized>(
        &mut self,
        current_revision: Revision,
        renderer: &R,
    ) -> Result<Option<EmbeddedRequestResult>, EmbeddedCacheError> {
        let Some(request) = self.pending() else {
            return Ok(None);
        };
        let result = renderer.render(&request);
        Ok(Some(self.complete(request, current_revision, result)?))
    }

    pub fn complete(
        &mut self,
        request: EmbeddedRenderRequest,
        current_revision: Revision,
        result: Result<EmbeddedRenderPayload, EmbeddedRenderError>,
    ) -> Result<EmbeddedRequestResult, EmbeddedCacheError> {
        match result {
            Ok(payload) => self
                .publish(request, current_revision, payload)
                .map(EmbeddedRequestResult::Ready),
            Err(error) => self
                .record_failure(request, current_revision, error.failure_kind())
                .map(EmbeddedRequestResult::Failed),
        }
    }

    pub fn publish(
        &mut self,
        request: EmbeddedRenderRequest,
        current_revision: Revision,
        payload: EmbeddedRenderPayload,
    ) -> Result<EmbeddedRenderPublication, EmbeddedCacheError> {
        if request.revision != current_revision {
            self.remove_in_flight(request.key(), request.revision());
            return Err(EmbeddedCacheError::StaleRevision {
                expected: current_revision,
                actual: request.revision,
            });
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(EmbeddedCacheError::GenerationOverflow)?;
        self.remove_in_flight(request.key(), request.revision());
        self.pending.retain(|pending| pending.key != request.key);
        self.pending_keys.remove(&request.key);
        self.failures.remove(&request.key);
        let generation = self.next_generation;
        let access_tick = self.next_access_tick();
        self.entries.insert(
            request.key.clone(),
            EmbeddedCacheEntry {
                generation,
                payload: payload.clone(),
                last_used: access_tick,
            },
        );
        self.trim_to_capacity();
        Ok(EmbeddedRenderPublication {
            revision: current_revision,
            generation,
            source_range: request.source_range,
            key: request.key,
            payload,
        })
    }

    pub fn record_failure(
        &mut self,
        request: EmbeddedRenderRequest,
        current_revision: Revision,
        kind: EmbeddedFailureKind,
    ) -> Result<EmbeddedFailure, EmbeddedCacheError> {
        if request.revision != current_revision {
            self.remove_in_flight(request.key(), request.revision());
            return Err(EmbeddedCacheError::StaleRevision {
                expected: current_revision,
                actual: request.revision,
            });
        }
        self.remove_in_flight(request.key(), request.revision());
        self.pending.retain(|pending| pending.key != request.key);
        self.pending_keys.remove(&request.key);
        let attempts = self
            .failures
            .get(request.key())
            .filter(|failure| failure.revision == current_revision)
            .map_or(1, |failure| failure.attempts.saturating_add(1));
        let next_retry_tick = if kind.is_retryable() {
            self.retry_tick
                .saturating_add(self.retry_policy.delay_for_attempt(attempts))
        } else {
            u64::MAX
        };
        let failure = EmbeddedFailure {
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
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn failure_count(&self) -> usize {
        self.failures.len()
    }

    #[must_use]
    pub const fn eviction_count(&self) -> u64 {
        self.evictions
    }

    #[must_use]
    pub fn failure(&self, key: &EmbeddedResourceKey) -> Option<&EmbeddedFailure> {
        self.failures.get(key)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.failures.clear();
        self.pending.clear();
        self.pending_keys.clear();
        self.in_flight.clear();
    }

    fn remove_in_flight(&mut self, key: &EmbeddedResourceKey, revision: Revision) {
        if self.in_flight.get(key).copied() == Some(revision) {
            self.in_flight.remove(key);
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use yu_core::ByteOffset;

    fn make_request(
        revision: u64,
        source_start: u64,
        kind: EmbeddedResourceKind,
        source: &str,
    ) -> EmbeddedRenderRequest {
        EmbeddedRenderRequest::new(
            Revision::new(revision),
            TextRange::new(
                ByteOffset::new(source_start),
                ByteOffset::new(source_start + 4),
            )
            .expect("range"),
            kind,
            source,
        )
        .expect("request")
    }

    fn raster(byte: u8) -> EmbeddedRenderPayload {
        EmbeddedRenderPayload::rgba8(1, 1, [byte, byte, byte, 255]).expect("payload")
    }

    struct TestRenderer;

    impl EmbeddedRenderer for TestRenderer {
        fn render(
            &self,
            request: &EmbeddedRenderRequest,
        ) -> Result<EmbeddedRenderPayload, EmbeddedRenderError> {
            if request.kind() == EmbeddedResourceKind::Mermaid {
                Ok(EmbeddedRenderPayload::svg(2, 3, "<svg />").expect("svg"))
            } else {
                Ok(raster(7))
            }
        }
    }

    #[test]
    fn math_and_mermaid_keys_do_not_alias() {
        let math = EmbeddedResourceKey::new(EmbeddedResourceKind::Math, "x^2").expect("math");
        let mermaid =
            EmbeddedResourceKey::new(EmbeddedResourceKind::Mermaid, "x^2").expect("mermaid");
        assert_ne!(math, mermaid);
        assert_ne!(math.fingerprint(), mermaid.fingerprint());
        assert_eq!(math.kind(), EmbeddedResourceKind::Math);
        assert_eq!(mermaid.kind().language(), "mermaid");
    }

    #[test]
    fn payload_validation_rejects_invalid_output() {
        assert_eq!(
            EmbeddedRenderPayload::rgba8(1, 1, [0; 3]).expect_err("short pixels"),
            EmbeddedPayloadError::PixelLength {
                expected: 4,
                actual: 3,
            }
        );
        assert_eq!(
            EmbeddedRenderPayload::svg(1, 1, "  ").expect_err("empty SVG"),
            EmbeddedPayloadError::EmptySvg
        );
    }

    #[test]
    fn pending_requests_are_deduplicated_and_latest_range_wins() {
        let mut cache = EmbeddedResourceCache::new();
        assert!(matches!(
            cache.request(make_request(0, 0, EmbeddedResourceKind::Math, "x^2")),
            EmbeddedRequestResult::Pending
        ));
        assert!(matches!(
            cache.request(make_request(0, 12, EmbeddedResourceKind::Math, "x^2")),
            EmbeddedRequestResult::Pending
        ));
        let pending = cache.pending().expect("pending");
        assert_eq!(pending.source_range().start().get(), 12);
        assert!(cache.pending().is_none());
    }

    #[test]
    fn renderer_trait_publishes_ready_output() {
        let mut cache = EmbeddedResourceCache::new();
        let current = Revision::new(3);
        let request = make_request(3, 8, EmbeddedResourceKind::Mermaid, "flowchart TD");
        assert!(matches!(
            cache.request(request),
            EmbeddedRequestResult::Pending
        ));
        let result = cache
            .render_pending(current, &TestRenderer)
            .expect("render")
            .expect("one result");
        let EmbeddedRequestResult::Ready(publication) = result else {
            panic!("expected ready publication");
        };
        assert_eq!(publication.revision(), current);
        assert_eq!(publication.generation(), 1);
        assert_eq!(publication.source_range().start().get(), 8);
        assert_eq!(publication.payload().format(), EmbeddedRenderFormat::Svg);
        assert_eq!(publication.payload().dimensions().height(), 3);
        assert!(matches!(
            cache.request(make_request(
                3,
                20,
                EmbeddedResourceKind::Mermaid,
                "flowchart TD"
            )),
            EmbeddedRequestResult::Ready(_)
        ));
    }

    #[test]
    fn stale_publication_is_rejected_and_does_not_poison_new_revision() {
        let mut cache = EmbeddedResourceCache::new();
        let stale = make_request(1, 0, EmbeddedResourceKind::Math, "x^2");
        assert!(matches!(
            cache.request(stale),
            EmbeddedRequestResult::Pending
        ));
        let stale = cache.pending().expect("stale work");
        assert_eq!(
            cache.publish(stale, Revision::new(2), raster(1)),
            Err(EmbeddedCacheError::StaleRevision {
                expected: Revision::new(2),
                actual: Revision::new(1),
            })
        );
        assert!(matches!(
            cache.request(make_request(2, 4, EmbeddedResourceKind::Math, "x^2")),
            EmbeddedRequestResult::Pending
        ));
    }

    #[test]
    fn unsupported_failure_is_stable_until_revision_changes() {
        let mut cache = EmbeddedResourceCache::new();
        let current = Revision::new(4);
        let request = make_request(4, 0, EmbeddedResourceKind::Math, "x^2");
        assert!(matches!(
            cache.request(request),
            EmbeddedRequestResult::Pending
        ));
        let result = cache
            .render_pending(current, &UnsupportedEmbeddedRenderer)
            .expect("failure");
        let result = result.expect("one result");
        let EmbeddedRequestResult::Failed(failure) = result else {
            panic!("expected failure");
        };
        assert_eq!(failure.kind(), EmbeddedFailureKind::Unsupported);
        assert_eq!(failure.attempts(), 1);
        assert!(matches!(
            cache.request(make_request(4, 8, EmbeddedResourceKind::Math, "x^2")),
            EmbeddedRequestResult::Failed(_)
        ));
        assert!(matches!(
            cache.request(make_request(5, 8, EmbeddedResourceKind::Math, "x^2")),
            EmbeddedRequestResult::Pending
        ));
    }

    #[test]
    fn cache_capacity_evicts_oldest_publication() {
        let mut cache = EmbeddedResourceCache::with_capacity(1);
        let first = make_request(0, 0, EmbeddedResourceKind::Math, "x");
        assert!(matches!(
            cache.request(first),
            EmbeddedRequestResult::Pending
        ));
        let first = cache.pending().expect("first");
        cache
            .publish(first, Revision::new(0), raster(1))
            .expect("first");
        let second = make_request(0, 4, EmbeddedResourceKind::Math, "y");
        assert!(matches!(
            cache.request(second),
            EmbeddedRequestResult::Pending
        ));
        let second = cache.pending().expect("second");
        cache
            .publish(second, Revision::new(0), raster(2))
            .expect("second");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.eviction_count(), 1);
    }
}
