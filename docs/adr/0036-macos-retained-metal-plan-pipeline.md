# ADR 0036：macOS retained Metal plan pipeline

## 状态

已接受（Phase 1）

## 背景

`yu-render` 已经把 shaped layout 转换为 revision-bound `RenderPlan`，0035 也验证了
`CAMetalLayer` 的 clear/present 生命周期。下一步需要证明共享命令确实可以进入 Metal，而不把
GPU 句柄、窗口或 AppKit 状态倒灌到 shared crates。当前开发机没有可用的 Metal Toolchain，
因此不能依赖构建期 metallib 产物；shader 必须仍能在有 device 的 macOS session 中验证。

## 决策

`yu-render-macos` 增加以下边界：

- `MetalPipeline` 在 backend 内用内嵌的最小 Metal source 创建 solid-rectangle 和 alpha-glyph
  两个 `MTLRenderPipelineState`，并拥有 glyph sampler；Objective-C 对象只在 backend/native
  bridge 中存活。
- `MetalAtlas` 接收 `RenderPlan::uploads()`，把 page id 映射到 `R8Unorm` `MTLTexture`；它是
  可丢弃的 GPU cache，设备重建时可以整体清空，shared plan 不保存任何纹理指针。
- `MetalFrameRenderer::render_plan` 在 Rust 中先校验 viewport、glyph bounds、page dimensions、
  UV 和 painter order，再通过固定 `repr(C)` command/texture binding 数组调用 native bridge。
- native bridge 对每帧执行 `nextDrawable → commandBuffer → renderPass(clear) → draw commands
  → presentDrawable → commit`。矩形和 glyph quad 使用同一 position/UV vertex layout，glyph
  fragment 只采样 page 的 red channel 作为 coverage。
- shader source 由 `include_str!` 受控内嵌；构建不要求 `metallib` 工具。后续具备 Metal
  Toolchain 后，可以替换为预编译 metallib，而不改变 Rust command ABI。

## 结果

- 共享层仍只有 `RenderPlan` 和 owned atlas bytes；native device、pipeline、sampler、texture
  全部留在 macOS backend。
- 无窗口单元测试可以验证 command conversion、painter order、atlas UV 和缺页错误；有 Metal
  device 的 ignored test 覆盖 pipeline creation、atlas upload 和实际 drawable 提交。
- 当前实现每个 `MetalFrameRenderer` 只创建一次 pipeline，避免每帧重新编译 shader；完整窗口、
  damage scissor、drawable resize synchronization 和 batch/indirect draw 仍留给后续阶段。

## 限制

真实呈现仍要求 CAMetalLayer 附着到 AppKit view。没有有效 drawable 时 native bridge 返回
`DrawableUnavailable`，默认 workspace 测试不把它视为失败。开发机缺少 Metal Toolchain 时无法
做离线 shader 编译检查，但 `newLibraryWithSource` 会在实际 device session 中执行编译并返回
`PipelineUnavailable`。
