# Yu native renderer patches

Vendored from crates.io mermaid-rs-renderer 0.3.1, upstream commit
`2f993bd79a55235eb59a34d807852276ba25bea7`. Upstream:
https://github.com/1jehuang/mermaid-rs-renderer

MIT license preserved in LICENSE and bundled as Mermaid-MIT.txt.
Cargo uses this source through the workspace crates.io patch.

The strict entry point now checks unrecognized flowchart, sequence and class
statements at the parser branch that would otherwise silently discard them.
The unrecognized-statement checks are limited to the strict entry point. Strict class parsing
also rejects an unclosed body. This is not yet complete grammar validation:
partial consumption inside recognized declarations still requires auditing.

Run the helper library tests and the vendored library tests without default
features when changing these patches. Preserve upstream provenance and license.

The crates.io archive omits these upstream test fixtures. Restored unchanged
from the same pinned commit (SHA256):

- `docs/comparison_sources/er_blog.mmd`: `0da9ad4cb6cb3a350702b8d7941bccadacd38169149efdf1960b2d2ea0edb075`
- `docs/comparison_sources/flowchart_cycles.mmd`: `a521a3fd4dca63247b44b2a945343f549eeec65120c87366d0a3afc60f2834be`
- `docs/comparison_sources/flowchart_dense.mmd`: `58aa699aae999e22c5ce76e9fd347b09950a48b7b511d4d8fb4d84e440633ba2`
- `docs/comparison_sources/flowchart_opaque.mmd`: `6436bd1c9f897bf4387e3cfa67be332797104ae6fead6faeb271ada6d75af783`
- `docs/comparison_sources/flowchart_subgraph_dir.mmd`: `0108d4cea98754a9c001f7cf01d09d6f81536b9365a1df6872853448298cb97c`

Validation commands from the Yu workspace root:

```sh
cargo test -p yu-document-renderer --lib
cargo test --manifest-path vendor/mermaid-rs-renderer/Cargo.toml --lib --no-default-features --locked --target-dir target
```

On the current macOS 27 host, both this copy and an unmodified upstream
parser/library baseline report 323 passing tests and one failure:
`layout::tests::cycle_fixture_subgraph_entry_aligns_with_spine` (four route
points instead of two). The assertion was kept unchanged. A subsequent patch fixes its cause:
routing used an obsolete left-aligned subgraph title obstacle although SVG
rendering centers that title. `SubgraphLayout::label_bounds` now supplies both
render placement and routing obstacles, including state header placement.
The patched library now passes all 324 tests.

Class members now create their owning node when declared with `Class : member`,
and class bodies retain members after the opening brace on the same line.
The strict entry rejects unconsumed suffixes after closing class braces, while
allowing a terminal semicolon. Helper tests cover explicit/implicit classes,
opening-line members, both themes and invalid suffixes.

Strict sequence parsing now validates branch context: `else` needs `alt`,
`and` needs `par`, and `option` needs `critical`. Missing or mismatched frames
produce diagnostics rather than silently discarded branch markers. Nested
valid frames are exercised by the Yu helper tests in both themes.

Sequence numbering is now stored per message instead of as a final global flag.
Exact hundredths support start/increment, reset and off commands without binary
floating point drift; invalid parameters and overflow are diagnostics. Number
badges carry measured bounds through layout, SVG rendering, collision avoidance
and final extent calculation. The old global numbering field was removed.
The existing off test now checks the equivalent per-message disabled state.

Quoted pie entries share one public parser with Yu validation. It consumes the
quoted string before the numeric separator, decodes escapes and rejects
non-finite/negative f32 data. Comment scanning respects escaped quotes in both
preprocessors. Pie totals, ratios and value formatting use f64 intermediates
so finite large f32 weights cannot silently collapse to zero-angle slices.

