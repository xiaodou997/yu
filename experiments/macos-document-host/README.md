# Yu macOS Document Host

这是 Phase 2 的最小 AppKit 文档窗口 host，不是完整 Yu GUI。

它只验证产品壳与 `yu-storage::DocumentSession` 的边界：

- Rust `YuStorageSession` 持有唯一 `DocumentEditorSession`，统一 source、Revision、dirty、BOM、
  editor command、IME composition 和磁盘冲突状态；
- Swift/AppKit 只消费 owned UTF-8 snapshot，作为只读 `NSTextView` source mirror；
- 标题和状态栏显示文件名、Revision、dirty、BOM 与磁盘状态；
- Save、Reload 和关闭前 Save/Discard/Cancel 都回到 Rust storage session；
- 外部修改不会被静默覆盖，冲突关闭只提供丢弃本地修改或取消；
- FFI 已经提供统一 session 的 command/selection/key route/composition 入口，但当前 host 仍不调用
  可写路径；不包含 Markdown visual projection、完整 NSTextInputClient 或 workspace/tab。

构建并运行：

```bash
experiments/macos-document-host/build-app.sh
open experiments/macos-document-host/.build/YuMacDocumentHost.app
```

也可以直接打开指定的 Markdown 文件：

```bash
swift run --package-path experiments/macos-document-host YuMacDocumentHost /path/to/file.md
```

无路径启动时会弹出文件选择器。窗口中的文本视图暂时只读，这是刻意的：在正式编辑器 FFI
接入前，不能让 AppKit `NSTextView` 变成第二个可变 source。
