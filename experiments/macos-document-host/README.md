# Yu macOS Document Host

这是 Phase 2 的最小 AppKit 文档窗口 host，不是完整 Yu GUI。

它只验证产品壳与 `yu-storage::DocumentSession` 的边界：

- Rust `YuStorageSession` 持有唯一 `DocumentEditorSession`，统一 source、Revision、dirty、BOM、
  editor command、IME composition 和磁盘冲突状态；
- Swift/AppKit 只消费 owned UTF-8 snapshot，作为可丢弃的 `NSTextInputClient`/TextKit source mirror；
- 标题和状态栏显示文件名、Revision、dirty、BOM 与磁盘状态；
- Save、Reload 和关闭前 Save/Discard/Cancel 都回到 Rust storage session；
- 外部修改不会被静默覆盖，冲突关闭只提供丢弃本地修改或取消；
- 普通字符、allowlist 命令和 marked text 都通过同一个 Rust session；`unmarkText` 只改变 native
  presentation，commit/cancel 用 Revision + composition generation 防止迟到回调污染新状态；
- copy/paste/cut/selectAll 都通过 Rust selection/source/command，copy/cut 同时发布
  `net.daringfireball.markdown`、纯文本和 `public.html` payload，三者都来自同一
  Revision-bound source range；paste 优先读取 Markdown source，再读取纯文本，最后才调用 Rust
  strict HTML→Markdown policy；策略拒绝时不注入 HTML，而是回退纯文本；TextKit 不提供独立 undo 或
  canonical source；`⌘Z`、`⇧⌘Z` 以及编辑菜单的撤销/重做都显式路由到 Rust history，避免
  TextKit 的禁用 undo manager 吞掉快捷键；
- Accessibility 文本查询与 Markdown semantic node count/fill 都从同一 Revision-bound Rust
  session 读取；Swift 只保存 owned 节点元数据，`DocumentTextView` 作为可编辑 AX root，并将
  block/inline 节点映射为实现 `NSAccessibilityElementProtocol` 的 children；Heading/Link custom
  rotor 只查询当前 child tree；链接 destination 只暴露 `accessibilityURL`，task checkbox press 回到
  Rust `toggle_task` Transaction；macOS VoiceOver 真实朗读已由人工确认通过；Rotor/语义 action
  的跨平台回归仍属于后续工作；
- 当前包含最小 Rust glyph overlay、CoreText-shaped Rust visual pointer/caret 映射、projected
  selection highlight、Revision-bound caret reveal、CoreText shaped vertical command 和 TextKit
  回退；仍不包含完整 Markdown delimiter reveal、shaped visual IME renderer 或 workspace/tab。

构建并运行：

```bash
experiments/macos-document-host/build-app.sh
open experiments/macos-document-host/.build/YuMacDocumentHost.app
```

也可以直接打开指定的 Markdown 文件：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost /path/to/file.md
```

可以在不启动窗口的情况下验证 AppKit semantic child tree 的 Revision/parent/label 契约：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --accessibility-self-check /path/to/file.md
```

该自检会创建真实 AppKit Accessibility 子节点，验证节点文本和 URL 来自当前 Revision、task checkbox
状态/press、Heading/Link rotor 目标，并在一次 Rust 编辑后确认旧节点不能继续读取新 source；真实
VoiceOver 朗读已由人工确认，Rotor/语义 action 的真实导航仍应在后续版本回归。

验证 native clipboard priority/fallback（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --clipboard-self-check /path/to/file.md
```

该自检使用私有 pasteboard，验证 Markdown > 纯文本 > 受控 HTML 的顺序，以及 HTML policy 拒绝时
不会把脚本等内容写入 Markdown source；同时读取 `Fixtures/clipboard` 中的 semantic mail、GFM
table、browser wrapper 和 unsafe HTML fixture，覆盖接受与拒绝路径。

验证鼠标/拖选使用的 AppKit `setSelectedRanges` 是否同步到 Rust selection（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --selection-self-check experiments/macos-document-host/Fixtures/sample.md
```

