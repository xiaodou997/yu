# ADR 0031：CoreText glyph metrics、CPU 栅格化与 atlas 边界

## 状态

已接受（Phase 1）

## 背景

`CoreTextShaper` 已经返回真实 glyph id、advance、position 和 source cluster，但布局之后的
绘制层还缺少统一的 owned glyph 数据。直接把 `CTFontRef`、`CGContext` 或平台纹理放进共享
layout/editor 状态，会把平台生命周期和 GPU 资源管理传播到 canonical source。

## 决策

在 `yu-font` 增加平台无关的准备层契约：

- `FontMetricKey` 和 `GlyphRasterKey` 用 `FontFaceId + size bits` 标识可缓存对象；
- `FontMetricsSnapshot`、`GlyphMetrics`、`GlyphBitmap` 和 `RasterizedGlyph` 只包含自有标量与
  `Arc<[u8]>` alpha 像素；
- `GlyphRasterizer` 负责把 native font backend 转成这些 owned 值；
- `FontMetricsCache` 以 face/size 缓存字体 ascent、descent、leading 和 units-per-em；
- `GlyphAtlas` 使用单通道 CPU page 和简单 shelf packing，返回 page/rect/metrics。它是渲染
  准备缓存，不是 `EditorDocument`、`TextSnapshot`、projection 或 layout 的 canonical state；
- 空 bitmap glyph（例如空格）仍保留 advance/metrics，但不占用 atlas page。

macOS `CoreTextGlyphRasterizer` 与 `CoreTextShaper` 共享 provider 内的 PostScript-name ↔
`FontFaceId` 表：

- 用 `CTFontGetAscent/Descent/Leading/UnitsPerEm` 复制 font metrics；
- 用 `CTFontGetBoundingRectsForGlyphs` 和 `CTFontGetAdvancesForGlyphs` 得到 glyph 几何与 advance；
- 使用 alpha-only `CGBitmapContext` 和 `CTFontDrawGlyphs` 生成 owned、top-down、8-bit coverage
  bitmap；
- CoreGraphics/CoreText 对象在一次调用内销毁，atlas 只保留自有像素与 placement；
- native 错误、未知 face、非法 glyph id、bitmap/context 失败都显式返回错误。

当前 atlas 是 CPU 侧数据结构，尚未绑定 `wgpu`、Metal 或任何 GPU texture。以后 renderer 可以
按 page epoch 上传和淘汰纹理，而不改变本文定义的 source/editor 边界。

## 结果

- macOS 实测可把 shaping 返回的 glyph 转换成非空 alpha bitmap，并保留 bearing/advance；
- 同一 face/size 的 font metrics 和同一 face/glyph/size 的 rasterization 可命中缓存；
- shared crates 不需要链接 CoreText/CoreGraphics，也不会携带平台句柄；
- layout 仍只消费 glyph advance/source range，GPU renderer 可以在后续阶段消费 atlas placement。

## 限制

- 当前 rasterizer 只验证水平 glyph 和单通道 coverage，不定义 LCD/subpixel 或彩色 emoji atlas；
- atlas 使用固定大小 CPU page 和 shelf packing，尚未提供 GPU upload、LRU 纹理淘汰或 page compaction；
- 完整 BiDi、OpenType feature 选择、hinting 策略和可访问性绘制语义仍属于后续阶段。
