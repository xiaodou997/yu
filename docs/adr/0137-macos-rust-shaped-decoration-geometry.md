# ADR 0137：macOS Rust/CoreText-shaped visual decoration geometry

## 状态

已接受（Phase 3 Track B；完整 visual renderer 迁移的过渡步骤）。

## 背景

`MacosVisualDecorationView` 已经从 source TextKit view 拆成独立 sibling，但如果它继续读取
TextKit visual mirror 的 line fragment，就会让 selection/caret 的最终像素仍受第二套布局影响。
Metal surface、pointer hit-test、scroll reveal 和 decoration 必须共享同一份 Rust/CoreText
shaped layout 坐标。

## 决策

- `yu_storage_session_macos_visual_decorations` 是 revision-与 composition-generation-bound 的
  count/fill C ABI；它返回 owned selection rectangles、caret scalar 和 viewport header，不跨边界
  暴露 `LayoutSnapshot`、CoreText 对象或 native view。
- 选择矩形和 caret 使用 Rust layout 的 document-space 坐标。Swift 只把当前 `scroll_y` 转成
  decoration sibling 的 viewport-local y，不复制 HeightIndex、换行或 Markdown projection。
- count 与 fill 每次都验证 expected Revision、expected composition generation；Revision 或
  generation 变化时清空输出并拒绝调用，容量不足时不得部分写入矩形数组。
- active composition 返回 `YU_STORAGE_NO_OVERLAY`。marked text 仍由 `DocumentTextView` 的
  TextKit 输入/IME fallback 绘制，直到另有 composition-aware decoration 协议；这避免把暂态
  preedit 误当成 canonical selection。
- Swift 只在 caret、header、矩形都通过 revision/有限值检查时关闭 TextKit 自绘；CoreText 不可用、
  block 不在 viewport、surface 尚未布局、stale 或 composition active 都恢复现有 TextKit fallback。

## 结果

普通非 composition 状态下，selection/caret 像素现在由与 Rust glyph RenderPlan 相同的 shaped
layout 产生；TextKit 只保留输入、IME、Accessibility 和失败回退职责。协议可以在后续迁移完整
visual renderer 时继续复用，而无需改变窗口层级或 canonical source 模型。

## 非目标

- 不在本阶段隐藏 source TextKit glyph，也不迁移真实鼠标命中或 Accessibility 几何。
- 不在 ABI 中返回文本、DOM、富文本对象或 Rust 引用。
- 不把 active composition 的 transient preedit 强行塞进普通 selection decoration 查询。

## 验证

```bash
cargo test -p yu-storage-ffi ffi_macos_visual_decorations_are_shaped_count_fill_and_generation_bound
swift build --package-path experiments/macos-document-host
swift run --package-path experiments/macos-document-host YuMacDocumentHost \
  --visual-decoration-self-check \
  experiments/macos-document-host/Fixtures/projection.md
```