验证 native undo/redo 菜单与 Rust history 的桥接（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --undo-self-check experiments/macos-document-host/Fixtures/sample.md
```

该自检通过同一个 `DocumentTextView` 执行一次插入、撤销和重做，确认 TextKit 不建立第二套
history，Rust source、revision 和 redo 状态保持一致。

验证同一个 `DocumentEditorSession` 提供 source-backed Markdown projection 和 source↔visual
caret 映射（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --projection-self-check experiments/macos-document-host/Fixtures/projection.md
```

该自检只验证 projection FFI 契约，不改变生产窗口的 TextKit source mirror；隐藏的 emphasis/link
语法会从 visual 文本移除，但 source UTF-16 caret 必须在当前 Revision 内 round-trip。完整 block
projection、IME 映射和 GPU scene 接入会在该边界稳定后进行。

验证 visual selection 与 metrics-layout point hit-test 的 source↔visual round-trip（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --projection-hit-test-self-check experiments/macos-document-host/Fixtures/projection.md
```

该自检使用 Rust 返回的 visual selection 和 projection-local point，不在 Swift 重建 Markdown delimiter
或布局；stale Revision 会被拒绝，命中结果只携带 layout-local 坐标。生产 pointer adapter 会
复用同一 Revision-bound reverse mapping，失败时回到 source hit-test。

验证生产 pointer adapter 使用 CoreText-shaped Rust block layout 做 point→visual→source 命中（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --shaped-projection-hit-test-self-check experiments/macos-document-host/Fixtures/projection.md
```

该自检先发布同一字体/宽度的 viewport metrics，再用 Rust shaped endpoint 命中 document-space
原点并检查 source/visual round-trip；stale Revision 必须拒绝。生产 TextKit mirror 仍只负责
输入、IME、Accessibility、caret/selection 矩形和 source hit-test 回退。

