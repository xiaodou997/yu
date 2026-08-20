# ADR 0067：Editor Viewport Scene Integration

## 状态

已接受（Phase 1 诊断）

## 背景

ADR 0066 已经定义了 `SceneBuilder::append_viewport` 的批量原子边界，但调用方仍需要把
`EditorDocument` 的 viewport snapshot、block layout cache 和 scene 输入拼接起来。如果这段
逻辑散落在 macOS host 或未来 GUI 中，平台层就会重新解释 block range、kind、height 和
Revision。

## 决策

- 新增 `yu-workspace` 集成 crate，作为 editor model 与 retained scene/render plan 之间的产品
  组合层；`yu-editor` 不依赖 scene，`yu-scene` 不依赖 editor。
- `assemble_viewport_scene` 先调用 `EditorDocument::visible_blocks_with_shaper`，把返回的
  block metadata 转换为 `ViewportBlockGeometry`/`ViewportSceneInput`，再按同一 block index 和
  layout config 取得 shaped `LayoutSnapshot`。
- 所有 layout snapshot 只在集成层短暂拥有/借用；scene 最终只拥有 primitives、damage、viewport
  和 source `Revision`。平台字体 shaper 与 CPU atlas 由调用方拥有，不写入 canonical editor state。
- 集成层返回 `ViewportSceneFrame`，同时保留产生 scene 的 `ViewportSceneInput`，方便 host 或
  后台 worker 做 Revision 丢弃和诊断，而不复制 HeightIndex。

## 结果

- 第一个无窗口的真实产品 vertical slice 已闭合：`EditorDocument → ViewportSceneInput →
  Scene → RenderPlan`。
- stale/missing atlas 仍由 scene batch 边界拒绝，失败不会发布部分 scene；编辑器 source、selection、
  history 和 composition 不会因渲染准备而改变。
- `yu-workspace` 目前只处理 shaped glyph block；图片、selection/caret overlay、嵌入 block 和
  Metal 提交继续留在后续阶段。
