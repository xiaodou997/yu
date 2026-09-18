# Mac native editor v3

> 2026-09-18 平台与界面基线已更新：macOS 26+ / arm64，系统 AppKit 外观与 Yu 默认主题。当前改造及验收状态见 [Mac 26+ 现代化](mac-modernization-26.md)；下文旧平台记录保留为历史证据。

Status: implementation in progress. This document supersedes conflicting v2
implementation constraints for the macOS product. No old ABI or runtime editor
fallback is required. Markdown bytes, transactions, safe file persistence and
the prohibition on browser rendering remain requirements.

Current implementation, validation evidence and remaining gaps are recorded in
[mac-native-v3-status.md](mac-native-v3-status.md). No milestone below is
complete merely because one of its implementation steps has landed.

## Architecture

Swift/AppKit owns native UI and NSTextInputClient. Rust owns document editing,
source projection and a hierarchical PresentationTree. CoreText lays out styled
paragraphs; its positioned output is authoritative for drawing and hit testing.
One layout snapshot and coordinate transform serve the renderer, selection,
IME, scrolling and accessibility. Metal paints that output without relayout.

ParagraphInput carries an explicit Auto/Ltr/Rtl base direction from LayoutConfig
into CoreText's paragraph style. Direction is part of the paragraph cache key.
Mac prose uses Ltr to match the fixed reference themes; native run directions
remain automatic within that embedding. Source bytes are never changed to add
direction-control characters.

Native objects remain within their platform owner. Font identities survive
worker/publication boundaries. Immutable snapshots share text and parsed
structure; layout workers must not reparse cloned editor documents.

The implementation separates transaction/history ownership (`EditorState`) from
source-bound paragraph preparation (`LayoutContext`). The live editor alone
changes source, selection and composition input. Workers receive shared
immutable input and have no editor command or history API. Published paragraph
caches are rejected if document identity, revision or projection state changed.
The published `LayoutSnapshot` now freezes source identity/revision, visual
projection, viewport parameters, resource geometry version, measured block
positions and shared paragraph layouts. Rasterization, scene construction and
Mac source/caret/composition/hit/table queries read those same paragraphs.
Changing search decorations reuses geometry. Cached images are invalidated when
known intrinsic dimensions change, including already-ready images.

Paragraph-local to document-content coordinates are converted by snapshot block
placement. Swift applies the shared theme's reading-column padding, scroll/view
and screen transforms once. Caret height comes from the actual native text line,
including a cell paragraph within a taller table row. A source-addressed query
measures its target when outside retained coverage; it never guesses a separate
paragraph origin. Offscreen prefixes can still use height estimates.

Snapshot preparation checks cancellation between paragraphs, also during syntax
reveal and IME composition, and publishes only completed geometry. Background
publications are rejected after source, projection, width or resource changes.
Numerical measurement transfer is private to the layout context; frame workers
publish the exact immutable snapshot used to paint the frame.

The remaining coordinator work includes active-paragraph priority, merging
independent background measurements, progressive resize and source-anchored
scroll stabilization. Selection changes can still invalidate geometry when the
visual projection would be unchanged; that optimization is not yet complete.

Container layout now follows the PresentationTree ancestry of semantic leaves.
Paragraph margins collapse through the branches meeting at a common container;
only direct paragraphs of tight list items suppress their paragraph margins.
Snapshots publish measured quote/list/item bounds with coverage flags. Quote
borders cover internal paragraph gaps and use the same theme advances as text.
Ordered item numbers and structural context fingerprints are computed with the
presentation tree. Caches retain source-shifted paragraphs while rejecting
changed container ancestry, tightness or numbering. Container-local table,
image and code box layout remains in progress.

Table navigation and presentation share parser-owned cell ranges through
table_for_block, including quote/list prefixes. Tab at the final visible cell
appends one source row in a separate undo group, preserving existing Markdown
spelling, alignment markers and line endings. Column dragging remains transient
layout state; saving must not serialize geometry by reformatting untouched rows.

## Delivery ledger

- [ ] M0: fixed Typora 1.10.8 Github/Night reference and acceptance fixtures.
- [ ] M1: native paragraphs, unified geometry, input/edit/save vertical slice.
- [ ] M2: hierarchical document layout, tables, images and live theme tokens.
- [ ] M3: NSView input host, compact native sidebar/find/file/window UI.
- [ ] M4: native math/diagrams, bounded HTML, writing modes and exports.
- [ ] M5: release performance, real-window visual and input acceptance.

Unchecked means unverified or unfinished. Builds and model-level tests alone
never establish visual parity, real IME behavior or trackpad frame cadence.

## Acceptance

As clarified by the user on 2026-09-18, Typora is a reference for core writing
experience, not a pixel-equivalence target. Yu may use its own typography,
spacing and visual identity. A complete Typora screenshot matrix, identical
wrapping/line counts, a ≤1pt reference-boundary threshold, and a dedicated real
1× display session are no longer release or P1 completion gates.

Review representative 900×620, 1200×800 and 1600×1000 point windows, light/dark
appearance and sidebar states for readable hierarchy, consistent spacing,
clipping, overlap and visible jumps. Preserve existing screenshots and diffs as
regression evidence. Product hit-testing, caret, selection, IME, scrolling and
source-preservation contracts remain required; this change does not relax
internal geometry consistency or turn programmatic input into real IME evidence.
The current P1 requirements and evidence are maintained in
[p1-experience-acceptance.md](p1-experience-acceptance.md).

Release targets on the fixed Mac: input-to-visible p95 ≤32ms, 60Hz scrolling
dropped frames <1%, 1MiB resize-visible p95 ≤100ms. Unfocused settled CPU ≤0.5%
over 60 seconds. Physical footprint targets: 100KiB text ≤150MiB, 1MiB text
≤250MiB. Glyph cache ≤32MiB, decoded images ≤128MiB. A 5MiB stress document
must remain cancellable, editable and savable. These are targets, not results.

Math and diagrams use bundled native helper code, started on demand and stopped
after 60 seconds idle. No WebView, Chromium or Node runtime. Arbitrary HTML/CSS,
scripts and direct Typora CSS-theme compatibility are outside the product
contract. Unknown source constructs stay visible/editable and are never lost.

Windows UI work is outside this iteration. Shared Rust code stays portable;
existing cross-platform checks remain useful and are not silently disabled.


## P0 实施补充：布局协调与发布

Mac 帧准备由会话统一捕获不可变呈现输入，工作线程先测活动源码段落与视口，再执行有限的远处段落批次。每批最多 32 段、4ms 软预算，在段落之间检查取消；按段落完成测量，批次结束才构建布局快照。`layout_pending` 经 FFI 通知宿主继续调度，归零后停止；它与图片资源等待状态独立。

前后台缓存与缺失高度合并。若前台已测量的前缀使工作线程产出的块原点过期，保留有效测量并重新准备帧，不把旧像素与新命中坐标组合。发布序号下限在每个任务开始时应用于实际发布器，取消不能绕过它。

窗口缩放保留原块高度作为估计，优先替换视口测量；Swift 持有绑定源码版本的阅读锚点，使用 Rust 光标几何恢复相对视口位置。连续重排期间不能用新的顶部行首替换原锚点。相同投影复用字形，预编辑内部选区只更新映射。

P0 验收结果见 `mac-native-v3-status.md` 与 仓库中的 `artifacts/mac-native-v3/p0/validation.json`。该验收不改变 M0–M5 的最终完成条件。
