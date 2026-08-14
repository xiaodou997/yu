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
  Revision-bound source range；paste 优先读取 Markdown source；TextKit 不提供独立 undo 或
  canonical source；
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

无路径启动时会弹出文件选择器。窗口中的 `DocumentTextView` 可以接收普通字符和系统
`NSTextInputClient` marked text，但它只是 Rust canonical source 的可丢弃镜像，不拥有独立
source、dirty 或 history。
