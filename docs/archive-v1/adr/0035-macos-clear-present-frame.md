# ADR 0035：macOS clear-only frame lifecycle

## 状态

已接受（Phase 1）

## 背景

ADR 0034 已经建立 `MTLDevice`、`CAMetalLayer` 和 alpha texture upload，但没有验证 command
queue、drawable acquisition 和 present/commit 的顺序。完整 glyph pipeline 还需要 shader、
vertex layout、atlas sampling 和 damage scissor；在这些内容准备好之前，先固定最小 frame
lifecycle 可以把窗口/设备错误与绘制算法错误分开。

## 决策

`yu-render-macos` 增加：

- `MetalCommandQueue`：绑定一个 `MetalDevice`，拥有 `MTLCommandQueue` 生命周期；
- `MetalFrameRenderer::present_clear`：验证 surface 与 queue 使用同一 device，然后执行
  `nextDrawable → commandBuffer → renderPass(clear) → endEncoding → presentDrawable → commit`；
- 明确的 `DrawableUnavailable`、`CommandBufferUnavailable` 和 `RenderEncoderUnavailable` 错误；
- 共享 `Rgba8` 颜色输入，native bridge 只接收已验证的通道值。

当前 frame 只做 clear，不消费 `RenderPlan::commands`。glyph/rect command encoding 必须在
pipeline 和 atlas sampling 契约确定后另行加入。

## 结果

- queue 与 surface 的 device mismatch 会在 native 调用前拒绝；
- `CAMetalLayer` 没有 drawable 时不会提交半成品 command buffer；
- 不需要完整窗口即可编译和测试 command lifecycle API；
- 后续 AppKit shell 只需把现有 layer 附着到 view，再复用 `present_clear` 验证第一帧。

## 测试限制

真实 frame 测试必须在有 Metal device 且 layer 已有有效显示上下文的 macOS session 中显式运行：

```text
cargo test -p yu-render-macos -- --ignored
```

当前无图形会话可能返回 `DeviceUnavailable` 或 `DrawableUnavailable`，默认 workspace 测试不应
因此失败。
