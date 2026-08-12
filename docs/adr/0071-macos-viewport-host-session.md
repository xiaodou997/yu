# ADR 0071：macOS Viewport Host Session

## 状态

已接受（Phase 1 诊断）

## 背景

`MetalFrameRenderer::submit_viewport_frame` 已经固定了单帧内部的 Revision gate、atlas staging、
render 和 commit 顺序，但真实 macOS host 还需要处理跨帧状态：编辑后当前 Revision 改变、旧
frame cache 失效、CAMetalLayer resize 后 surface generation 改变，以及一次 frame 是否已经成功
提交。若这些状态散落在 AppKit view、Metal renderer 和 editor bridge 中，就会形成第二套 viewport
generation 或把 stale frame 重新提交。

## 决策

- `MetalViewportHostSession` 作为单文档、单 viewport 的 host-side 状态机；它只拥有
  `ViewportFrameCache`、current Revision、surface generation、frame serial 和最后一次 owned
  scalar submission，不拥有 `EditorDocument`、source、layout/cache、NSView/NSScrollView 或 GPU
  handle。
- `advance_revision` 只接受单调不下降的 Revision；发生变化时清理不同 Revision 的 frame、frame
  serial 和 last submission。
- `sync_surface_generation` 只接受单调不下降的 generation；resize 成功后清理 last submission，
  但允许同一 Revision frame 在新 surface 上重新提交。
- `publish_frame` 通过 `ViewportFrameCache::publish_if_current` 发布完整 frame，并分配 host-local
  monotonic serial。
- `submit` 先检查 surface generation，再从 cache 取 current Revision frame，最后调用
  `MetalFrameRenderer::submit_viewport_frame`；只有整个 backend submission 成功才写入
  `last_submission`。
- AppKit ignored probe 使用该 session，而不是自行维护 stale/frame/generation 状态；probe 仍只
  验证临时 host，不改变产品窗口所有权。

## 结果

- editor、surface resize 和 Metal renderer 之间出现一个可审计的 host 状态边界。
- stale frame、Revision 回退、surface generation 回退、generation mismatch 和失败提交都有
  明确错误，不会静默改变 session 状态。
- 这是单 viewport 生命周期协议，不是完整 workspace/tab/window manager；多 viewport 需要为每个
  session 单独绑定 frame cache 和 surface generation，不能共享可变状态。

