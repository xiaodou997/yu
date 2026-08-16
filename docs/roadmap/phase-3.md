# Phase 3：Source Projection & Native Layout

## 目标

在不复制 Markdown source、selection、IME 或 history 的前提下，把 Rust 的 source-backed
projection、layout 和 hit-testing 接入 macOS native editor。阶段初期只建立 Revision-bound
FFI 与诊断边界；当前已进入最小可见 RenderPlan overlay，但 TextKit 仍保留为输入、IME、
Accessibility、caret/selection 和失败回退表面。完整 visual mirror 仍需后续逐步替换。

## Track A：Projection bridge

- [x] `DocumentEditorSession` 暴露 inline projection 的 owned snapshot
- [x] macOS storage FFI 提供 expected-Revision projection UTF-8 count/fill
- [x] macOS storage FFI 提供 source UTF-16 ↔ visual UTF-16 caret round-trip
- [x] Swift/AppKit projection self-check 覆盖 strong/emphasis/link delimiter 与 Unicode caret
- [x] macOS storage FFI 按 parser-owned block index 暴露 source range/kind/visual lengths 与 UTF-8 snapshot
- [x] Swift block projection self-check 覆盖 heading/task/fenced-code、Unicode 和 stale/out-of-bounds
- [x] macOS storage FFI 暴露 generation-bound composition projection、visual selection 与 marked caret
- [x] composition projection metadata 暴露 visual replacement range，供 visual IME overlay 绑定
- [x] Swift composition projection self-check 覆盖 Unicode preedit、update stale generation、cancel/source 保持
- [ ] 完善 heading/list/task/fence/table 的 visual delimiter 语义，并统一 block projection kind
- [x] visual selection range、metrics hit-testing 和 point↔source mapping 的 Revision-bound 诊断契约
- [x] stale Revision/generation 在 native projection callbacks 上的全路径回归（含视觉 scene/glyph/render-plan count/fill header）

## Track B：Native layout

- [x] parser-owned block-scoped projection snapshot、惰性 layout metadata 与 block-local caret
- [x] macOS CoreText shaping metrics 与 `yu-layout` line/caret contract 对齐
- [x] shaped viewport snapshot、block origin/height 与可见窗口 count/fill
- [x] storage FFI shaped viewport header 暴露 scroll/viewport/max-scroll 坐标协议
- [x] storage FFI shaped caret scroll target 与 visual viewport transform self-check
- [x] TextKit 过渡镜像自检支持 visual/source 双向映射（生产 view 尚未切换）
- [x] `DocumentTextView` visual pointer adapter 与 source-mirror fallback self-check
- [x] `DocumentTextView` opt-in visual IME composition mirror、marked range 与 attributed substring self-check
- [x] 生产 visual view 启用点击/拖选 visual boundary→Rust source selection，以及 source→visual caret 映射
- [x] 生产 visual view 的 projected selection highlight 与同一 Revision 的 shaped caret reveal
- [x] 生产 Up/Down/Shift-Up/Shift-Down 使用当前 CoreText metrics/shaper 的 Revision-bound command
- [x] 生产 pointer adapter 使用同一 CoreText-shaped Rust block layout 命中 visual boundary；TextKit 只保留输入/IME/AX/矩形回退
- [x] visual IME active caret 使用 Revision + composition generation-bound CoreText shaped block geometry
- [x] visual IME composition hit-test 使用 Revision + composition generation-bound transient projection，覆盖跨 block document-space mapping
- [x] visual IME preedit 在所属 block 使用 CoreText shaped glyph、CPU atlas 与持久 Metal surface 发布
- [x] visual IME preedit 的跨 block transient layout：按受影响 block span 投影、重测 viewport 高度并进入持久 RenderPlan/Metal publication
- [x] macOS RenderPlan 将 document-space scroll origin 传入 Metal viewport，统一滚动后的 glyph/damage 坐标变换（仍保留 TextKit 字形回退）
- [ ] 完整 visual renderer 迁移（移除 TextKit source mirror 的生产渲染职责）

## Track C：Scene and rendering

- [x] Rust `ViewportSceneInput`/`SceneBuilder` 生成 Revision-bound 最小 owned scene snapshot，macOS host 以 count/fill 自检 primitive 顺序、来源范围、坐标和 stale 丢弃（诊断桥，尚未替换生产 renderer）
- [x] Rust 使用 CoreText glyph rasterization、CPU `GlyphAtlas` 与 `yu_workspace::assemble_viewport_render_frame` 生成 Revision-bound RenderPlan；macOS host 以 count/fill 自检 glyph command、atlas page fingerprint、damage 和 stale 丢弃（诊断桥，尚未接入生产窗口）
- [x] `yu-render-macos` 新增持久 `CoreTextViewportFrameBuilder`，重复 Revision 重用 CPU atlas/RenderPlan fingerprint；ignored AppKit probe 使用真实 CoreText publication 进入 `MetalAtlas`/retained target（生产窗口仍未切换）
- [x] persistent macOS host 通过 count/fill ABI 暴露 Revision-bound retained glyph primitives（含 atlas placement、metrics、bounds 与 source block range；生产 view 仍未切换）
- [x] macOS document host opt-in surface-submit self-check 使用临时 AppKit `NSView` 完成 `CAMetalLayer` attachment、drawable、atlas upload 与真实 Metal submit（生产窗口仍保留 source mirror）
- [x] persistent native surface adapter 复用同一 view 的 Metal surface/renderer/atlas，覆盖重复提交、resize generation、显式 detach 与 stale Revision（生产窗口仍未切换）
- [x] product `NSView` surface lifecycle coordinator 接入 attach/layout/resize/scroll/edit/close，空文档通过 CoreText metrics FFI 初始化 viewport（source TextKit mirror 仍可见）
- [x] macOS document host 诊断桥持有 persistent CoreText/atlas/publication host；编辑、scroll、resize 的 frame serial、surface generation 和 stale Revision self-check
- [x] macOS native GPU surface 在 ignored AppKit probe 中消费 Rust-owned CoreText workspace publication（生产窗口仍保留 source mirror）
- [x] active composition 的 transient block layout/glyph atlas 进入同一持久 RenderPlan 与 Metal submit，Swift submit key 绑定 composition generation
- [x] heading、emphasis、code、link 的最小真实 visual render 通过产品窗口 persistent Metal
  surface 可见提交；TextKit 仍保留为透明 overlay 下的输入/AX/回退表面

## 约束

1. Markdown source、Revision、selection、composition 和 history 仍只由 Rust
   `DocumentEditorSession` 持有。
2. Swift 不解析 Markdown，不根据 delimiter 自行推导 source range。
3. 任何 visual snapshot、layout、scene 和 glyph cache 都必须携带 Revision；过期结果不得提交。
4. 完整 visual mirror 接入前，现有 native source mirror 必须继续可用，确保 IME、复制粘贴和
   Accessibility 有安全回退路径。
5. visual IME overlay 只能消费同一 Revision + composition generation 的 Rust projected text 和
   visual replacement range；generation 失效时必须回到 canonical source mirror。
6. visual viewport 的 block `y`、caret `y` 和 scroll target 都是同一 Revision 的 document-space
   坐标；Swift 只能使用 header 提供的 scroll transform，不能复制 HeightIndex 或自行推导高度。
