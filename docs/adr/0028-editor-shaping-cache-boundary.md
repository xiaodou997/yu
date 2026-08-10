# ADR 0028：编辑器层的 shaping provider 与缓存边界

- 状态：Accepted
- 日期：2026-08-10

## 背景

`yu-layout` 已经可以直接消费 `ShapedText/GlyphRun`，但 `yu-editor` 的
`LayoutCache` 和 `ViewportLayout` 仍只有 grapheme metrics 路径。如果把 provider
直接放进 `EditorDocument`，字体数据库、平台对象和 canonical source 会发生不必要的
生命周期耦合；如果继续复用同一个 cache key，metrics layout 也可能被错误地返回给
shaped 查询。

## 决策

- `EditorDocument::block_layout_with_shaper` 与
  `EditorDocument::visible_blocks_with_shaper` 接受调用方持有的 provider；document
  不拥有 provider，也不把字体对象或 glyph cache 放入 source state；
- `LayoutCache` 的 key 增加 `LayoutBackend::{Metrics, Shaped}`，两种布局不会互相命中；
- `ViewportLayout` 同样记录当前 backend。切换 backend 时，已测量高度回退为 estimate，
  再按新 backend 重新测量可见窗口；
- provider 配置的更换由上层负责生命周期管理。若同一 `LayoutBackend::Shaped` 下替换
  了字体配置，应调用 `EditorDocument::clear_layout_state` 清理 layout/viewport 状态后
  重新查询。

## 结果

- 现有 metrics API 保持兼容，shaped API 可以在纯 Rust 测试中验证；
- cache 与 viewport 不会把旧 backend 的换行/高度带入新 backend；
- macOS CoreText、未来 Windows DirectWrite 或 Linux fontconfig backend 可以在平台层
  创建并持有，editor 只接收一次查询期间的 shaping provider；
- `EditorDocument` 仍只拥有 source、Markdown、selection、projection 和 revision-bound
  layout metadata。

## 限制

当前 `LayoutBackend::Shaped` 只有一个粗粒度 cache namespace，不包含字体文件、字号或
OpenType feature 的具体 fingerprint。更换 shaped provider 配置后，上层必须清理缓存；
后续接入真实平台字体时再引入稳定的 font/shaping fingerprint。
