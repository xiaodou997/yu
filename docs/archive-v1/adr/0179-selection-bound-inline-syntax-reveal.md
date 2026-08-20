# ADR 0179: Selection-bound inline syntax reveal

## Context

Yu keeps Markdown source canonical while its normal projection hides paired
inline delimiters. Typora-style editing requires the source markers of the
active construct to become visible when the caret or selection enters it.
Selection changes do not advance the source Revision, so a revealed projection
cannot share the source-only projection and layout caches without returning
stale visual state.

Reveal also changes line width and can reflow the focus block. The projected
TextKit mirror, hit testing, decorations, glyph atlas and retained scene must
therefore consume the same visual state for one frame.

## Decision

1. Reveal is derived only from parser-owned `InlineSpan` ranges. The projection
   layer does not rescan Markdown source or infer delimiters from strings.
2. The first supported set is emphasis, strong emphasis, code spans, inline and
   reference links, and autolinks. Images, tables and block prefixes keep their
   existing specialized projections.
3. A collapsed caret reveals a span only while strictly inside its source
   range. A non-empty selection reveals every supported span it intersects.
4. Only the selection focus block receives a transient revealed projection and
   layout. Canonical projection/layout caches remain keyed by source Revision
   and never contain selection-dependent state.
5. The focus block is remeasured before viewport lookup because delimiter
   visibility may change wrapping and all following document-space offsets.
6. Active IME composition takes precedence over selection reveal. Composition
   keeps using its generation-bound transient projection until commit or
   cancel.
7. macOS refreshes its disposable projected mirror on selection-only changes.
   Mirror mapping, shaped hit testing, decoration geometry, glyph rasterization
   and retained scene assembly all query the same Rust visual-state projection.
8. A selection-only frame keeps the same source Revision but receives a new
   publication/frame serial. Native consumers must not deduplicate it by
   Revision alone.

## Consequences

- Moving the caret into supported inline syntax reveals the exact original
  delimiters without editing or reserializing Markdown.
- Moving out restores the canonical hidden projection without invalidating its
  caches.
- Reveal can rewrap one block and update downstream vertical offsets even
  though the source Revision is unchanged.
- Images and structural Markdown remain intentionally unchanged until their
  editing policies define equivalent source mapping and interaction contracts.

