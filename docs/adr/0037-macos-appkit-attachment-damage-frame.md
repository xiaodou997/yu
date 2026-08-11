# ADR 0037：macOS AppKit attachment and damage frame

## 状态

已接受（Phase 1）

## 背景

0036 已经把 retained `RenderPlan` 送入 Metal，但 layer 仍完全脱离 AppKit，且每帧都 full
clear。下一步需要固定真实产品 shell 将 layer 交给 view 的生命周期，同时让 scene 已经提供的
damage rectangles 影响 GPU 工作量；这一步仍不应创建窗口、菜单或编辑器 UI。

## 决策

- `MetalSurface::attach_to_view` 接收外部 `NSView` 的 native pointer，要求调用方在 AppKit main
  thread 运行，并返回带 `MetalSurface` 生命周期的 `MetalViewAttachment`。native bridge 保存
  view、旧 backing layer 和 Yu `CAMetalLayer` 的 retain；drop 时只有 view 仍指向 Yu layer 才
  恢复旧 layer，避免覆盖其他组件后续安装的 layer。
- `MetalFrameRenderer` 记录 `needs_full_clear` 和最后一次 surface generation。创建后第一帧、
  resize 后第一帧都使用 full clear；成功提交后才更新状态，drawable/encoder 错误不会伪造
  generation 已消费。
- Rust 将 `RenderPlan::damage()` 转换为相对于 plan viewport 的裁剪矩形，拒绝非有限或负尺寸，
  并通过固定 `repr(C)` damage ABI 交给 native bridge。
- `MetalRenderTarget` 是 backend-owned 的持久 BGRA color texture；full clear 之后的 frame 在
  该 target 上使用 `MTLLoadActionLoad`。每个 damage region 设定像素 scissor，先以无 blending
  的 clear pipeline 擦除该区域，再按原 painter order 重绘完整 command list，最后把完整 target
  blit 到当前 `CAMetalDrawable`。不把 drawable 内容当作跨帧缓存，避免 layer 轮换导致未定义像素。

## 结果

- AppKit shell 可以在不让 backend 创建窗口的前提下托管 layer，并安全恢复原 view layer。
- resize generation 会自然触发下一帧 full clear；失败的 resize 不改变 generation。
- 稳定 revision 的渲染工作限制在 damage 区域，glyph/rect command 顺序和 atlas ownership 保持
  0036 的契约。
- drawable 每帧只承担最终 present；target 尺寸不匹配时拒绝提交，而不是在错误尺寸上截断内容。
- 无窗口测试可以验证 damage clipping；真实 view attachment、drawable 和 scissor 仍需有图形
  session 的 ignored test 或后续 AppKit shell 验证。

## 限制

当前 damage path 仍对每个 damage region 重放完整 retained command list，尚未做 command-level
damage culling、批处理或 indirect draw；完整 target 到 drawable 的 blit 也尚未按 damage 做带宽
优化。surface generation 或 target 重建会先 full clear，避免跨尺寸保留未定义像素。
