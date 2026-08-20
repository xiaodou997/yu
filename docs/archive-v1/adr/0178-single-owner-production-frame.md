# ADR 0178: Single-owner production frame

## Context

The macOS document host could accept a retained Metal frame while asking the
source `NSTextView` to paint selected source ranges above it. This transitional
mode was conservative, but it made one production frame depend on two unrelated
text layout systems. Their wrapping, delimiter visibility and block geometry
cannot be guaranteed to match, so a local fallback could overlap retained
glyphs or appear at the wrong document position.

Resource fallbacks no longer need this path. Images have retained placeholders,
embedded failures keep fenced-code projection glyphs, and an image without a
resource identity still has its projected alt label.

## Decision

1. A production document frame has exactly one glyph owner: either the complete
   retained Rust/Metal publication or the complete TextKit source fallback.
2. `VisualRetainedCoverage` carries only Revision and completeness. It does not
   carry local source ranges.
3. The Rust surface is visible only when its publication, decoration frame and
   retained coverage are current and complete for the same Revision and
   composition generation.
4. Unknown block kinds, mismatched block masks, unknown non-zero resource
   identities and invalid ABI status tags make coverage incomplete and restore
   the complete source fallback.
5. Unknown images with no resource fingerprint keep the source-backed alt-label
   glyphs already emitted by the retained projection; they do not reactivate
   TextKit for that range.
6. TextKit remains the native input, IME, clipboard and Accessibility host. This
   decision removes only its mixed production glyph role.

## Consequences

- Accepted Metal frames cannot contain TextKit glyphs with independent wrapping
  or coordinates.
- Coverage failures remain visible and editable through the complete source
  fallback.
- The remaining Phase 3 migration is narrower: eliminate the complete TextKit
  production fallback once the retained renderer can represent empty documents,
  delimiter reveal and every supported failure state.
