# ADR 0069：macOS Revision-aware Metal Frame Consumer

## 状态

已接受（Phase 1 诊断）

## 背景

`yu-workspace` 现在可以发布绑定同一 Revision 的 `ViewportRenderFrame`，但 macOS Metal
backend 仍需要一个明确的最后消费边界。渲染准备可能在 editor 状态之外完成；在这段时间内，
用户输入可能让 host 进入新 Revision。若 Metal backend 直接把完成的 scene/render plan 转成
native command，就可能把旧 frame 提交到当前 surface，或让较旧 frame 回退覆盖已经接受的新
frame。

## 决策

- `MetalFrameConsumer` 记录最后成功接受的 Revision，不持有 source、`EditorDocument`、
  layout/cache、native object 或 GPU handle。
- `MetalFrameRenderer::render_viewport_frame` 先验证 `ViewportRenderFrame::revision()` 等于
  host 传入的 current Revision，并且不小于最后接受的 Revision；验证必须发生在
  `render_plan` 的 native command conversion、target allocation 和 Metal bridge 调用之前。
- 只有 `render_plan` 成功返回后，consumer 才提交该 Revision。command conversion、atlas 检查、
  target/drawable 或 native encoder 失败都不能推进 consumer 状态。
- `render_plan` 仍保留为低层 backend-neutral plan entry point，供硬件 probe 和已经完成 revision
  gate 的内部调用；workspace frame 进入 Metal 时必须使用 `render_viewport_frame`。
- consumer 的 Revision gate 同时提供无窗口 revision-only 测试入口，因此默认 CI 不需要 Metal
  device、drawable 或产品窗口即可覆盖 stale/reorder 行为。

## 结果

- editor/workspace 到 Metal 的最后共享边界具有单一的 stale-frame 规则；macOS host 不需要复制
  cache 检查或维护第二套 generation。
- 旧后台任务即使完成，也只能在进入 native command path 前被丢弃；失败的提交不会污染已接受
  Revision。
- 当前 consumer 只保证单 renderer、单 viewport 的单调消费。多窗口、多 viewport 或 GPU queue
  调度仍需在这个 Revision gate 之上定义明确的 ownership，而不能把 `Revision` 检查移到
  Objective-C bridge。

