# ADR 0056：Viewport Metrics FFI Contract

## 状态

已接受（Phase 1）

## 背景

`CaretScrollRequest` 的几何必须与平台 viewport 使用同一坐标单位。此前 macOS spike 让 Rust
保留默认 `LayoutConfig(80, 1)`，Swift 再用 TextKit 行高乘除返回结果；这只能证明消费时序，
不能证明 Rust 的换行、行高估计和 AppKit content height 使用的是同一套尺寸。

共享 FFI 不能传递 `NSFont`、CoreText 对象或 glyph cache，但可以传递当前平台布局所需的值。
这些值必须 revision-bound，不能让旧窗口在新 source 上重置 Rust viewport。

## 决策

- `yu_composition_session_set_viewport_config` 接受 expected Revision 和五个有限正/非负值：
  `max_width`、`line_height`、`default_advance`、`estimated_block_height`、`overscan`。
- Rust 将这些值构造成 `ViewportConfig`；metrics-only `LayoutSnapshot::from_projection` 使用
  `LayoutConfig::default_advance` 创建 `MonospaceMetrics`，并把 `default_advance` 纳入 layout
  cache key。shaped backend 仍可完全替换 fallback advance。
- 配置成功不推进 source Revision、selection 或 history；expected Revision 不匹配返回 stale，
  非法值返回专用 `YU_FFI_INVALID_VIEWPORT_CONFIG`，且不改变旧配置。
- macOS spike 将 TextKit container width、font line height 和混合字符样本的平均 grapheme
  advance 量化到 0.01 pt 后提交配置。caret request 的输入和输出都直接使用 native point，
  bridge 不再做 scale 换算。
- 无窗口 Rust/FFI 测试必须覆盖配置实际影响 wrapping/scroll target、stale rejection 和非法
  config；macOS attached host self-check 必须覆盖长文档 content height 与自动滚动。

## 结果

- 平台 host、Rust viewport 和 adapter 的坐标协议现在是显式的；后续接入 CoreText/GlyphRun
  只需替换 advance provider，不需修改 command 或 caret reveal ABI。
- metrics-only fallback 仍然不能代表最终比例字体 shaping，尤其是 CJK、emoji、ligature 和
  Markdown hidden syntax 的视觉宽度；产品 GUI 前必须接入 shaped layout 并比较 native/Rust
  line break 结果。

