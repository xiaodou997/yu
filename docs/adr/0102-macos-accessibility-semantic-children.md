# ADR 0102：macOS Accessibility semantic child elements

## 状态

已接受（Phase 2，macOS host）。真实 VoiceOver 朗读和导航仍需人工验收；Rotor 的自动契约见
ADR-0103；跨平台 Accessibility role mapping 不在本 ADR 范围内。

## 背景

ADR-0101 固定了 Rust 的 Revision-bound semantic node ABI，但只提供节点元数据还不足以让
VoiceOver 在 Markdown 文档中按标题、列表项和链接导航。让 AppKit 从 TextKit 字符串重新解析
Markdown 会产生第二套语义，也会在编辑后留下跨 Revision 的旧 AX 元素。

## 决策

- `DocumentTextView` 作为可编辑 Accessibility root，仍负责完整 source text、selection、IME
  和 source range 查询；Rust 返回的 document root 不重复暴露为另一个 AppKit element。
- 每个非 root semantic node 物化为一个实现 AppKit `NSAccessibilityElementProtocol` 的 native
  Accessibility child。节点的 `parent`/`children` 完全来自 Rust node index；Swift 只保存 role、flags、
  level 和 Revision-bound source/label ranges，不复制 source 文本。
- role mapping 采用 macOS 14 可用的保守角色：标题/段落/代码/强调使用 `staticText` 并提供本地化
  role description；链接使用 `link`；图片使用 `image`；列表/引用使用 `group`；task list 使用
  `checkBox` 并用布尔 `accessibilityValue` 表示完成状态。避免依赖 macOS 26 才可用的 heading role。
- `accessibilityLabel` 和静态文本范围查询通过 `StorageBridge` 的 expected Revision source-range
  ABI 获取；Revision 改变后旧 element 的 label 查询返回 nil，host 重建整棵 owned tree。
- `accessibilityFrame` 由当前 TextKit layout 将 source UTF-16 range 转换为屏幕坐标；composition
  active、layout 尚未建立或 Revision 不一致时返回 zero，而不是猜测几何。
- host 在 source/selection/IME 状态变化后重建 semantic children，并对旧 child 发送
  `uiElementDestroyed`，再发送 root 的 `valueChanged` 与 `layoutChanged`；不会让 child element
  持有 `EditorDocument`、parser 或可变 source。
- `DocumentTextView` 提供 Heading 和 Link 两个 AppKit custom rotor。Rotor 只按当前 child 顺序过滤
  节点并返回 source range，不维护另一份索引；delegate 由 root 强引用，避免 AppKit 的 weak delegate
  在查询时失效。
- 增加 `--accessibility-self-check <file>` 无窗口命令，创建真实 AppKit elements，验证 parent/label/
  role、task checkbox value、Heading/Link rotor、编辑后的 stale node 和新 Revision tree。它只能
  验证 AX 数据契约，不能代替 VoiceOver 实际朗读、转子导航和日文/emoji 朗读验收。

## 验证

- `swift build --package-path experiments/macos-document-host`
- `experiments/macos-document-host/.build/arm64-apple-macosx/debug/YuMacDocumentHost \
  --accessibility-self-check README.md`
- `cargo test --workspace` 与 `cargo clippy --workspace --all-targets -- -D warnings` 保持 Rust
  semantic node ABI 不回归。

## 后续

在真实 macOS 会话中打开 VoiceOver，依次验证标题/链接 Rotor、列表/task 状态、链接/图片、中文/日文/
emoji 以及编辑后焦点和 stale element 行为；记录结果后再决定是否需要 URL action、表格 child role
或跨平台 role abstraction。
