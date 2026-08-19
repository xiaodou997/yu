# ADR 0174: Source-backed task checkbox scene

## Status

Accepted.

## Context

Task-list projection intentionally hides the parser-owned `[ ]`/`[x]` marker. Once the macOS
Metal surface takes visual ownership, leaving that hidden range without a replacement makes the
task state disappear even though the native coverage gate reports the block as supported.

## Decision

`yu-workspace` resolves each visible task marker from the canonical Markdown block and positions a
checkbox at the marker's projected caret boundary. It publishes source-backed
`TaskCheckboxPrimitive` layers into `yu-scene`: an unchecked item has border/interior layers, while
a checked item has an accent border and check layers. Every layer retains the exact parser marker
range and the current source Revision.

`yu-render` lowers these layers to existing solid-fill commands. The macOS backend therefore uses
its established Metal rectangle pipeline and does not parse Markdown or add a new shader. The
diagnostic FFI exposes task layers with a distinct command tag even though the backend-neutral
render operation remains a solid fill.

## Consequences

Task status remains visible when the Rust surface owns glyph rendering, and toggling the canonical
marker naturally rebuilds the checkbox on the next Revision. TextKit remains the input, IME,
Accessibility and failure-fallback host. Pointer activation of the visual checkbox remains a
separate interaction contract; this decision covers retained visual publication only.
