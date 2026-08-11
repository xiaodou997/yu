# ADR 0057：CoreText System UI Viewport Metrics

## 状态

已接受（Phase 1）

## 背景

macOS spike 的 native `NSFont.systemFont` 不能把内部 family 名直接交给
`CTFontCreateWithName`。例如 `.SFNS-Regular` 是系统 UI 别名，不是可由普通 family
查找 API 重建的用户字体；CoreText 会把它回退成 Times New Roman，并打印运行时警告。
如果 viewport 仍使用这个错误的字体对象，line height 和 grapheme advance 都不能作为
TextKit 的布局指标。

同时，CoreText 原生对象不能跨 `yu-editor-ffi` 边界。FFI 必须只返回可复制的 point-based
数值，且系统 UI 字体和用户选择字体必须有明确、可测试的创建路径。

## 决策

- `yu-font-macos::CoreTextShaper` 增加内部 `CoreTextFontSource`，区分 requested family
  与 system UI。system UI 使用 `CTFontCreateUIFontForLanguage(kCTFontUIFontSystem, ...)`，
  不把 `.SFNS-*` 之类的私有名称传给 `CTFontCreateWithName`。
- `CoreTextGlyphRasterizer` 继承同一个 font source；当 system UI shaping 产生私有系统
  face 时，metrics/rasterization 重新使用 system UI 创建 API，保持 glyph identity 和
  metrics 的来源一致。
- CoreText catalog 不把以 `.` 开头的私有 UI alias 暴露为用户可选 family。普通 family 仍
  通过 `CTFontCreateWithName` 解析，并保留原有 fallback 行为。
- 除现有的显式 family FFI 函数外，新增
  `yu_macos_core_text_system_ui_viewport_metrics`。它只接收 size、UTF-8 sample 和输出
  结构体；`YuCoreTextViewportMetrics` 只包含 owned `line_height` 与 `default_advance`。
- macOS `TextInputView` 的 metrics-only viewport 使用 system UI provider 以及
  `M中🙂é` 混合 grapheme sample。TextKit container width、CoreText line height 和
  shaped sample advance 都以 point 直接发布给 Rust，并继续由 Revision-bound viewport
  config 校验。

## 结果

- native viewport 指标来自和 AppKit system font 相同的 CoreText 创建路径，不再静默回退到
  Times，也不会在 spike 启动时产生系统字体警告。
- FFI 仍然没有泄漏 `CTFontRef`、`CTLine` 或 glyph cache；平台边界只传 owned scalar。
- 当前只替换 metrics-only viewport 的字体指标，尚未宣称 Rust metrics-only layout 等价于
  完整 TextKit/CoreText shaped layout。后续接入 shaped viewport 时必须复用同一 font source
  和 face/fallback 规则。
- family provider 与 system UI provider 分开测试；macOS spike 运行时 self-check 继续
  覆盖 content height、caret reveal 和 native scroll 消费。
