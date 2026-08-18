# ADR 0168：macOS table resize 的 hover 反馈与 source transaction 边界

## 状态

已接受（2026-08-18）

## 背景

ADR 0167 已将 CoreText-shaped 的 table divider pointer 路由接入 macOS 产品窗口。
下一步需要让用户知道 divider 可以拖动，同时确认拖动完成不会悄悄改写 Markdown。

标准 GFM Markdown 没有列宽或行高字段。当前 session 的 `TableResizeCommit` 只有
Revision、表格/列坐标和临时 divider geometry；它不能直接构成 source transaction。

## 决策

1. `DocumentTextView` 创建覆盖整个可见区域的 mouse-moved tracking area。hover 查询通过
   `MacosSurfaceHostCoordinator.tableResizeHover` 调用与 begin 相同的 document-space
   CoreText-shaped FFI；只有命中内部 column divider 时使用 `NSCursor.resizeLeftRight`，否则
   恢复 `NSCursor.arrow`。
2. hover 查询是只读的，不创建 resize session，也不改变 selection、Revision、history 或
   surface submit state。active column drag 期间保持 resize cursor，finish/cancel、离开窗口和
   view detach 都恢复普通 cursor。
3. finish 仍然只提交当前 session 的 transient geometry。self-check 必须验证 finish 前后
   canonical Markdown source 相等；任何 source 写回都必须经过未来明确的
   `EditorCommand`/`Transaction`。
4. 本阶段不暴露一个没有稳定 divider geometry 与 source cell 语义的伪造 Accessibility
   splitter 节点。VoiceOver 继续由 `DocumentTextView` 的 source-backed semantic tree 负责；
   table divider 的可访问 action 要在确定可枚举的 cell/divider layout ABI、焦点语义和键盘
   增减策略后单独实现。

## 结果

- macOS 真实窗口在列 divider 附近有明确的调整光标反馈，普通文本区域仍使用系统箭头。
- hover、begin、drag、finish 使用同一 Revision-bound shaped geometry，减少 TextKit 与 retained
  surface 坐标漂移。
- 完成拖动不会产生 Markdown 改写或 Undo 历史；当前列宽预览仍是 session-only，source 变化、
  detach 或 submit failure 会被清理。
- Accessibility 不会得到一个看似可操作但无法稳定定位/提交的 divider 元素；后续实现必须先
  固定其完整协议，而不是在 Swift 中猜测表格结构。

## 验证

- `--macos-table-resize-coordinator-self-check` 验证 divider hover 命中、表格外 hover 拒绝、
  begin/update/finish/cancel、Revision reset，以及 finish 前后 source 不变。
- Rust workspace tests/clippy、macOS FFI 静态库构建和 Swift package build 继续作为提交门槛。
