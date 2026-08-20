# ADR 0182: Source-backed blockquote decoration

## Context

Canonical blockquote projection already hid parser-owned `>` prefixes, but the
retained document had no visual replacement. Drawing a quote line in Swift
would duplicate Markdown semantics, lose source identity and bypass shared
layout, scene ordering and damage tracking. Merely shifting glyph paint origins
would also leave wrapping, images, caret geometry and hit-testing inconsistent.

Focus projection must reveal exact Markdown syntax while retaining the stable
semantic quote decoration and base gutter.

## Decision

1. `BlockQuotePresentation` retains the complete parser block range, every
   parser-owned physical prefix range and quote depth. Canonical and focus
   projections carry identical presentation metadata; only hidden syntax runs
   differ.
2. `yu-layout` derives one quote unit from line height and default advance. The
   total depth gutter reduces available layout width before line breaking.
3. The same horizontal inset is applied to text clusters, shaped glyphs,
   semantic list markers and image placement. Caret and hit-test geometry
   therefore use the painted coordinate system rather than a renderer-only
   offset.
4. `BlockQuoteLayout` stores block-local bar geometry and the complete source
   range. Image intrinsic-size updates refresh bar height after block reflow.
5. `yu-scene` emits one `BlockQuotePrimitive` per depth level before the
   block's glyphs. Each primitive retains the complete quote source range.
6. `yu-render` lowers quote primitives to the existing `FillRect` command.
   Metal needs no Markdown-specific pipeline and platform code does not parse
   quote delimiters.
7. The current Markdown parser reports one quote depth. The retained contracts
   support greater depth without inventing nesting in projection or layout.

## Consequences

- An unfocused quote has stable indentation and a visible quote line while its
  Markdown prefixes remain hidden.
- Entering the quote reveals exact prefixes without removing the line or base
  gutter. The revealed source text still participates in line layout and may
  reflow content.
- Wrapping, caret placement, pointer hit-testing and image width agree with the
  final painted position.
- Quote color remains a product-level workspace policy; syntax, layout, scene
  and renderer crates do not depend on Yu Desktop styling.
- Nested quote semantics remain limited by current parser metadata, not by the
  downstream projection/scene contract.

## Verification

```bash
cargo test -p yu-projection structural_block_projection
cargo test -p yu-layout blockquote_reserves_content_width
cargo test -p yu-workspace blockquote_bar_precedes_glyphs
cargo test --workspace
```
