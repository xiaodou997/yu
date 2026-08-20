# ADR 0181: Source-backed semantic list markers

## Context

Ordinary list prefixes remained visible after heading and blockquote prefixes
became structural projection syntax. Hiding `- ` or `12) ` without a visual
replacement would erase list semantics, while drawing a marker directly in a
platform shell would duplicate Markdown parsing and bypass glyph shaping,
atlas publication and source hit-testing.

Task lists already have a separate source-backed checkbox overlay. This
decision must not destabilize that accepted interaction.

## Decision

1. `yu-markdown::list_marker` is the sole owner of ordinary-list marker and
   prefix ranges. It scans the first parser-owned line through snapshot chunks
   and validates the result against `BlockKind` metadata.
2. Canonical ordinary-list projections hide indentation, marker and following
   whitespace. They retain a `LeadingMarker` whose source identity is the
   exact marker token. Unordered lists display `•`; ordered lists display the
   parsed starting number and original `.`/`)` delimiter.
3. A focus projection reveals the exact Markdown prefix and removes the
   semantic marker. Selection movement changes visual state only; source,
   Revision and canonical projection caches remain unchanged.
4. `yu-layout` shapes semantic marker text through the same provider used for
   document text. Marker glyph placements map back to the exact source marker
   range and use a zero-width visual identity.
5. Parser-owned leading spaces, marker advance and one default advance form a
   hanging gutter. Body clusters and glyphs on every wrapped line share that
   inset, so nested indentation, caret and hit-test geometry stay aligned with
   painting.
6. Marker glyphs enter the normal CPU atlas, retained scene and render plan.
   No list-specific platform drawing or renderer command is introduced.
7. Task-list prefixes keep their existing source-visible bullet and
   source-backed checkbox path. Replacing that combination requires a separate
   interaction decision.

## Consequences

- Unfocused ordinary lists show stable bullets or ordinals without exposing
  Markdown punctuation.
- Entering a list item reveals the exact spelling, indentation and whitespace
  the user can edit.
- CoreText fallback, glyph rasterization, damage tracking and Metal rendering
  need no list-specific code.
- Very narrow layout widths still reserve at least one fallback advance for
  content; a marker may consume most of such a diagnostic layout, but source
  mapping remains valid.

## Verification

```bash
cargo test -p yu-markdown list_marker
cargo test -p yu-projection structural_block_projection
cargo test -p yu-layout semantic_list_marker
cargo test -p yu-layout shaped_semantic_marker
cargo test --workspace
```
