# ADR 0170：macOS table divider 的真实 Accessibility splitter

## 状态

已接受（2026-08-18）

## 背景

ADR 0169 固定了 CoreText-shaped、Revision-bound 的 table divider descriptor，但只读 descriptor
本身还不能被 VoiceOver 定位或调整。macOS 产品窗口需要把 divider 暴露为原生 Accessibility 子元素，
同时保持 Markdown source 是唯一真源，且不能让 Accessibility 动作绕过 Rust 的表格 resize 生命周期。

## 决策

1. DocumentTextView 将当前 viewport 的 column descriptor 投影为短生命周期的
   YuAccessibilityTableResizeElement。元素使用 splitter role、source-backed label/value、当前
   屏幕 frame provider 和稳定的 Revision/block/column identifier；它不保存文本或 Rust layout。
2. splitter 的 accessibilityPerformIncrement/accessibilityPerformDecrement 通过 coordinator
   复用 document-space CoreText hit/begin/update/finish 路径。步长为 8–16 pt 的小几何步进，结果只
   保留为 Rust session-only TableResizeCommit，不改变 source、selection、history 或 Revision。
3. Rust 的 macOS divider descriptor 查询和 hit-test 在存在有效 column override 时使用
   block_layout_with_table_resize_and_shaper。这样每次 AX 动作重新枚举到的是有效 divider，连续
   increment/decrement 会累积而不会从 canonical 位置重置。
4. descriptor 只在当前 Revision、可见 viewport 且无 active composition 时创建。source edit、
   composition、surface detach、stale geometry 或 override 清理会发布旧元素的 uiElementDestroyed，
   并按需发布 layoutChanged。

## 结果

- VoiceOver 可以导航到可见表格列分隔线，并执行增大/减小动作。
- 所有几何仍由同一 Rust/CoreText layout 提供；Swift 只负责 AX 生命周期和屏幕坐标转换。
- AX resize 是临时预览，不会伪造 GFM 列宽写回，也不会引入第二份 Markdown 文档模型。
- 当前仍只支持 column splitter；row divider 的 variable-row 布局与持久化继续后置。

## 验证

- Rust FFI test 验证 finish 后 descriptor x 使用有效 override，并可在新 divider 上再次 hit-test。
- macOS Accessibility self-check 验证 splitter role、label/frame、increment/decrement、source
  不变，以及 Revision 更新后旧 element action 失败且 children 为空。
- Swift build、workspace tests 和 clippy 继续作为阶段门槛。
