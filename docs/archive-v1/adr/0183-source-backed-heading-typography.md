# ADR 0183: Source-backed heading typography

## Context

ATX heading prefixes were already parser-owned hidden syntax, but every block
still used the body font size, weight and line height. Applying heading sizes
only in Swift or Metal would make wrapping, caret geometry, hit-testing,
viewport heights and atlas keys disagree with the painted result.

The current shaping providers own one base font request. Creating a separate
platform shaper object for every heading level would duplicate fallback state
and make the shared layout contract provider-specific. Blindly multiplying a
body-size shaping result would also lose native kerning, hinting and optical
size behavior.

## Decision

1. `HeadingPresentation` retains the complete block source, exact parser-owned
   ATX prefix and H1-H6 level. Canonical and focus-reveal projections carry the
   same metadata.
2. `yu-layout` maps each level to stable font and line-height scales:
   H1 `(2.0, 2.2)`, H2 `(1.7, 1.9)`, H3 `(1.45, 1.65)`, H4 `(1.25, 1.4)`,
   H5 `(1.1, 1.2)` and H6 `(1.0, 1.1)`.
3. Heading text is shaped as `Strong` through the existing provider's
   scale-aware entry point. FontShaper and CoreText shape at the target point
   size; deterministic providers use a source-preserving scaled fallback.
   Layout uses the returned advances and offsets with the scaled line box for
   wrapping, caret and viewport geometry.
4. Every shaped `GlyphPlacement` carries its raster-size multiplier. Scene
   atlas lookup, macOS CoreText rasterization, storage FFI publication and test
   atlases use `base font size × placement scale`.
5. `HeadingLayout` exposes level and effective scales for diagnostics without
   making the scene or renderer understand Markdown.
6. The block kind already contains heading level, so existing projection and
   layout cache keys separate H1-H6 and invalidate when a source edit changes
   the level.
7. Heading theme customization and combined bold-italic font traits remain a
   later typography policy. This stage establishes the source/layout/raster
   contract rather than a complete theme system.

## Consequences

- H1-H6 now have visible hierarchy in the retained renderer.
- Wrapping, caret height, pointer hit-testing and viewport virtualization use
  the same scaled geometry that the GPU paints.
- Entering a heading reveals the exact Markdown prefix without dropping its
  level or reverting to body typography.
- One frame and one atlas can safely contain multiple point sizes keyed by the
  existing `GlyphRasterKey` size field.
- Platform shells do not parse heading delimiters or select heading sizes.

## Verification

```bash
cargo test -p yu-projection structural_block_projection
cargo test -p yu-layout heading_level_controls_line_metrics
cargo test -p yu-workspace heading_and_body_share_a_frame
cargo test --workspace
```
