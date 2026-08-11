# ADR 0058：macOS CoreText Shaped Line Comparison

## 状态

已接受（Phase 1 诊断）

## 背景

ADR 0056 和 ADR 0057 已经让 macOS 的 native container width、CoreText system UI
line height 和 shaped sample advance 进入 Rust 的 metrics-only viewport 配置，但这仍
不能证明共享 `yu-layout` 与 TextKit 在真实换行边界上保持一致。完整 GUI 之前需要一条
不改变 canonical editor state 的诊断路径，把同一份 plain source 的 shaped line ranges
和宽度暴露给 macOS spike，并与 TextKit 的 UTF-16 行范围比较。

这次比较还暴露了一个共享布局问题：触发换行的最后一个 glyph 不能同时扩展前一行的
source range。否则 line ranges 会重叠，鼠标命中、selection 和 native mirror 都可能
把同一个 source cluster 归到两行。

## 决策

- 在 `yu-editor-ffi` 增加 `YuCoreTextShapedLine`，只传递 owned 的
  `source_start_utf16`、`source_end_utf16` 和 `width`；不让 `CTLine`、`CTRun`、font
  handle 或 layout cache 穿过 ABI。
- `yu_macos_core_text_shaped_lines` 使用 `System UI` 的 CoreText shaper 和共享
  `yu-layout::block_layout_with_shaper`，输入为 UTF-8 source、font size 和 native
  point `max_width`。
- FFI 使用 count/fill 两次调用：`capacity == 0` 只返回所需行数，填充调用在容量不够
  时返回 `YU_FFI_BUFFER_TOO_SMALL`，始终通过 `written` 报告所需/写入数量；非法布局
  参数返回 `YU_FFI_LAYOUT_FAILED`。
- 行范围使用 UTF-16 units，便于 Swift/TextKit 直接比较；Rust 在转换前仍以 source-backed
  UTF-8 `TextRange` 为唯一内部坐标。
- `yu-layout` 只在 line-break、hidden run 或可见 glyph 真正属于当前行时更新
  `line_source_end`；触发 wrap 的 glyph 在新行放置后才归入新行。每条 line 的 source
  range 必须有序、非重叠并保持 source-backed。
- macOS spike 在启动 self-check 中枚举 TextKit line fragments，并比较 Rust 返回的
  非空 UTF-16 source ranges、行数和有限宽度；Rust 额外保留的零宽 trailing caret line
  必须有序且宽度为零，但不计入 TextKit 的 line-fragment 数量。该路径只读当前 TextKit
  source，不提交 Transaction、不推进 Revision，也不替代 AppKit 的 canonical mirror。

## 结果

- 在进入完整 GUI 前，可以用真实 macOS system UI shaping 验证共享布局的 line-break
  边界和 UTF-16 映射；Rust 与 TextKit 不一致时会在 spike 启动阶段显式失败，而不是
  静默接受错误坐标。
- 比较协议明确区分 source-consuming visual lines 与 editor-only zero-width caret lines，
  不会为了迎合 TextKit 而删除共享布局需要的 trailing caret 状态。
- source-range wrap 回归测试固定了两行 glyph 的不重叠范围，避免后续 shaping 改动重新
  引入 overlap。
- 该 probe 目前只覆盖 plain source 和当前 spike 的单一 TextKit 容器；它不宣称已经
  解决 Markdown hidden syntax、完整 fallback/BiDi、嵌入块或最终 viewport virtualization
  的等价性。正式 shaped viewport 仍需在 projection、layout cache 和平台渲染边界上单独
  定义 revision-bound 契约。
