# ADR 0104：macOS Accessibility semantic URL 与 task actions

## 状态

已接受（Phase 2，macOS host）。本 ADR 固定 source/action 边界；真实 VoiceOver 手势、URL 打开
策略和跨平台 action adapter 仍需人工/平台验收。

## 背景

语义 child 已经可以被 VoiceOver 读取和通过 Rotor 定位。下一步必须明确哪些 AX 属性可以只读暴露，
哪些操作可以改变 Markdown source，避免 Swift 在语义层偷偷维护另一份文档或直接改写文本。

## 决策

- Rust `AccessibilitySemanticNode` 为 link/image 解析结果携带可选 destination UTF-16 range；
  reference link 在 Rust parser 的 definition index 中解析到同一 Revision 的 definition destination。
- 既有 `yu_storage_session_accessibility_semantic_nodes` 与
  `YuStorageAccessibilityNode` 的 C ABI 保持不变；destination/action 元数据通过显式的
  `YuStorageAccessibilityNodeV2` 和 `yu_storage_session_accessibility_semantic_nodes_v2` 获取，避免
  已有原生调用方因结构体尾部变化而发生 ABI 破坏。
- macOS link/autolink/reference-link child 暴露 `accessibilityURL`。Swift 只按 expected Revision
  回查 URL 字符串并构造 `URL`；不在 Swift 重新解析 Markdown，不自动调用 `NSWorkspace.open`，也不
  允许 URL 属性隐式产生外部副作用。email autolink 在没有 scheme 时映射为 `mailto:`。
- Rust 为 task-list semantic node 携带对应 Markdown block index。macOS task child 的
  `accessibilityPerformPress` 只有在 node Revision 仍是当前 Revision、没有 composition overlay、且
  `toggle_task` command 可用时才执行；执行结果必须是普通 source Transaction，进入同一 Undo/dirty/
  Revision 链路。
- press 成功后 host 使用 command result 同步 source mirror，重建 semantic child tree，并让旧 child
  的 source-backed label/value 失效；press 失败或节点过期返回 `false`，不进行 fallback 文本替换。
- link press、image action、task 的独立鼠标 overlay、URL 安全策略和跨平台 action protocol 不在本
  阶段实现；它们需要产品层明确导航/沙箱/权限策略后再接入。

## 验证

```bash
cargo test -p yu-editor -p yu-storage-ffi
experiments/macos-document-host/build-rust-ffi.sh

CLANG_MODULE_CACHE_PATH=/private/tmp/yu-clang-cache \
SWIFTPM_MODULECACHE_OVERRIDE=/private/tmp/yu-swiftpm-cache \
swift build --package-path experiments/macos-document-host

experiments/macos-document-host/.build/arm64-apple-macosx/debug/YuMacDocumentHost \
  --accessibility-self-check experiments/macos-document-host/Fixtures/sample.md
```

self-check 验证 destination URL、todo/done checkbox 的 press 会推进 Revision、旧 child 失效、刷新后
状态反转，以及无 task 的普通 Markdown 文件不会误触发动作。

## 后续

真实 VoiceOver 会话中记录 checkbox press 的朗读/焦点反馈和链接 URL 的可发现性；在确定 URL 打开策略
后，再增加受控的 link action，并为图片、表格和扩展 block 设计相同的 source-backed action contract。
