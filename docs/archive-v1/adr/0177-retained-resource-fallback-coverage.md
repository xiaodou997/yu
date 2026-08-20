# ADR 0177: Retained resource fallback coverage

## Context

The macOS visual coverage gate kept TextKit source ranges visible whenever an
image or embedded renderer was pending, failed or explicitly unsupported. That
was safe but duplicated production painting: every image scene already carries
an opaque placeholder, and fenced embedded blocks retain projected source
glyphs when no SVG publication is available.

This duplication also obscured ownership. A renderer failure should select a
defined retained fallback, not silently reactivate a second text renderer for
the same supported block.

## Decision

1. Image and embedded C status tags are normalized through separate mappings;
   numeric equality between those two ABI domains is not a host contract.
2. Resource coverage is evaluated only after the current Revision, supported
   render command mask, parser block mask and viewport block snapshot match.
3. A ready image or embedded resource is covered by its uploaded texture.
4. Pending or failed images are covered by their source-backed
   `ImagePrimitive` placeholder. They may request a bounded refresh but do not
   add a TextKit source fallback range.
5. Pending, failed or unsupported embedded resources are covered by the
   retained fenced-code projection. Mermaid remains explicitly unsupported;
   users see its source projection rather than a blank result or TextKit
   duplicate.
6. An unknown image with fingerprint zero keeps the projected alt-label glyphs
   already present in the retained layout because no image primitive identity
   was created.
7. An unknown resource with a non-zero fingerprint, or an unknown status tag,
   makes coverage incomplete. The whole visual surface fails closed until the
   host can classify that state.

## Consequences

- Known resource lifecycle states no longer reactivate TextKit production
  painting for otherwise supported blocks.
- Image failures remain visible as deterministic placeholders, and unsupported
  Mermaid remains editable as projected source.
- Retry scheduling and resource diagnostics are unchanged.
- Unknown ABI/resource states still prefer a complete source mirror over a
  potentially incomplete Metal publication.
- ADR 0178 removes the transitional local TextKit range path entirely; this
  ADR's resource classifications feed an all-or-nothing retained coverage gate.