Quoted flowchart labels now pass through a shared logical-line projection before
both typed validation and parsing. Physical newlines inside quotes become native
label line breaks; padding after the logical statement preserves following source
line numbers. Quoted `end`, `%%`, and `click` text is retained rather than treated
as declarations or comments. Yu's host validator uses the same projection and
still rejects actual interaction directives. CRLF, empty label lines, metadata,
node/edge/subgraph labels and both themes are covered. This patch does not claim
Markdown label emphasis support; rich label spans require separate work.

Native-window review then exposed positioned SVG tspans being flattened by the
AppKit image importer. Common TextBlock and indexed class/ER label output now
uses one absolute-position text element per line, retaining original baseline,
line-height, anchor and weight. Quoted pipe edge labels also strip their enclosing
quotes before measuring/rendering. The first light/dark window captures are
retained as failed visual evidence, not counted as passing on source checks alone.
Other specialized tspan emitters still require their own native visual audit.

The remaining specialized multiline emitters for Sankey values, C4 text and
GitGraph branches now use absolute text coordinates as well. GitGraph draws
exactly the same split lines as its measurement path, strips quoted branch
names consistently in branch/checkout/merge, and no longer subtracts half the
label height twice when placing the branch background. Its default background
Y offset now matches the text top/padding; custom offsets remain applied.
C4 context shapes (Person/System families) map their third positional argument
to description, with subsequent sprite/tags/link slots shifted accordingly;
Container/Component technology slots remain unchanged. The signatures are
checked against https://mermaid.js.org/syntax/c4.

C4 centered text is emitted with an explicit alphabetic baseline rather than
relying on imported dominant-baseline. Sankey's label group explicitly inherits
the theme font family. Native AppKit NSImage previews before/after are retained
for these fixes; SVG serialization alone is not visual acceptance.

GitGraph init mainBranchName/mainBranchOrder now initialize the actual root
branch identity and order before commands are parsed. Incorrect types, empty
names and non-finite orders produce diagnostics. Branch targets are separated
from command attributes: order no longer becomes part of a new branch name,
and merge id/tag parameters no longer prevent the source branch being found.
Branch/checkout reject unconsumed suffixes. Rendering tests also verify the
merge's two parent IDs and actual branch ordering, not just visible names.
Other init display settings still need integration; this does not claim all
Mermaid configuration is supported.

Quoted branch command targets decode with the same JSON5 string rules used by
init configuration, so escaped quotes/newlines cannot create a second identity
for the configured root or a previously declared branch. Dedicated tests cover
this equality and merge parent preservation.

GitGraph source layout configuration is now resolved through one atomic native
method, shared by root parsing, Yu's helper and public render/measure/timing
entry points. Supported options are mainBranchName, mainBranchOrder,
showBranches, showCommitLabel, rotateCommitLabel, parallelCommits, useMaxWidth
and diagramPadding. Invalid types, non-finite values, negative padding and
unknown GitGraph keys are diagnostics. Partial overrides retain host settings;
a failed override does not mutate the destination. Hidden branch labels no
longer contribute to bounds. Tests check actual parallel commit coordinates,
label removal, width reduction and the library entry points as well as Yu output.
This remains scoped to GitGraph init options; root-level and other diagram
configuration, including YAML frontmatter, still need separate support/audit.

Strict state/ER parsing now diagnoses unclosed bodies, unmatched state braces,
root-level concurrency separators, unknown statements and discarded suffixes
after ER bodies. ER delimiters inside quoted descriptions are not structural.
Standalone state identifiers and multiline notes are retained. Note bodies are
protected from generic directive/arrow validation and from preprocessing that
would remove blank lines or %% text; notes are consumed before transitions so
literal arrows/braces cannot create nodes or edges. This is a structural audit,
not a claim that all state/ER attributes and styles are fully consumed.

State edge heads now use explicit positioned paths, preserving the original
concave shape and routing tangent, instead of SVG markers not painted by AppKit.
Start arrows use the reversed endpoint tangent; obsolete state marker definitions
and references are removed. Native light/dark images cover TB/BT/LR/RL with
arrows at both endpoints. Other marker-dependent diagram families still require
their own audit; this patch is not whole-SVG-importer compatibility proof.
