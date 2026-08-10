//! macOS CoreText font discovery and fallback selection.
//!
//! This crate intentionally returns owned, platform-neutral metadata rather
//! than exposing `CTFontRef` to the editor core. A future CoreText shaper can
//! use the same resolver while keeping CoreText objects on the platform side.

use std::error::Error;
use std::fmt;
use std::sync::Arc;

use yu_font::FontRequest;

#[cfg(target_os = "macos")]
use objc2_core_foundation::{CFArray, CFRange, CFRetained, CFString};
#[cfg(target_os = "macos")]
use objc2_core_text::{CTFont, CTFontManagerCopyAvailableFontFamilyNames};

/// Errors raised by the CoreText adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoreTextFontError {
    UnsupportedPlatform,
    EmptyCatalog,
    InvalidTextRange,
    FontNameUnavailable,
}

impl fmt::Display for CoreTextFontError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => formatter.write_str("CoreText is only available on macOS"),
            Self::EmptyCatalog => formatter.write_str("CoreText returned no font families"),
            Self::InvalidTextRange => formatter.write_str("text is too large for a CoreText range"),
            Self::FontNameUnavailable => formatter.write_str("CoreText did not return a font name"),
        }
    }
}

impl Error for CoreTextFontError {}

/// A retained, sorted snapshot of the font family names visible to CoreText.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreTextFontCatalog {
    families: Arc<[Arc<str>]>,
}

impl CoreTextFontCatalog {
    /// Reads the current system font family catalog from CoreText.
    pub fn system() -> Result<Self, CoreTextFontError> {
        #[cfg(target_os = "macos")]
        {
            let families = unsafe { CTFontManagerCopyAvailableFontFamilyNames() };
            // CoreText returns an array whose elements are CFStringRef. The
            // binding intentionally erases the element type at the C ABI, so
            // this checked-at-the-call-site cast restores it for iteration.
            let families: CFRetained<CFArray<CFString>> =
                unsafe { CFRetained::cast_unchecked(families) };
            let mut names = families
                .iter()
                .map(|family| Arc::<str>::from(family.to_string()))
                .filter(|family| !family.trim().is_empty())
                .collect::<Vec<_>>();
            names.sort_unstable_by(|left, right| left.as_ref().cmp(right.as_ref()));
            names.dedup_by(|left, right| left.as_ref() == right.as_ref());
            if names.is_empty() {
                return Err(CoreTextFontError::EmptyCatalog);
            }
            Ok(Self {
                families: Arc::from(names.into_boxed_slice()),
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            Err(CoreTextFontError::UnsupportedPlatform)
        }
    }

    /// Builds a catalog from deterministic names in tests or a platform
    /// bootstrap layer.
    #[must_use]
    pub fn from_families(families: impl IntoIterator<Item = impl Into<Arc<str>>>) -> Self {
        let mut names = families
            .into_iter()
            .map(Into::into)
            .filter(|family: &Arc<str>| !family.trim().is_empty())
            .collect::<Vec<_>>();
        names.sort_unstable_by(|left, right| left.as_ref().cmp(right.as_ref()));
        names.dedup_by(|left, right| left.as_ref() == right.as_ref());
        Self {
            families: Arc::from(names.into_boxed_slice()),
        }
    }

    #[must_use]
    pub fn families(&self) -> &[Arc<str>] {
        &self.families
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.families.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }

    #[must_use]
    pub fn contains_family(&self, family: &str) -> bool {
        self.families
            .iter()
            .any(|candidate| candidate.as_ref() == family)
    }

    #[must_use]
    pub fn resolver(&self) -> CoreTextFontResolver {
        CoreTextFontResolver {
            catalog: self.clone(),
        }
    }
}

/// Metadata for one CoreText-selected face. The underlying `CTFontRef` is
/// intentionally not stored here, so this value is safe to move across the
/// editor's platform-independent layers.
#[derive(Clone, Debug, PartialEq)]
pub struct CoreTextResolvedFont {
    requested_family: Arc<str>,
    family: Arc<str>,
    postscript_name: Arc<str>,
    size: f32,
    fallback: bool,
}

impl CoreTextResolvedFont {
    #[must_use]
    pub fn requested_family(&self) -> &str {
        &self.requested_family
    }

    #[must_use]
    pub fn family(&self) -> &str {
        &self.family
    }

    #[must_use]
    pub fn postscript_name(&self) -> &str {
        &self.postscript_name
    }

    #[must_use]
    pub const fn size(&self) -> f32 {
        self.size
    }

    #[must_use]
    pub const fn used_fallback(&self) -> bool {
        self.fallback
    }
}

/// A resolver scoped to one catalog snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoreTextFontResolver {
    catalog: CoreTextFontCatalog,
}

impl CoreTextFontResolver {
    #[must_use]
    pub fn catalog(&self) -> &CoreTextFontCatalog {
        &self.catalog
    }

    /// Resolves a family and lets CoreText choose a cascade fallback for the
    /// supplied UTF-8 string. This performs no shaping or glyph rasterization.
    pub fn resolve(
        &self,
        request: &FontRequest,
        text: &str,
    ) -> Result<CoreTextResolvedFont, CoreTextFontError> {
        #[cfg(target_os = "macos")]
        {
            let family = CFString::from_str(request.family());
            let base = unsafe { CTFont::with_name(&family, request.size() as _, std::ptr::null()) };
            let selected = if text.is_empty() {
                base
            } else {
                let text = CFString::from_str(text);
                let length = text.length();
                let range = CFRange {
                    location: 0,
                    length,
                };
                unsafe { base.for_string(&text, range) }
            };
            let selected_family = unsafe { selected.family_name() }.to_string();
            let postscript_name = unsafe { selected.post_script_name() }.to_string();
            if selected_family.trim().is_empty() || postscript_name.trim().is_empty() {
                return Err(CoreTextFontError::FontNameUnavailable);
            }
            Ok(CoreTextResolvedFont {
                requested_family: Arc::from(request.family()),
                fallback: selected_family != request.family(),
                family: Arc::from(selected_family),
                postscript_name: Arc::from(postscript_name),
                size: request.size(),
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (request, text);
            Err(CoreTextFontError::UnsupportedPlatform)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_normalizes_names() {
        let catalog = CoreTextFontCatalog::from_families(["Zed", "Yu", "Yu", ""]);
        assert_eq!(
            catalog.families(),
            &[Arc::<str>::from("Yu"), Arc::from("Zed")]
        );
        assert!(catalog.contains_family("Yu"));
        assert!(!catalog.contains_family("Missing"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_catalog_and_resolver_are_live() {
        let catalog = CoreTextFontCatalog::system().expect("CoreText should expose families");
        assert!(!catalog.is_empty());
        let family = catalog.families()[0].clone();
        let request = FontRequest::new(family.as_ref(), 13.0).expect("request should be valid");
        let resolved = catalog
            .resolver()
            .resolve(&request, "羽🙂")
            .expect("CoreText should resolve a fallback font");
        assert_eq!(resolved.requested_family(), family.as_ref());
        assert!(!resolved.family().is_empty());
        assert!(!resolved.postscript_name().is_empty());
        assert_eq!(resolved.size(), 13.0);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn system_catalog_is_explicitly_unsupported_off_macos() {
        assert_eq!(
            CoreTextFontCatalog::system(),
            Err(CoreTextFontError::UnsupportedPlatform)
        );
    }
}
