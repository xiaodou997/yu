# ADR 0175: Revision-bound task checkbox pointer

## Context

ADR 0174 publishes hidden GFM task markers as source-backed checkbox layers in
the retained scene. The visible checkbox still could not be activated with the
mouse: AppKit's projected text hit-test maps text positions, while the checkbox
is a scene decoration replacing hidden `[ ]`/`[x]` source.

A native implementation must not infer checkbox bounds from font metrics or
parse Markdown independently. It also must not activate geometry from an old
frame after an edit, scroll/layout publication, or composition change.

## Decision

1. `ViewportSceneFrame::task_checkbox_hit_test` queries only `Border` layers.
   Interior/check layers are painter details and never produce duplicate hits.
2. A successful hit contains the scene `Revision`, parser block index, exact
   marker `TextRange`, and document-space bounds.
3. The macOS storage FFI queries the persistent render host's last accepted
   publication. It does not rebuild layout. Missing publications, active IME
   composition, non-finite points, points outside a checkbox, and stale
   revisions are rejected without mutation.
4. Swift forwards a plain primary click before ordinary projected selection.
   A valid hit invokes the existing `ToggleTask(block)` Rust command; source
   synchronization, undo history, selection, Accessibility refresh, and the
   next visual publication follow the normal command result path.
5. A consumed checkbox mouse-down also consumes its drag/up sequence. Shift
   clicks and clicks without a current Metal publication remain ordinary text
   selection.

## Consequences

- Pointer geometry is identical to the checkbox users see.
- `[ ]`/`[x]` remains the canonical state and undo/redo needs no second model.
- Native code owns no Markdown grammar or long-lived task identity.
- The interaction is deliberately macOS-first, while the scene hit contract is
  platform-neutral and reusable by future Windows/Linux shells.

