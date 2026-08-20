# ADR 0176: Retained editor decorations in the Metal surface

## Context

The product host already draws projected glyphs, tables, images, Math and task
checkboxes through the persistent Rust/Metal publication. Selection and caret
pixels remained in a separate AppKit sibling even though their geometry came
from Rust/CoreText. This split made one visual frame depend on two painters and
kept normal selection/caret rendering outside the retained scene.

The migration cannot hide the AppKit decoration layer merely because a Metal
submit succeeded. A stale or incomplete frame could otherwise remove the
caret, especially while an IME composition changes without advancing the
canonical source Revision.

## Decision

1. `yu-scene` retains source-backed `EditorDecorationPrimitive` layers with
   explicit `Selection`, `Caret` and `CompositionCaret` roles.
2. `yu-workspace` derives their document-space geometry from the same visible
   block layouts and projections used by the frame. Normal selection uses the
   canonical anchor/focus range; composition uses its transient visual
   selection and active visual caret without editing source.
3. Decoration style is an optional `ViewportRenderConfig` input. Generic scene
   assembly remains decoration-free unless a platform host opts in.
4. The renderer lowers decoration layers to the existing solid rectangle
   command, preserving semantic roles in the retained scene while reusing the
   current Metal pipeline.
5. macOS host and surface snapshots publish exact selection and caret layer
   counts. Swift disables `MacosVisualDecorationView` painting only when the
   submitted surface has the current Revision and composition generation and
   its counts equal the independently queried Rust decoration geometry.
6. Selection/caret movement does not advance source Revision. Every native
   caret-change event therefore revokes the current surface snapshot and
   submit deduplication key, restores TextKit fallback, and coalesces a new
   retained publication before Metal decorations can be accepted again.
7. A missing, stale or count-mismatched surface keeps AppKit decoration
   painting enabled. TextKit continues to own source, selection, input, IME and
   Accessibility throughout this migration.

## Consequences

- The normal selection and caret pixels now share the same retained scene,
  RenderPlan, damage tracking and Metal submit as document content.
- IME preedit selection/caret remains generation-bound without a shadow text
  model.
- AppKit remains a fail-closed visual fallback rather than a second normal
  renderer.
- Complete removal of the TextKit source mirror is still deferred until all
  input, unsupported-content and Accessibility fallback requirements have
  replacement contracts.
