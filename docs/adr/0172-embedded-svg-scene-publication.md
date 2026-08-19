# ADR 0172：Embedded SVG Scene Publication Boundary

## 状态

Accepted — 2026-08-19

## 背景

`yu-assets` 已经可以把 Math/Mermaid renderer 的结果作为 Revision-bound
`EmbeddedRenderPublication` 保存下来，但这仍然不是可绘制的场景。若让 FFI 在
cache 命中后直接把 Math 状态报告为 ready，native host 可能在 Scene/RenderPlan
尚未消费 SVG 时隐藏 Markdown 源码，造成“资源 ready 但窗口没有可画内容”的状态。

## 决策

1. `yu-scene` 增加 `EmbeddedSvgPrimitive`。它只保存 resource fingerprint、generation、
   kind、source range、布局 bounds、intrinsic dimensions 和 fallback color；不保存 SVG
   markup，也不依赖 `yu-assets` 的 cache 实现。
2. `yu-render` 增加 `EmbeddedSvg` command 和 `EmbeddedSvgUpload`。只有调用
   `RenderPlanBuilder::build_with_embedded` 并提供同 Revision、同 source range、同
   fingerprint/generation/kind、同 intrinsic dimensions 的 SVG publication，primitive
   才能进入 RenderPlan。普通 `build` 对 embedded primitive 会拒绝缺失 publication。
3. Upload 与 command 分离：markup 是一次性 backend upload，command 只携带小型资源
   identity、bounds、dimensions 和 fallback。这保持 command 可复制，并让后端自己决定
   何时编译、缓存和淘汰 SVG。
4. `yu-workspace` 的 viewport assembly 将匹配的 publication 同时交给 Scene 和
   RenderPlan；macOS 诊断 FFI 也沿用这一入口，追加 embedded command kind、generation、
   kind、dimensions，并将 embedded command/upload count 与 markup byte count 追加到 plan
   snapshot。旧字段顺序不改变。
5. 当前 Metal backend 只把 `EmbeddedSvg` 绘制为确定性的 fallback rectangle。它会
   校验几何并且不会把 SVG publication 当成已完成的可视渲染；真正的 SVG raster/vector
   consumer 作为后续阶段，复用这个 upload boundary，不另起 GPU 管线。

## 不做什么

- 不在这一阶段修改 Markdown source projection 或隐藏 fenced source；当前 Scene primitive
  使用透明 fallback，仍由 source glyph/host fallback 保持可见。
- 不在 FFI cache ready 状态和真正可绘制状态之间建立隐式假设。
- 不把 SVG markup 放进 `Scene`、`RenderCommand` 或 canonical document model。

## 验证

```text
cargo test -p yu-scene -p yu-render -p yu-storage-ffi
cargo check --workspace
```

测试覆盖 primitive 的 source/resource identity、publication matching、尺寸一致性、
upload deduplication 和缺失 publication 拒绝；macOS backend 的 fallback path 继续保持
无 SVG 时的安全可见性。
