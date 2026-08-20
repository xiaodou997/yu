# ADR 0065：Viewport Scene Input

## 状态

已接受（Phase 1 诊断）

## 背景

ADR 0064 已将 shaped viewport 的 block origin、height、source UTF-16 range 和 kind tag
限制为 revision-bound owned scalar。下一步若让 Swift、Metal backend 或 scene builder 各自把
这些 scalar 解释成 block y，便会重新产生第二套 viewport 定位规则；若把 `ViewportSnapshot`
或 `LayoutSnapshot` 直接跨层传递，则会越过 scene 的所有权边界。

## 决策

- 在 `yu-scene` 增加 `ViewportBlockGeometry` 与 `ViewportSceneInput`。它们只拥有 Revision、
  block index/source range、document-space y/height、measured/kind 和 content height，不拥有
  source text、Markdown node、layout cache、atlas pixels 或平台对象。
- `ViewportSceneInput::new` 验证同一 Revision、连续 block range、source range 顺序、单调 y、
  正 height 与 content-height 上界；不合法 metadata 在进入 scene 前拒绝。
- `SceneBuilder::append_layout_at` 接受显式 document-space origin；既有 `append_layout` 保持
  零 origin 兼容行为。`append_layout_at_block` 额外验证 viewport geometry、layout Revision 和
  block source range 完全一致后才解析 atlas 并追加 primitive。
- block-local `LayoutSnapshot` 仍由 `yu-editor`/调用方拥有；scene 只负责把其 glyph placement
  平移到 geometry 提供的 document-space origin。scene 不根据 block kind 或 source 重新布局。

## 结果

- FFI block snapshot 可以无损地转换成 scene 输入，native host 不需要复制 HeightIndex 或
  Markdown block traversal。
- 每个 block 的 scene append 是 revision/source-bound 且原子的；stale geometry、错误 source
  range、缺 atlas entry 或预算失败都不会留下部分 primitive。
- 当前只建立 metadata → scene 的无窗口 vertical slice；完整 block renderer、图片、selection/
  caret overlay、glyph virtualization 和 Metal 提交仍属于后续阶段。
