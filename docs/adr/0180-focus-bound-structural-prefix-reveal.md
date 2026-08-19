# ADR 0180: Focus-bound structural prefix reveal

## Context

Canonical block projections hide parser-owned ATX heading and blockquote
prefixes because their normal visual form is represented by block semantics.
ADR 0179 added selection-bound inline delimiter reveal, but its block-level
hidden ranges deliberately remained fixed. Entering a heading or quote could
therefore reveal inner emphasis while leaving `#` or `>` unavailable.

Ordinary list markers are different today. Their source glyphs are still the
only visible bullet or ordinal; the retained scene does not yet have an
independent list-marker primitive. Hiding them before that replacement exists
would make unfocused lists ambiguous or unreadable.

## Decision

1. The focus block reveals its parser-owned heading or blockquote prefix
   together with any active inline span. Source bytes and Revision do not
   change.
2. Structural prefixes come exclusively from
   `yu_markdown::block_syntax_hidden_ranges`; projection does not rescan source
   text or infer container syntax.
3. Heading and blockquote are currently the only block projections with
   parser-owned structural hidden ranges. List projection passes no such range,
   task markers remain always hidden, and fenced code, tables and reference
   definitions keep their specialized projections.
4. `selection_reveal_block_index` is non-empty only when the transient block
   projection is actually longer than its canonical projection. A caret in an
   ordinary block therefore does not trigger off-screen remeasurement or a new
   visual-state layout.
5. Ordinary list bullets and ordinals remain source-visible in both focused
   and unfocused states. They may adopt the same reveal policy only after a
   source-backed semantic list-marker scene primitive preserves their normal
   visual representation and hit-testing.
6. macOS retained lifecycle verification compares heading reveal, inline
   reveal and no-reveal publications at one source Revision. Each selection
   change must produce a newer frame serial with the expected glyph delta.

## Consequences

- Entering a heading shows its exact indentation, `#` run and separating
  whitespace; entering a blockquote shows every parser-owned `>` prefix in the
  block.
- Nested inline syntax can be revealed in the same frame without creating a
  second transient model.
- Selection in plain paragraphs and ordinary lists continues to use canonical
  cached viewport/layout state.
- List-marker replacement remains an explicit follow-up rather than an
  accidental projection regression.
