# ADR 0091：macOS 产品壳状态投影与外部文件监听

## 状态

已接受（2026-08-14）

## 背景

可写 native mirror 已经把字符输入、IME、命令、选区和基础纯文本剪贴板接回统一的
`DocumentEditorSession`。下一步需要让产品窗口对外部文件变化、保存/重载动作和辅助功能状态有
稳定行为，但不能让 AppKit watcher 或菜单重新拥有一份 dirty、revision 或文件指纹逻辑。

## 决策

macOS host 使用目录级 `DispatchSourceFileSystemObject` 监听文档所在目录，而不是只监听文件 inode：

```text
directory vnode event
          │
          ▼
150 ms main-thread coalescing
          │
          ▼
YuStorageSession::state / disk_state
          │
     ┌────┴─────┐
     │          │
   clean       dirty
     │          │
 reload       status + conflict prompt
```

监听器只产生“需要检查”的信号。它不读取文件、不更新 source，也不决定是否覆盖或重载；Rust
session 的 fingerprint、dirty 和 close state 仍是唯一权威。监听目录可以覆盖同目录临时文件
写入后的 atomic rename，不依赖旧文件描述符继续代表新 inode。

窗口层提供最小的状态投影：

- 保存菜单和按钮只在 Rust `dirty` 时启用；
- 重新加载只在 clean 且 Rust 报告磁盘变化时启用；
- 标题继续来自 Rust-owned path，状态栏展示 dirty、Revision、disk state 和 BOM；
- `NSTextView` 作为可丢弃 mirror 暴露 text-area role、value，并在 source/selection 变化时发送
  `.valueChanged` 与 `.selectedTextChanged`；
- copy/paste/cut/selectAll 菜单动作仍调用 native mirror 的 Rust-backed 实现。

外部变化的产品行为固定为：clean 文档可以确认后 reload；dirty 文档只提示冲突，不自动丢弃本地
修改，也不通过 watcher 静默覆盖磁盘。

## 取舍

本阶段不实现完整 VoiceOver 语义树、跨平台 Accessibility、目录级事件的共享 FFI 或 Markdown/HTML
富剪贴板。host 内的 150ms 合并仅用于降低提示抖动；共享 `FileWatchDebouncer` 仍保留在 Rust
与 macOS adapter 中，未来真正跨平台 watcher 接入时继续复用。

## 验证

- `swift build --package-path experiments/macos-document-host`
- `experiments/macos-document-host/build-app.sh`
- `codesign --verify --deep --strict --verbose=1 .../YuMacDocumentHost.app`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
