# ADR 0073：macOS Command-Level Damage Culling

## 状态

已接受（Phase 1 诊断）

## 背景

`MetalFrameRenderer` 已经把 scene damage 转成 scissor，并在 retained target 上逐个 dirty
region 清除。但此前每个 region 仍会重放完整的 native command list：局部编辑虽然减少了
clear 面积，却没有减少 CPU 到 Metal encoder 的 command conversion/encoding 工作。

## 决策

- 只在 `platform/macos/yu-render-macos` backend 内实现 command-level culling；共享
  `Scene`、`RenderPlan`、atlas upload 和 `yu-workspace` publication 协议不变。
- full-clear frame 保留完整 painter-order command list。
- 稳定 surface generation 的 partial-damage frame 只保留 bounds 与至少一个 damage region
  严格相交的 command；命令仍按原顺序出现，并且跨多个 region 的 command 只保留一次。
- culling 在 Rust native ABI 转换之后、Objective-C bridge 之前完成；native bridge 继续用
  每个 damage region 的 scissor 重放已裁剪 command list。

## 结果

- 大 viewport 的局部编辑不再为明显不相交的段落/字形编码 Metal commands。
- shared render plan 仍是完整 retained scene 的唯一描述，backend culling 不会污染 source、
  layout、scene 或 revision。
- culling 只减少 command 数量，不改变 dirty region、target clear 或最终 painter order；
  复杂命令 bounds 仍由 `RenderPlan` conversion 计算并验证。
- 当前没有做 GPU-side indirect draw 或 command batch；如果 profiling 证明 ABI 数组复制仍是
  瓶颈，后续可在相同的 backend 边界内继续优化。