验证 TextKit 过渡镜像接收 Rust projected UTF-8，并通过 Rust reverse mapping 完成 visual/source
selection、caret 双向 round-trip 与 stale Revision 拒绝（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-mirror-self-check experiments/macos-document-host/Fixtures/projection.md
```

该自检中的 `NSTextStorage`/`NSLayoutManager` 只是一份可丢弃的 visual view cache，用于验证
caret/selection 矩形和 reverse mapping；生产窗口的点击/拖选由上面的 CoreText-shaped Rust
endpoint 命中，仍由 canonical source mirror 接收 IME、复制粘贴和 Accessibility。

验证 parser-owned block 的 metrics layout、macOS CoreText shaped layout 与 block-local caret（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --block-layout-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

该自检同时比较 metrics/CoreText 的 block visual 长度，检查 CoreText line height/caret point，并确认
旧 Revision 的 block layout 查询会被拒绝；LayoutSnapshot 和 CoreText 对象不会穿过 FFI。

验证 parser-owned block projection 的 count/fill、source range、kind、visual 长度以及 stale/
越界拒绝（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --block-projection-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

该自检逐 block 消费 Rust 返回的 owned UTF-8，不在 Swift 重建 Markdown block；当前只建立
block-local projection 诊断边界，生产 TextKit mirror 和完整 visual delimiter 语义保持不变。

验证 macOS CoreText shaped viewport 的 count/fill、可见 block 文档坐标、source range、kind、
measured 标记和 stale Revision 拒绝（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --shaped-viewport-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

该自检先将 CoreText 的 line height/default advance 发布到 Rust viewport policy，再请求一个完整
可见窗口；Swift 只消费 owned block 元数据，不创建第二套 Markdown parser、layout 或渲染树。

验证生产上下移动使用 CoreText shaped block layout、保持 preferred-X/selection Revision，且不
修改 source（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --shaped-vertical-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

该自检通过同一个 storage FFI 发布 viewport metrics，再执行两次 shaped Down；返回值仍是普通
`CommandResult`，stale Revision 会被 Rust 拒绝。生产窗口在键盘命令前同步准备相同 metrics。

验证 Rust `ViewportSceneInput`/`SceneBuilder` 生成的最小 owned scene primitive count/fill、背景/文本
顺序、source range、document-space 矩形和 stale Revision 拒绝（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-scene-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

该自检目前只验证矩形 scene 协议，不连接生产 TextKit、glyph atlas 或 Metal surface；Swift 不推导
block 高度或 Markdown 语义。

验证 CoreText-shaped glyph、CPU atlas page metadata、backend-neutral RenderPlan damage，以及同一
Revision 的 count/fill/stale 协议（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-render-plan-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

该自检只接收 Rust 返回的 glyph command、page fingerprint 和 damage owned scalars；atlas 像素、
CoreText 对象、layout cache 与 Metal texture 不穿过 FFI。产品窗口的可见 overlay 由后续
persistent surface lifecycle 提交；source TextKit mirror 仍保留为输入与回退表面。

验证 macOS document host 复用 Rust-owned CoreText/CPU atlas/publication state，并覆盖重复 frame、
scroll、resize、surface generation 与编辑后的 stale Revision（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

这个 self-check 只返回 lifecycle scalar；它不启动可视化演示模式，也不替换生产 TextKit source
mirror。产品窗口的 native surface lifecycle 已接入，成功提交后会显示最小 Rust glyph overlay；
完整 visual projection、caret 和 hit-testing 仍在后续阶段。

验证真实 AppKit `NSView` → `CAMetalLayer` attachment、drawable acquisition、atlas upload、
retained target blit/present 和 stale Revision 拒绝（显式测试命令，会短暂创建并关闭临时窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-surface-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

该 self-check 只返回 Rust-owned lifecycle scalar；同一个 storage session 会复用 surface、renderer、
atlas、Metal target 和 attachment，重复提交应复用 atlas upload，surface 尺寸变化会推进 generation，
结束前显式 detach。它不是产品可视化演示模式，也不切换生产 TextKit source mirror。

验证产品 document-host 窗口中的 `NSView` lifecycle coordinator。它会在真实 AppKit window 中把
attach、layout/resize、scroll、编辑 Revision 和 close detach 映射到同一个 Rust surface session；
surface view 位于 source TextKit mirror 上方但不参与 hit-test，成功提交后只显示 Rust glyph overlay，
不会开启第二套 source/selection 文档模型：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --macos-render-host-lifecycle-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

生命周期 coordinator 使用 `yu_storage_session_macos_font_metrics`，所以空 Markdown 没有 parser
block 时也可以初始化 CoreText viewport；metrics、surface submit、caret reveal 和 detach 仍受
Revision/main-thread 契约约束。覆盖层提交失败时自动隐藏，输入、IME、caret、selection 和
Accessibility 回到 TextKit；selection highlight 的 visual rectangles 和 scroll target 仍来自
同一 Rust projection/viewport 契约。

验证 persistent host 从 retained scene 导出的 glyph primitive count/fill、atlas placement、
source block range、block 顺序、几何有限性以及编辑后的 stale Revision 拒绝（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-scene-glyph-self-check experiments/macos-document-host/Fixtures/block-projection.md
```

该自检只缓存 Rust 返回的 owned glyph metadata；atlas 像素、CoreText/layout/scene 对象和 Metal
handle 不穿过 FFI。当前 source range 是所属 block 的 UTF-16 范围，因此同一 block 的多个 glyph
可以共享该范围；生产窗口仍保留 source TextKit mirror 作为输入/AX/回退表面，native surface
overlay 在其上方消费同一 Rust-owned publication。

验证 marked-text composition 的 transient projection、visual selection、active caret 与
Revision/generation 生命周期（不启动窗口）：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --composition-projection-self-check experiments/macos-document-host/Fixtures/projection.md
```

该自检使用 Unicode/emoji preedit，确认 update 后旧 generation 被拒绝，cancel 不推进 source
Revision 且恢复原文；Rust `CompositionOverlay` 是唯一 transient source/visual 映射来源。

无路径启动时会弹出文件选择器。窗口中的 `DocumentTextView` 可以接收普通字符和系统
`NSTextInputClient` marked text，但它只是 Rust canonical source 的可丢弃镜像，不拥有独立
source、dirty 或 history。
