# ADR 0027：平台无关的 font fallback 与 shaping contract

- 状态：Accepted
- 日期：2026-08-10

## 背景

当前 `yu-layout` 已经按 grapheme 调用 `ClusterMetrics`，但真正的文字渲染还需要字体选择、
fallback、script/direction 提示、glyph id、cluster source range 和 glyph advance。若现在
直接接入 AppKit/CoreText，会把平台字体对象和布局生命周期混进编辑器核心；若继续只用一个
`f32` advance，又无法验证未来 shaping 的 source mapping。

## 决策

新增 `yu-font` crate，定义平台无关的数据和替换点：

- `FontDatabase` 注册 `FontFaceSpec`，按 family、weight、slant 和 `FontCoverage` 做确定性
  fallback；
- `FontRequest` 保存 family、size、weight、slant，`VisualRunStyle` 只在选择阶段映射为
  Strong/Emphasis 的合成请求；
- `ShapeRequest` 显式携带 source `TextRange`、style、direction、script 和 font request；
- `TextShaper` 返回 `ShapedText`，其中可有多个 `GlyphRun`，每个 run 绑定一个 fallback face，
  每个 `Glyph` 保存 glyph id、source cluster range 和 advance；
- `yu-layout::LayoutSnapshot::from_projection_with_shaper` 直接消费这些 runs：glyph advance
  决定换行宽度，glyph source cluster range 决定 visual cluster 和 hit-test 映射；
- `MockShaper` 采用一 grapheme 一 glyph 的确定性实现，专门用于测试；
- `FontMetrics` 实现现有 `yu-layout::ClusterMetrics`，让 layout 在没有平台字体 API 时也能
  使用同一套 fallback/size 规则。

## 结果

- layout 不依赖 CoreText、DirectWrite、Fontconfig、GPU 或字体文件解析；
- fallback 切换的边界和 source cluster 映射可以在纯 Rust 中测试；
- 真实 backend 可以只实现 `TextShaper`/font database adapter，不改变 Markdown source、
  Projection 或 LayoutSnapshot 的 canonical state；
- shaped glyph 的 advance 可以控制 layout wrapping，同时 source/visual caret 与 hit-test
  contract 保持不变；`ClusterMetrics` 仍可作为兼容的简化入口。

## 限制

`MockShaper` 不实现 ligature、BiDi、OpenType feature、真实 font metrics 或 rasterization；当前
shaping-aware layout 只验证 glyph cluster 的范围、顺序、advance 和有限性，不执行真实的
BiDi 重排或 glyph positioning。下一阶段再分别接入 macOS 原生字体、真实 shaping 和 glyph
atlas；这些 backend 仍必须通过同一 `ShapingProvider`/`ShapedText` 边界。
