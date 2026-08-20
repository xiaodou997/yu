# ADR 0167：macOS table resize 的文档坐标 pointer routing

状态：已接受（Phase 3 Track C；macOS 产品窗口接线）

## 背景

Yu 的 retained Metal surface 和 decoration sibling 都是透明 hit-test 层，实际键盘、IME、选择与
Accessibility 仍由 `DocumentTextView` 接收。此前 table resize 只有 block-local FFI 自检；如果让
Swift 根据 block 高度或 Markdown 文本自行猜测表格，拖动坐标会和 CoreText retained frame 漂移。

## 决策

1. storage FFI 增加 macOS/CoreText-shaped 的 document-space hit/begin 入口。Rust 用同一份
   `visible_blocks_with_shaper` 解析 document `y` 到 block，再把 point 转为 table-local 坐标；Swift
   不扫描 pipe、行号或 Markdown source。
2. `MacosSurfaceHostCoordinator` 保存最小的 pointer session（Revision 与 column/row axis），把
   `mouseDragged` 的 x/y 标量转发到既有 `table_resize_update`，并在每个 update/finish/cancel 前清除
   surface submit key，保证 transient override 即使 geometry 不变也会生成新 retained frame。
3. `DocumentTextView` 只负责事件优先级：divider begin 成功时阻止普通 selection，active drag 继续
   吞掉 mouse events，mouse-up 调 finish，Escape 调 cancel；没有 active session 时完整回到 TextKit
   source hit-test。
4. source Revision 变化、surface detach、stale FFI 或提交失败都会清掉 pointer state；finished
   column preview 只保留到下一次 source reset，不创建 Markdown transaction。
5. row resize 继续沿用现有 deferred variable-row 约束：可以命中和预览协议，但 retained render
   host 当前只消费 column override。

## 验证

- Rust macOS FFI test 验证 document-space hit/begin 与 retained frame 使用同一 divider。
- `--macos-table-resize-coordinator-self-check` 在不创建 window 的 AppKit host 中验证 begin/update/
  finish/cancel、Escape 等价 cancel、stale Revision reset 和无 surface 时的 fallback 状态。
- 实际窗口仍由 TextKit 保留输入、IME、复制粘贴与 VoiceOver 所有权；table resize 不会把 source mirror
  变成第二份文档模型。

## 后续

下一步再定义 hover cursor、可访问的 table divider action，以及将 column preview 写回 Markdown 的
明确 editor transaction；本 ADR 不提前引入 source formatter 或表格重排策略。
