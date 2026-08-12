# ADR 0070：macOS Viewport Frame Submission

## 状态

已接受（Phase 1 诊断）

## 背景

上一阶段已经把 `MetalFrameConsumer` 放在 `yu-workspace` frame 与 native command path 之间，
但 host 仍需要手工编排三个有顺序的动作：验证 Revision、上传 `RenderPlan` 的 atlas pages、
提交 Metal render。手工编排容易让 stale frame 先触发 GPU 上传，或让 renderer 在 atlas 未同步时
进入 native path。

## 决策

- `MetalFrameRenderer::submit_viewport_frame` 成为 macOS host 消费 workspace frame 的推荐入口。
- 提交流程固定为：
  `current Revision gate → MetalAtlas::sync_plan → render_plan → commit accepted Revision`。
- `MetalAtlas::sync_plan` 先 staging 所有新 page；任一上传失败都不替换已有 page。page 的
  `(width, height, fingerprint)` 相同且属于同一 Metal device 时不重复上传；不同 device 的
  atlas 使用在 native 调用前拒绝。
- 提交成功只返回 owned scalar `MetalFrameSubmission { revision, uploaded_pages }`，不暴露
  native handle、scene/source 或 drawable。
- `render_viewport_frame` 保留为已同步 atlas 的低层入口；workspace/host 代码应使用
  `submit_viewport_frame`，而不是自行组合三个步骤。
- AppKit ignored probe 使用真实 `yu-workspace::ViewportRenderFrame`，覆盖 stale frame 拒绝、
  匹配 Revision 提交、resize 后再次提交和 atlas page 复用；probe 仍不是产品窗口所有权。

## 结果

- stale frame 在 atlas upload 和 native command conversion 前被丢弃，GPU cache 不会因旧工作
  产生副作用。
- atlas upload、retained target 和 frame Revision 的边界集中在一个可审计的 host API；后续
  真正的 NSView/NSScrollView host 只需准备 current Revision、surface、uploader 和 atlas。
- 默认无窗口 CI 仍只验证 revision consumer、command conversion 和 shared workspace；真实
  drawable/pipeline/alpha sampling 由 macOS ignored probe 验证。

