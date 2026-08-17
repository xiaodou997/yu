# ADR 0165：将 table transient geometry 绑定到 retained frame

## 状态

已接受（2026-08-17）

## 背景

ADR 0163/0164 已把 column resize 定义为 caller-owned、Revision-bound 的 session override，
并通过 FFI 返回过一次调用的 cell rectangles。若 scene 仍从 canonical layout 组装，而 FFI
或 native overlay 另行使用调整后的坐标，border、glyph 和 render plan 会产生两套几何。

## 决策

1. `ViewportRenderConfig::with_table_resize` 携带一个可选的 `TableResizeCommit`，但不拥有
   `EditorDocument` 或 source mirror。没有 override 的配置保持原有 API/渲染路径。
2. `yu-workspace` 在 image intrinsic 尺寸应用之后、scene assembly 之前，对匹配 block 的
   transient `LayoutSnapshot` 应用 column resize。`SceneBuilder` 因而同时消费调整后的 table
   decoration 和 cell glyph，`RenderPlanBuilder` 再从同一 scene 生成 fill/glyph commands。
3. workspace 在 assembly 开始时拒绝 stale Revision 和 row target；不在当前 viewport 的同一
   Revision commit 不会修改其他 block。column resize 不改变 block height，所以不重建
   `HeightIndex`。
4. frame cache 仍按 source Revision 管理；同一 Revision 的不同 session geometry 是新的
   owned frame，可替换上一个 frame，但不会写回 canonical layout cache。

## 结果

- table divider、selection/header fill、glyph origin、render fill command 使用同一份 transient
  geometry，不再需要 native 层二次猜测 x 坐标。
- source、selection、history、viewport height 和 Markdown fidelity 保持不变。
- row height、跨会话持久化和真正 drag transaction 仍以后续 variable-row/serialization ADR
  为准。

## 验证

- `yu-workspace` 测试确认 transient divider 同时出现在 `TablePrimitive` 和 render-plan
  `FillRect`，canonical layout/cache/source 不变。
- 同一测试在 source Revision 改变后确认旧 commit 在 scene assembly 前被拒绝。
