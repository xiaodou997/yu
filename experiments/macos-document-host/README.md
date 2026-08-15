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
  canonical source；
- Accessibility 文本查询与 Markdown semantic node count/fill 都从同一 Revision-bound Rust
  session 读取；Swift 只保存 owned 节点元数据，`DocumentTextView` 作为可编辑 AX root，并将
  block/inline 节点映射为实现 `NSAccessibilityElementProtocol` 的 children；Heading/Link custom
  rotor 只查询当前 child tree；链接 destination 只暴露 `accessibilityURL`，task checkbox press 回到
  Rust `toggle_task` Transaction；macOS VoiceOver 真实朗读已由人工确认通过；Rotor/语义 action
  的跨平台回归仍属于后续工作；
- 不包含 Markdown visual projection、最终渲染或 workspace/tab。

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

无路径启动时会弹出文件选择器。窗口中的 `DocumentTextView` 可以接收普通字符和系统
`NSTextInputClient` marked text，但它只是 Rust canonical source 的可丢弃镜像，不拥有独立
source、dirty 或 history。
