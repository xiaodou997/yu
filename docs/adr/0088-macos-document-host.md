# ADR 0088：macOS 最小文档窗口 host

## 状态

已接受（Phase 2，最小产品壳验证）

## 背景

`yu-storage::DocumentSession` 已经固定了 UTF-8 文档、Revision-bound dirty、BOM、外部文件
冲突和 close prompt。现有 `experiments/macos-text-input` 则验证了 AppKit 输入/IME 风险，
但它是固定文本的 spike，并不拥有文件生命周期。直接把两者拼进一个可编辑 `NSTextView` 会
让 Swift 同时拥有一份 source，重新引入长期架构要避免的双写和状态分裂问题。

## 决策

新增 `yu-storage-ffi`，把唯一可变 `DocumentSession` 放在 Rust-owned
`YuStorageSession` 中。ABI 只提供：

- 打开/销毁 session；
- owned path/source snapshot 的长度查询和复制；
- Revision、saved Revision、dirty、BOM、磁盘状态和 close 状态；
- save、reload、close request、save/discard/cancel close。

`experiments/macos-document-host` 使用一个极薄的 AppKit 壳来验证：

- 从命令行路径或 `NSOpenPanel` 打开 Markdown 文件；
- 只读 `NSTextView` 显示 Rust source snapshot；
- 标题和状态栏显示文件名、Revision、dirty、BOM 与磁盘状态；
- 保存、重载和窗口/应用退出前的 close prompt 都回到 Rust session；
- 外部修改不会静默覆盖，冲突时只能丢弃本地 session 或取消关闭。

这个 host 明确不是完整编辑器：文本视图保持只读，不实现 Markdown Source Projection、完整
`NSTextInputClient`、workspace/tab、文件 watcher 消费、最终 Metal renderer 或 Accessibility
树。窗口也不复制 parser、selection、history 或 source。

## 结果

产品壳已经有一个可编译的生命周期竖切片，同时保留了后续把编辑和存储合并到同一 Rust
session 的空间。Swift 端只处理窗口、提示和 owned snapshot，storage/dirty/conflict 规则仍
集中在 Rust；长度查询 ABI 允许 native 端分配自己的缓冲区，不让 Rust 的 buffer 指针逃逸。

代价是当前 host 不能编辑文档，也不能通过该实验验证 IME。下一步必须定义一个同时持有
`DocumentSession` 与 `EditorDocument` 的单一 Rust 可变会话，再把已有 `yu-editor-ffi` 的
command/composition 协议接到它上面；在此之前不应把 `NSTextView` 改成可写。

## 验证

```bash
cargo test -p yu-storage-ffi
experiments/macos-document-host/build-app.sh
```

构建产物位于实验目录下的 `.rust/` 与 `.build/`，由 `.gitignore` 忽略，不提交到仓库。
