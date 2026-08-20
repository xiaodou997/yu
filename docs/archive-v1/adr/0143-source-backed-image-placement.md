# ADR 0143：source-backed image placement 与命中映射

## 状态

Accepted（Phase 3 Track C）

## 背景

0141/0142 已经建立图片 source metadata、ImageIO 解码、Metal texture ownership 和
`RenderCommand::Image`，但资源命令还没有 document-space 几何。若由 native host 根据 Markdown
字符串自行计算图片位置，就会重新实现 alt/delimiter projection，并破坏 source-only 真源约束。

## 决策

1. `yu-layout::LayoutSnapshot` 从同一 `Projection::images()` 生成 `ImagePlacement`。placement
   保存完整 image source range、alt-label source range、projected visual range、line 和有限
   `LayoutRect`；destination/resource identity 不进入 layout。
2. placement 的几何覆盖图片的 projected alt-label span，当前以该 span 的首个 layout line 为
   anchor，并提供不小于四倍 line-height 的 placeholder 宽度（受 viewport 剩余宽度限制）。多行
   alt label 的精确 intrinsic size 与真实图片尺寸留给后续 intrinsic-measurement 阶段。
3. `LayoutSnapshot::hit_test` 先检查 placement bounds。命中图片时返回完整 image source range、
   visual range 的首/尾边界和 `image: Some(source_range)`；普通文字命中保持原有 source/visual
   mapping。`yu-storage-ffi` 将该 optional range 以 Revision-bound UTF-16 起止字段暴露给
   native host，未命中使用 `YU_STORAGE_IMAGE_DESTINATION_NONE`。
4. `yu-workspace` 解析 inline/reference destination，仅把 `ImageKey::fingerprint()`、bounds
   和 fallback color 转成 `ImagePrimitive`。`yu-scene` 在对应 block glyph 后追加 image primitive，
   因此 ready texture 可以遮盖 projected alt label，而 canonical Markdown source 不变。
5. Image resource 未 ready、解码失败或 host 尚未绑定 `MetalImageAtlas` 时，RenderPlan 仍保留
   image command 并由 backend 绘制 fallback；图片 pipeline 不得阻塞 text edit、parser 或 layout。

## 结果

- 图片拥有与文字相同的 source-backed source↔visual↔layout 映射，native 不需要解析 Markdown。
- Scene/RenderPlan 已能表达图片的 document-space overlay，资源级 placeholder 可在真实 texture
  到达前稳定绘制。
- 当前 placement 仍是第一行、占位尺寸策略；持久产品 surface 的 ImagePublication→MetalImageAtlas
  wiring 和真实 intrinsic image measurement 单独推进，不把本阶段伪装成完整图片窗口功能。

## 验证

```text
cargo test -p yu-layout -p yu-scene -p yu-workspace -p yu-storage-ffi
```

覆盖 layout placement/whole-source hit、scene painter order、FFI UTF-16 image range 和现有
Revision/stale render-plan 回归。
