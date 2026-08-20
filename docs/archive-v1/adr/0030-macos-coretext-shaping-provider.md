# ADR 0030：通过 CoreText CTLine/CTRun 接入真实 glyph shaping

## 状态

已接受（Phase 1）

## 背景

`yu-layout` 已经能消费 `yu-font::ShapedText`，但此前只有确定性 `MockShaper`。仅依靠
grapheme advance 无法验证 ligature、组合字符、fallback 字体和真实 glyph cluster 的 source
映射。macOS 第一平台应先使用系统 CoreText 验证这一条 vertical slice，同时继续保持
`CTFontRef` 不进入共享编辑器状态。

## 决策

在 `platform/macos/yu-font-macos` 增加 `CoreTextShaper`：

- 用 `CTFont` 创建请求 family，并按 `VisualRunStyle` 映射 Bold/Italic symbolic traits；
- 用 `CFAttributedString` 设置 `kCTFontAttributeName`，通过 `CTLine` 触发 CoreText 的真实
  shaping 与 cascade fallback；
- 遍历 `CTRun`，复制 glyph id、position、advance 和 UTF-16 string indices；每个 run 从
  `kCTFontAttributeName` 读取实际字体的 PostScript name，并分配 provider 内稳定的
  `FontFaceId`；
- 通过只存在于适配器内的 `Utf16Map` 将 CoreText UTF-16 index 转成请求 `TextRange` 内的
  UTF-8 source cluster，拒绝 surrogate 中间位置、越界或非单调索引；
- 同时实现 `yu-font::TextShaper` 和 layout-facing `ShapingProvider`。后者创建默认 LTR/
  Unknown script 的 `ShapeRequest`，真实 glyph advance 直接由 `yu-layout` 用于换行；
- native 错误映射为平台无关的 `ShapeError::Backend`，不把 CoreText 类型暴露给共享层。

当前 layout 的 source/visual 顺序契约尚未实现 BiDi，因此 `CTRunStatus::RightToLeft` 或
`NonMonotonic` 输出会明确失败；这比把 RTL indices 反转成错误的 source mapping 更安全。

## 结果

- macOS 实测可以对 Latin、中文、emoji、组合字符和 ligature 文本产生真实 glyph runs；
- `yu-layout::LayoutSnapshot::from_projection_with_shaper` 已有 smoke test，证明真实 advance
  影响行宽且 glyph cluster 仍回到原始 UTF-8 source range；
- CoreText 对象、CFAttributedString 和 glyph 临时数组均在一次 shape 调用内结束生命周期，
  不进入 canonical source、projection 或 layout cache；
- `ShapeError::Backend` 为未来 DirectWrite/Fontconfig backend 提供不暴露平台错误类型的
  共享错误通道。

## 限制

此阶段不实现 glyph rasterization、GPU atlas、font cache、完整 BiDi 或 OpenType feature 配置。
`CoreTextShaper` 目前只接受 LTR/non-monotonic-safe output；这些能力在 layout 的方向和绘制
协议稳定后单独推进。
